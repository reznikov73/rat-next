//! Конвейер спайка P1: захват (DXGI) → NV12 → H.264 (QSV) → декод → RGBA.
//! Плюс замеры по стадиям (capture+convert / encode / decode+convert / e2e).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

use crate::ffi::{Decoder, Encoder, PlaneView, Sws};

/// Кадр, уходящий в UI.
pub struct FrameOut {
    pub id: u64,
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub t_capture: Instant,
    pub t_decoded: Instant,
}

#[derive(Default)]
pub struct StageSamples {
    pub cap: Vec<f32>,
    pub enc: Vec<f32>,
    pub dec: Vec<f32>,
    pub e2e: Vec<f32>,
}

impl StageSamples {
    fn push(v: &mut Vec<f32>, x: f32) {
        v.push(x);
        if v.len() > 600 {
            v.remove(0);
        }
    }
    pub fn pct(v: &[f32], p: f32) -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((s.len() - 1) as f32 * p) as usize;
        s[idx]
    }
    pub fn avg(v: &[f32]) -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        v.iter().sum::<f32>() / v.len() as f32
    }
}

#[derive(Default)]
pub struct SharedStats {
    pub frames: u64,
    pub dropped: u64,
    pub errors: u64,
    pub last_error: String,
    pub fps: f32,
    pub codec: String,
    pub samples: StageSamples,
}

impl SharedStats {
    pub fn record(&mut self, cap_ms: f32, enc_ms: f32, dec_ms: f32, e2e_ms: f32) {
        self.frames += 1;
        StageSamples::push(&mut self.samples.cap, cap_ms);
        StageSamples::push(&mut self.samples.enc, enc_ms);
        StageSamples::push(&mut self.samples.dec, dec_ms);
        StageSamples::push(&mut self.samples.e2e, e2e_ms);
    }

    pub fn summary(&self) -> String {
        let s = &self.samples;
        format!(
            "codec={} frames={} dropped={} errors={} fps={:.1}\n\
             stage            avg      p50      p95\n\
             capture+conv   {:.2}ms  {:.2}ms  {:.2}ms\n\
             encode         {:.2}ms  {:.2}ms  {:.2}ms\n\
             decode+conv    {:.2}ms  {:.2}ms  {:.2}ms\n\
             e2e (internal) {:.2}ms  {:.2}ms  {:.2}ms",
            self.codec,
            self.frames,
            self.dropped,
            self.errors,
            self.fps,
            StageSamples::avg(&s.cap),
            StageSamples::pct(&s.cap, 0.5),
            StageSamples::pct(&s.cap, 0.95),
            StageSamples::avg(&s.enc),
            StageSamples::pct(&s.enc, 0.5),
            StageSamples::pct(&s.enc, 0.95),
            StageSamples::avg(&s.dec),
            StageSamples::pct(&s.dec, 0.5),
            StageSamples::pct(&s.dec, 0.95),
            StageSamples::avg(&s.e2e),
            StageSamples::pct(&s.e2e, 0.5),
            StageSamples::pct(&s.e2e, 0.95),
        )
    }
}

pub struct PipelineConfig {
    pub codec: String,
    pub fps: u32,
    pub bitrate: i64,
    pub gop: i32,
    pub preset: String,
    pub async_depth: i32,
    pub low_delay: i32,
}

// ============================== Захват ==============================

pub struct Capture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dupl: IDXGIOutputDuplication,
    staging: ID3D11Texture2D,
    pub width: usize,
    pub height: usize,
}

impl Capture {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
            let device = device.ok_or("D3D11 device not created")?;
            let context = context.ok_or("D3D11 context not created")?;

            let (dupl, w, h) = create_duplication(&device)?;
            let staging = create_staging(&device, w, h)?;
            Ok(Self {
                device,
                context,
                dupl,
                staging,
                width: w as usize,
                height: h as usize,
            })
        }
    }

    /// Пересоздать duplication (после ACCESS_LOST / смены режима).
    pub fn try_recreate(&mut self) -> bool {
        unsafe {
            match create_duplication(&self.device) {
                Ok((dupl, w, h)) => {
                    if let Ok(staging) = create_staging(&self.device, w, h) {
                        self.dupl = dupl;
                        self.staging = staging;
                        self.width = w as usize;
                        self.height = h as usize;
                        return true;
                    }
                }
                Err(_) => {}
            }
            false
        }
    }

    /// Забрать кадр и сконвертировать в NV12 (в переданные буферы).
    /// Ok(false) — не было нового кадра (таймаут).
    pub fn acquire_nv12(
        &mut self,
        timeout_ms: u32,
        sws: *mut std::ffi::c_void,
        y: &mut [u8],
        uv: &mut [u8],
    ) -> Result<bool, String> {
        unsafe {
            let mut frame_info: DXGI_OUTDUPL_FRAME_INFO = std::mem::zeroed();
            let mut res: Option<IDXGIResource> = None;
            match self.dupl.AcquireNextFrame(timeout_ms, &mut frame_info, &mut res) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(false),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    return Err("ACCESS_LOST".to_string())
                }
                Err(e) => return Err(format!("AcquireNextFrame: {e}")),
            }
            let res = res.ok_or("no resource in frame")?;
            let tex: ID3D11Texture2D = res.cast().map_err(|e| e.to_string())?;
            let staging_res: ID3D11Resource = self.staging.cast().map_err(|e| e.to_string())?;
            let src_res: ID3D11Resource = tex.cast().map_err(|e| e.to_string())?;
            self.context.CopyResource(&staging_res, &src_res);

            let mut mapped: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
            self.context
                .Map(&staging_res, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| e.to_string())?;
            crate::ffi::sws_bgra_to_nv12(
                sws,
                mapped.pData as *const u8,
                mapped.RowPitch as usize,
                y.as_mut_ptr(),
                self.width,
                uv.as_mut_ptr(),
                self.width,
                self.height,
            );
            self.context.Unmap(&staging_res, 0);
            self.dupl.ReleaseFrame().map_err(|e| e.to_string())?;
            Ok(true)
        }
    }
}

unsafe fn create_duplication(
    device: &ID3D11Device,
) -> Result<(IDXGIOutputDuplication, u32, u32), String> {
    let dxgi_device: IDXGIDevice = device.cast().map_err(|e| e.to_string())?;
    let adapter: IDXGIAdapter = dxgi_device.GetAdapter().map_err(|e| e.to_string())?;
    let output = adapter.EnumOutputs(0).map_err(|e| e.to_string())?;
    let output1: IDXGIOutput1 = output.cast().map_err(|e| e.to_string())?;
    let dupl = output1.DuplicateOutput(device).map_err(|e| e.to_string())?;
    let desc = dupl.GetDesc();
    Ok((dupl, desc.ModeDesc.Width, desc.ModeDesc.Height))
}

unsafe fn create_staging(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut texture: Option<ID3D11Texture2D> = None;
    device
        .CreateTexture2D(&desc, None, Some(&mut texture))
        .map_err(|e| e.to_string())?;
    texture.ok_or_else(|| "CreateTexture2D failed".to_string())
}

// ============================== Конверсии ==============================

/// BGRA → NV12 вручную (не используется; оставлено как fallback-референс).
#[allow(dead_code)]
pub fn bgra_to_nv12(
    src: *const u8,
    pitch: usize,
    w: usize,
    h: usize,
    y: &mut [u8],
    uv: &mut [u8],
) {
    for j in 0..h {
        let row = unsafe { std::slice::from_raw_parts(src.add(j * pitch), w * 4) };
        let yrow = &mut y[j * w..j * w + w];
        for i in 0..w {
            let b = row[i * 4] as i32;
            let g = row[i * 4 + 1] as i32;
            let r = row[i * 4 + 2] as i32;
            let yy = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
            yrow[i] = yy.clamp(0, 255) as u8;
        }
    }
    for j in (0..h).step_by(2) {
        let uvrow = &mut uv[(j / 2) * w..(j / 2) * w + w];
        for i in (0..w).step_by(2) {
            let si = j * pitch + i * 4;
            let (b, g, r) = unsafe {
                (
                    *src.add(si) as i32,
                    *src.add(si + 1) as i32,
                    *src.add(si + 2) as i32,
                )
            };
            let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
            let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
            uvrow[i] = u.clamp(0, 255) as u8;
            uvrow[i + 1] = v.clamp(0, 255) as u8;
        }
    }
}

/// YUV420P → RGBA вручную (не используется; оставлено как fallback-референс).
#[allow(dead_code)]
pub fn yuv420p_to_rgba(pv: &PlaneView, out: &mut [u8]) {
    let w = pv.w;
    let h = pv.h;
    for j in 0..h {
        let yrow = unsafe { std::slice::from_raw_parts(pv.y.add(j * pv.y_stride), w) };
        let urow =
            unsafe { std::slice::from_raw_parts(pv.u.add((j / 2) * pv.u_stride), w / 2 + 1) };
        let vrow =
            unsafe { std::slice::from_raw_parts(pv.v.add((j / 2) * pv.v_stride), w / 2 + 1) };
        let orow = &mut out[j * w * 4..(j + 1) * w * 4];
        for i in 0..w {
            let c = (yrow[i] as i32 - 16).max(0) * 298;
            let d = urow[i / 2] as i32 - 128;
            let e = vrow[i / 2] as i32 - 128;
            let r = (c + 409 * e + 128) >> 8;
            let g = (c - 100 * d - 208 * e + 128) >> 8;
            let b = (c + 516 * d + 128) >> 8;
            let o = i * 4;
            orow[o] = r.clamp(0, 255) as u8;
            orow[o + 1] = g.clamp(0, 255) as u8;
            orow[o + 2] = b.clamp(0, 255) as u8;
            orow[o + 3] = 255;
        }
    }
}

// ============================== Поток конвейера ==============================

pub fn run_pipeline(
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<SharedStats>>,
    tx: SyncSender<FrameOut>,
    cfg: PipelineConfig,
) -> Result<(), String> {
    let mut cap = Capture::new()?;
    let (cw, ch) = (cap.width, cap.height);

    let mut enc = match Encoder::new(
        &cfg.codec,
        cw as u32,
        ch as u32,
        cfg.fps,
        cfg.bitrate,
        cfg.gop,
        &cfg.preset,
        cfg.async_depth,
        cfg.low_delay,
    ) {
        Ok(e) => {
            stats.lock().unwrap().codec = cfg.codec.clone();
            e
        }
        Err(e1) => {
            println!("[pipeline] {e1} — fallback на libx264");
            let e = Encoder::new(
                "libx264",
                cw as u32,
                ch as u32,
                cfg.fps,
                cfg.bitrate,
                cfg.gop,
                "veryfast",
                1,
                0,
            )?;
            stats.lock().unwrap().codec = "libx264".into();
            e
        }
    };
    let mut dec = Decoder::new()?;

    // swscale-конверсии (SIMD внутри ffmpeg)
    let sws_in = Sws::new(cw as u32, ch as u32, crate::ffi::pix_fmt(0), crate::ffi::pix_fmt(1))?;
    let sws_out = Sws::new(cw as u32, ch as u32, crate::ffi::pix_fmt(2), crate::ffi::pix_fmt(3))?;

    let mut y_buf = vec![0u8; cw * ch];
    let mut uv_buf = vec![0u8; cw * ch / 2];
    let mut pkt_buf = vec![0u8; 8 * 1024 * 1024];
    let mut rgba = vec![0u8; cw * ch * 4];

    let frame_interval = Duration::from_micros(1_000_000 / cfg.fps.max(1) as u64);
    let mut next_t = Instant::now();
    let mut frame_id: u64 = 0;
    // (frame_id, t_capture, t_sent)
    let mut inflight: std::collections::VecDeque<(u64, Instant, Instant)> =
        std::collections::VecDeque::new();
    let mut fps_t = Instant::now();
    let mut fps_count: u32 = 0;

    while !stop.load(Ordering::Relaxed) {
        // пейсинг
        let now = Instant::now();
        if next_t > now {
            std::thread::sleep(next_t - now);
        }
        if next_t < Instant::now() {
            next_t = Instant::now() + frame_interval;
        } else {
            next_t += frame_interval;
        }

        // 1) захват + конверсия в NV12
        let t0 = Instant::now();
        let got = match cap.acquire_nv12(15, sws_in.ptr(), &mut y_buf, &mut uv_buf) {
            Ok(v) => v,
            Err(e) => {
                if e.contains("ACCESS_LOST") {
                    if cap.try_recreate() {
                        continue;
                    }
                }
                let mut st = stats.lock().unwrap();
                st.errors += 1;
                st.last_error = e;
                continue;
            }
        };
        if !got {
            continue;
        }
        let t_cap = Instant::now();

        // 2) забрать готовые пакеты и декодировать их
        loop {
            let pkt = match enc.recv(&mut pkt_buf) {
                Ok(Some(sz)) => sz,
                Ok(None) => break,
                Err(e) => {
                    let mut st = stats.lock().unwrap();
                    st.errors += 1;
                    st.last_error = e;
                    break;
                }
            };
            let (pid, tcap0, t_sent) = inflight
                .pop_front()
                .unwrap_or((frame_id, t0, Instant::now()));
            let t_enc = Instant::now();

            // декод
            let mut sent = false;
            for _ in 0..3 {
                match dec.send(&pkt_buf[..pkt]) {
                    Ok(0) => {
                        sent = true;
                        break;
                    }
                    Ok(1) => {
                        let _ = dec.recv();
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let mut st = stats.lock().unwrap();
                        st.errors += 1;
                        st.last_error = e;
                        break;
                    }
                }
            }
            if !sent {
                continue;
            }
            loop {
                let gotf = match dec.recv() {
                    Ok(v) => v,
                    Err(e) => {
                        let mut st = stats.lock().unwrap();
                        st.errors += 1;
                        st.last_error = e;
                        break;
                    }
                };
                if !gotf {
                    break;
                }
                if dec.pix_fmt() != crate::ffi::pix_fmt(2) {
                    let mut st = stats.lock().unwrap();
                    st.last_error = format!("unexpected pix_fmt {}", dec.pix_fmt());
                    st.errors += 1;
                    break;
                }
                if let Some(pv) = dec.planes() {
                    unsafe {
                        crate::ffi::sws_yuv_to_rgba(
                            sws_out.ptr(),
                            pv.y,
                            pv.y_stride,
                            pv.u,
                            pv.u_stride,
                            pv.v,
                            pv.v_stride,
                            rgba.as_mut_ptr(),
                            cw * 4,
                            ch,
                        );
                    }
                    let t_dec = Instant::now();

                    {
                        let mut st = stats.lock().unwrap();
                        st.record(
                            (t_cap - t0).as_secs_f32() * 1000.0,
                            (t_enc - t_sent).as_secs_f32() * 1000.0,
                            (t_dec - t_enc).as_secs_f32() * 1000.0,
                            (t_dec - tcap0).as_secs_f32() * 1000.0,
                        );
                        fps_count += 1;
                        let dt = fps_t.elapsed().as_secs_f32();
                        if dt >= 1.0 {
                            st.fps = fps_count as f32 / dt;
                            fps_t = Instant::now();
                            fps_count = 0;
                        }
                    }

                    let out_rgba = std::mem::replace(&mut rgba, vec![0u8; cw * ch * 4]);
                    let frame = FrameOut {
                        id: pid,
                        w: cw,
                        h: ch,
                        rgba: out_rgba,
                        t_capture: tcap0,
                        t_decoded: t_dec,
                    };
                    if tx.try_send(frame).is_err() {
                        stats.lock().unwrap().dropped += 1;
                    }
                }
            }
        }

        // 3) отправить текущий кадр в энкодер
        match enc.send(&y_buf, cw, &uv_buf, cw, frame_id as i64) {
            Ok(0) => {
                let t_sent = Instant::now();
                inflight.push_back((frame_id, t0, t_sent));
                frame_id += 1;
            }
            Ok(_) => {
                // EAGAIN — энкодер ещё занят, пропускаем этот кадр
                stats.lock().unwrap().dropped += 1;
            }
            Err(e) => {
                let mut st = stats.lock().unwrap();
                st.errors += 1;
                st.last_error = e;
            }
        }
    }

    Ok(())
}
