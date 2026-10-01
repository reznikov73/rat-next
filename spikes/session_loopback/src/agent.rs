//! Агентская сторона loopback-сессии.
//!
//! Поток A: пейсинг + захват (DXGI) + конверсия BGRA→NV12 (swscale) + отправка в H.264 энкодер.
//! Поток B: приём пакетов энкодера → mux-кадры (Video) → «сеть»; обработка Control (Ping→Pong).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use prost::Message;
use rat_mux::{flags, Channel, Encoder as MuxEncoder};
use rat_proto::rat::v0::{envelope::Body, Envelope, Hello, Pong};

use crate::ffi::{Encoder, Sws};
use crate::link::Duplex;
use crate::stats::{now_us, SessionStats, TimeBase};

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

pub struct AgentConfig {
    pub fps: u32,
    pub bitrate: i64,
    pub gop: i32,
    pub codec: String,
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

// SAFETY: захват живёт и используется только в одном потоке (поток A);
// Send нужен лишь чтобы передать владение в этот поток.
unsafe impl Send for Capture {}

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

    pub fn try_recreate(&mut self) -> bool {
        unsafe {
            if let Ok((dupl, w, h)) = create_duplication(&self.device) {
                if let Ok(staging) = create_staging(&self.device, w, h) {
                    self.dupl = dupl;
                    self.staging = staging;
                    self.width = w as usize;
                    self.height = h as usize;
                    return true;
                }
            }
            false
        }
    }

    /// Забрать кадр и сконвертировать в NV12 (swscale).
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

// ============================== Хелперы ==============================

fn encode_env(env: &Envelope) -> Vec<u8> {
    let mut b = Vec::with_capacity(64);
    env.encode(&mut b).expect("encode envelope");
    b
}

fn flush(mux: &mut MuxEncoder, link: &Duplex) -> bool {
    if mux.is_empty() {
        return true;
    }
    link.tx.send(mux.take_bytes()).is_ok()
}

/// Есть ли в начале потока IDR-слайс (type 5) — грубая проверка для флага KEYFRAME.
fn is_idr(data: &[u8]) -> bool {
    let scan = data.len().min(2048);
    let mut i = 0;
    while i + 4 < scan {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
            let nal_type = data[i + 4] & 0x1F;
            if nal_type == 5 {
                return true;
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    false
}

// ============================== Запуск агента ==============================

pub fn spawn_agent(
    base: TimeBase,
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<SessionStats>>,
    link: Duplex,
    cfg: AgentConfig,
) -> Result<Vec<std::thread::JoinHandle<()>>, String> {
    let cap = Capture::new()?;
    let (cw, ch) = (cap.width, cap.height);

    let open_enc = |codec: &str, preset: &str, async_depth: i32, low_delay: i32| {
        Encoder::new(
            codec,
            cw as u32,
            ch as u32,
            cfg.fps,
            cfg.bitrate,
            cfg.gop,
            preset,
            async_depth,
            low_delay,
        )
    };
    let (enc, codec_used) = match open_enc(&cfg.codec, &cfg.preset, cfg.async_depth, cfg.low_delay)
    {
        Ok(e) => (e, cfg.codec.clone()),
        Err(e1) => {
            println!("[agent] {e1} — fallback на libx264");
            let e = open_enc("libx264", "veryfast", 1, 0)?;
            (e, "libx264".to_string())
        }
    };
    stats.lock().unwrap().codec = codec_used;

    let sws_in = Sws::new(cw as u32, ch as u32, crate::ffi::pix_fmt(0), crate::ffi::pix_fmt(1))?;
    let enc_shared = Arc::new(Mutex::new(enc));
    // (frame_id, ts_us_capture, t_sent, cap_ms)
    let inflight: Arc<Mutex<VecDeque<(u64, u64, Instant, f32)>>> =
        Arc::new(Mutex::new(VecDeque::new()));

    // --- Поток A: пейсинг + захват + отправка в энкодер ---
    let stop_a = stop.clone();
    let stats_a = stats.clone();
    let enc_a = enc_shared.clone();
    let inflight_a = inflight.clone();
    let fps = cfg.fps.max(1);
    let thread_a = std::thread::spawn(move || {
        set_above_normal();
        let sws_in = sws_in;
        let mut cap = cap;
        let mut y_buf = vec![0u8; cw * ch];
        let mut uv_buf = vec![0u8; cw * ch / 2];
        let frame_interval = Duration::from_micros(1_000_000 / fps as u64);
        let mut next_t = Instant::now();
        let mut frame_id: u64 = 0;
        let mut consec_err: u32 = 0;

        while !stop_a.load(Ordering::Relaxed) {
            let now = Instant::now();
            if next_t > now {
                std::thread::sleep(next_t - now);
            }
            if next_t < Instant::now() {
                next_t = Instant::now() + frame_interval;
            } else {
                next_t += frame_interval;
            }

            let t0 = Instant::now();
            let got = match cap.acquire_nv12(15, sws_in.ptr(), &mut y_buf, &mut uv_buf) {
                Ok(v) => v,
                Err(e) => {
                    if e.contains("ACCESS_LOST") && cap.try_recreate() {
                        continue;
                    }
                    consec_err += 1;
                    {
                        let mut st = stats_a.lock().unwrap();
                        st.agent_errors += 1;
                        st.last_error = e;
                    }
                    if consec_err > 120 {
                        break;
                    }
                    continue;
                }
            };
            if !got {
                continue;
            }
            consec_err = 0;
            let t_cap = Instant::now();
            let cap_ms = (t_cap - t0).as_secs_f32() * 1000.0;
            let ts_us = now_us(&base);

            let t_sent = Instant::now();
            let r = {
                let mut e = enc_a.lock().unwrap();
                e.send(&y_buf, cw, &uv_buf, cw, frame_id as i64)
            };
            match r {
                Ok(0) => {
                    inflight_a
                        .lock()
                        .unwrap()
                        .push_back((frame_id, ts_us, t_sent, cap_ms));
                    frame_id += 1;
                    stats_a.lock().unwrap().captured += 1;
                }
                Ok(_) => {
                    stats_a.lock().unwrap().dropped += 1;
                }
                Err(e) => {
                    let mut st = stats_a.lock().unwrap();
                    st.agent_errors += 1;
                    st.last_error = e;
                }
            }
        }
        stop_a.store(true, Ordering::Relaxed);
    });

    // --- Поток B: пакеты энкодера → mux → «сеть»; Ping→Pong ---
    let stop_b = stop.clone();
    let stats_b = stats.clone();
    let enc_b = enc_shared.clone();
    let inflight_b = inflight.clone();
    let thread_b = std::thread::spawn(move || {
        set_above_normal();
        let mut mux = MuxEncoder::new();
        let mut dec = rat_mux::Decoder::new();
        let mut pkt_buf = vec![0u8; 8 * 1024 * 1024];

        // Hello (capabilities) сразу при старте
        let hello = Envelope {
            ver: 1,
            request_id: 0,
            body: Some(Body::Hello(Hello {
                agent_version: "session-loopback-0.1".to_string(),
                capabilities: vec!["video.h264".to_string(), "mux.v1".to_string()],
            })),
        };
        mux.push(Channel::Control, 0, &encode_env(&hello));
        if !flush(&mut mux, &link) {
            stop_b.store(true, Ordering::Relaxed);
        }

        while !stop_b.load(Ordering::Relaxed) {
            let mut did = false;

            // 1) входящие control-кадры
            while let Ok(bytes) = link.rx.try_recv() {
                dec.feed(&bytes);
                did = true;
            }
            let mut reset_dec = false;
            loop {
                match dec.poll_frame() {
                    Ok(Some(f)) => {
                        did = true;
                        if f.channel == Channel::Control {
                            if let Ok(env) = Envelope::decode(f.payload) {
                                {
                                    let mut st = stats_b.lock().unwrap();
                                    st.control_msgs += 1;
                                }
                                if let Some(Body::Ping(p)) = env.body {
                                    let pong = Envelope {
                                        ver: 1,
                                        request_id: env.request_id,
                                        body: Some(Body::Pong(Pong { nonce: p.nonce })),
                                    };
                                    mux.push(Channel::Control, 0, &encode_env(&pong));
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        reset_dec = true;
                        break;
                    }
                }
            }
            if reset_dec {
                dec.reset();
            }

            // 2) пакеты энкодера
            let recv_res = {
                let mut e = enc_b.lock().unwrap();
                e.recv(&mut pkt_buf)
            };
            match recv_res {
                Ok(Some(sz)) => {
                    did = true;
                    let (id, ts_us, _t_sent, _cap_ms) = inflight_b
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or((u64::MAX, 0, Instant::now(), 0.0));
                    let mut payload = Vec::with_capacity(16 + sz);
                    payload.extend_from_slice(&id.to_be_bytes());
                    payload.extend_from_slice(&ts_us.to_be_bytes());
                    payload.extend_from_slice(&pkt_buf[..sz]);
                    let fl = flags::DROPPABLE
                        | if is_idr(&pkt_buf[..sz]) {
                            flags::KEYFRAME
                        } else {
                            0
                        };
                    mux.push(Channel::Video, fl, &payload);
                    {
                        let mut st = stats_b.lock().unwrap();
                        st.sent_pkts += 1;
                        st.sent_bytes += (payload.len() + rat_mux::HEADER_LEN) as u64;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    let mut st = stats_b.lock().unwrap();
                    st.agent_errors += 1;
                    st.last_error = e;
                }
            }

            // 3) выгрузить накопленные mux-кадры в «сеть»
            if !mux.is_empty() {
                did = true;
                if !flush(&mut mux, &link) {
                    break;
                }
            }

            if !did {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    });

    Ok(vec![thread_a, thread_b])
}

fn set_above_normal() {
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
    }
}
