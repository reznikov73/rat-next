//! Минимальные Rust-биндинги к C-шиму (`shim.c`), оборачивающему libavcodec.

use std::ffi::{CString, c_char, c_int, c_void};

#[repr(C)]
pub struct RlEncOpaque {
    _private: [u8; 0],
}

#[repr(C)]
pub struct RlDecOpaque {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn rl_enc_create(
        codec_name: *const c_char,
        w: c_int,
        h: c_int,
        fps: c_int,
        bitrate: i64,
        gop: c_int,
        preset: *const c_char,
        async_depth: c_int,
        low_delay: c_int,
        err: *mut c_char,
        err_cap: c_int,
    ) -> *mut RlEncOpaque;

    fn rl_enc_send(
        e: *mut RlEncOpaque,
        y: *const u8,
        y_stride: c_int,
        uv: *const u8,
        uv_stride: c_int,
        pts: i64,
        err: *mut c_char,
        err_cap: c_int,
    ) -> c_int;

    fn rl_enc_recv(
        e: *mut RlEncOpaque,
        out: *mut u8,
        out_cap: c_int,
        out_size: *mut c_int,
        err: *mut c_char,
        err_cap: c_int,
    ) -> c_int;

    fn rl_enc_free(e: *mut RlEncOpaque);

    fn rl_dec_create(err: *mut c_char, err_cap: c_int) -> *mut RlDecOpaque;

    fn rl_dec_send(
        d: *mut RlDecOpaque,
        data: *const u8,
        size: c_int,
        err: *mut c_char,
        err_cap: c_int,
    ) -> c_int;

    fn rl_dec_recv(d: *mut RlDecOpaque, err: *mut c_char, err_cap: c_int) -> c_int;

    fn rl_dec_plane(
        d: *mut RlDecOpaque,
        idx: c_int,
        stride: *mut c_int,
        w: *mut c_int,
        h: *mut c_int,
    ) -> *const u8;

    fn rl_dec_pix_fmt(d: *mut RlDecOpaque) -> c_int;

    fn rl_dec_free(d: *mut RlDecOpaque);
}

const ERR_CAP: usize = 512;

fn err_to_string(buf: &[c_char; ERR_CAP]) -> String {
    let mut s = String::new();
    for &ch in buf.iter() {
        if ch == 0 {
            break;
        }
        s.push((ch as u8) as char);
    }
    s.trim().to_string()
}

// ============================== Encoder ==============================

pub struct Encoder {
    ptr: *mut RlEncOpaque,
}

impl Encoder {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        codec: &str,
        w: u32,
        h: u32,
        fps: u32,
        bitrate: i64,
        gop: i32,
        preset: &str,
        async_depth: i32,
        low_delay: i32,
    ) -> Result<Self, String> {
        let codec_c = CString::new(codec).map_err(|e| e.to_string())?;
        let preset_c = CString::new(preset).map_err(|e| e.to_string())?;
        let mut err = [0 as c_char; ERR_CAP];
        let ptr = unsafe {
            rl_enc_create(
                codec_c.as_ptr(),
                w as c_int,
                h as c_int,
                fps as c_int,
                bitrate,
                gop,
                preset_c.as_ptr(),
                async_depth,
                low_delay,
                err.as_mut_ptr(),
                ERR_CAP as c_int,
            )
        };
        if ptr.is_null() {
            return Err(err_to_string(&err));
        }
        Ok(Self { ptr })
    }

    /// 0 = принят, 1 = занят (нужно сначала забрать пакет), <0 — ошибка.
    pub fn send(
        &mut self,
        y: &[u8],
        y_stride: usize,
        uv: &[u8],
        uv_stride: usize,
        pts: i64,
    ) -> Result<i32, String> {
        let mut err = [0 as c_char; ERR_CAP];
        let ret = unsafe {
            rl_enc_send(
                self.ptr,
                y.as_ptr(),
                y_stride as c_int,
                uv.as_ptr(),
                uv_stride as c_int,
                pts,
                err.as_mut_ptr(),
                ERR_CAP as c_int,
            )
        };
        if ret < 0 {
            return Err(err_to_string(&err));
        }
        Ok(ret)
    }

    /// Some(size) = есть пакет, None = пока нет.
    pub fn recv(&mut self, out: &mut [u8]) -> Result<Option<usize>, String> {
        let mut err = [0 as c_char; ERR_CAP];
        let mut size: c_int = 0;
        let ret = unsafe {
            rl_enc_recv(
                self.ptr,
                out.as_mut_ptr(),
                out.len() as c_int,
                &mut size,
                err.as_mut_ptr(),
                ERR_CAP as c_int,
            )
        };
        if ret < 0 {
            return Err(err_to_string(&err));
        }
        Ok(if ret == 1 { Some(size as usize) } else { None })
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { rl_enc_free(self.ptr) }
    }
}

// SAFETY: доступ к энкодеру сериализован через Mutex в конвейере
// (send — в потоке захвата, recv — в потоке выдачи; никогда одновременно).
unsafe impl Send for Encoder {}

// ============================== Decoder ==============================

/// Прямые указатели на плоскости текущего декодированного кадра.
/// ВАЖНО: действительны до следующего вызова любого метода Decoder.
pub struct PlaneView {
    pub y: *const u8,
    pub y_stride: usize,
    pub u: *const u8,
    pub u_stride: usize,
    pub v: *const u8,
    pub v_stride: usize,
    pub w: usize,
    pub h: usize,
}

pub struct Decoder {
    ptr: *mut RlDecOpaque,
}

impl Decoder {
    pub fn new() -> Result<Self, String> {
        let mut err = [0 as c_char; ERR_CAP];
        let ptr = unsafe { rl_dec_create(err.as_mut_ptr(), ERR_CAP as c_int) };
        if ptr.is_null() {
            return Err(err_to_string(&err));
        }
        Ok(Self { ptr })
    }

    /// 0 = принят, 1 = нужно сначала забрать кадр, <0 — ошибка.
    pub fn send(&mut self, data: &[u8]) -> Result<i32, String> {
        let mut err = [0 as c_char; ERR_CAP];
        let ret = unsafe {
            rl_dec_send(
                self.ptr,
                data.as_ptr(),
                data.len() as c_int,
                err.as_mut_ptr(),
                ERR_CAP as c_int,
            )
        };
        if ret < 0 {
            return Err(err_to_string(&err));
        }
        Ok(ret)
    }

    /// true = кадр готов (см. [`Decoder::planes`]).
    pub fn recv(&mut self) -> Result<bool, String> {
        let mut err = [0 as c_char; ERR_CAP];
        let ret = unsafe { rl_dec_recv(self.ptr, err.as_mut_ptr(), ERR_CAP as c_int) };
        if ret < 0 {
            return Err(err_to_string(&err));
        }
        Ok(ret == 1)
    }

    pub fn planes(&mut self) -> Option<PlaneView> {
        let mut ys: c_int = 0;
        let mut us: c_int = 0;
        let mut vs: c_int = 0;
        let mut w: c_int = 0;
        let mut h: c_int = 0;
        let y = unsafe { rl_dec_plane(self.ptr, 0, &mut ys, &mut w, &mut h) };
        if y.is_null() {
            return None;
        }
        let u = unsafe {
            rl_dec_plane(
                self.ptr,
                1,
                &mut us,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        let v = unsafe {
            rl_dec_plane(
                self.ptr,
                2,
                &mut vs,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        Some(PlaneView {
            y,
            y_stride: ys as usize,
            u,
            u_stride: us as usize,
            v,
            v_stride: vs as usize,
            w: w as usize,
            h: h as usize,
        })
    }

    pub fn pix_fmt(&mut self) -> i32 {
        unsafe { rl_dec_pix_fmt(self.ptr) }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { rl_dec_free(self.ptr) }
    }
}

// SAFETY: Decoder используется только в одном потоке; Send нужен для передачи
// владения в рабочий поток.
unsafe impl Send for Decoder {}

// ============================== Sws (конверсии) ==============================

unsafe extern "C" {
    fn rl_sws_create(w: c_int, h: c_int, src_fmt: c_int, dst_fmt: c_int) -> *mut c_void;
    fn rl_sws_bgra_to_nv12(
        ctx: *mut c_void,
        bgra: *const u8,
        bgra_stride: c_int,
        y: *mut u8,
        y_stride: c_int,
        uv: *mut u8,
        uv_stride: c_int,
        h: c_int,
    ) -> c_int;
    fn rl_sws_yuv_to_rgba(
        ctx: *mut c_void,
        y: *const u8,
        ys: c_int,
        u: *const u8,
        us: c_int,
        v: *const u8,
        vs: c_int,
        rgba: *mut u8,
        rgba_stride: c_int,
        h: c_int,
    ) -> c_int;
    fn rl_sws_free(ctx: *mut c_void);
    fn rl_pix_fmt(which: c_int) -> c_int;
}

/// which: 0=BGRA, 1=NV12, 2=YUV420P, 3=RGBA (числа не стабильны между версиями ffmpeg).
pub fn pix_fmt(which: i32) -> i32 {
    unsafe { rl_pix_fmt(which) }
}

pub struct Sws {
    ptr: *mut c_void,
}

impl Sws {
    pub fn new(w: u32, h: u32, src_fmt: i32, dst_fmt: i32) -> Result<Self, String> {
        let ptr = unsafe { rl_sws_create(w as c_int, h as c_int, src_fmt, dst_fmt) };
        if ptr.is_null() {
            return Err(format!("sws_getContext failed ({src_fmt} -> {dst_fmt})"));
        }
        Ok(Self { ptr })
    }

    pub fn ptr(&self) -> *mut c_void {
        self.ptr
    }
}

impl Drop for Sws {
    fn drop(&mut self) {
        unsafe { rl_sws_free(self.ptr) }
    }
}

// SAFETY: каждый контекст Sws используется ровно в одном потоке за раз
// (sws_in — в потоке захвата, sws_out — в потоке декода).
unsafe impl Send for Sws {}

/// # Safety
/// `sws` — валидный контекст под пару BGRA→NV12; указатели буферов валидны для
/// заявленных размеров/шагов.
#[allow(clippy::too_many_arguments)]
pub unsafe fn sws_bgra_to_nv12(
    sws: *mut c_void,
    bgra: *const u8,
    bgra_stride: usize,
    y: *mut u8,
    y_stride: usize,
    uv: *mut u8,
    uv_stride: usize,
    h: usize,
) -> i32 {
    unsafe {
        rl_sws_bgra_to_nv12(
            sws,
            bgra,
            bgra_stride as c_int,
            y,
            y_stride as c_int,
            uv,
            uv_stride as c_int,
            h as c_int,
        )
    }
}

/// # Safety
/// Аналогично [`sws_bgra_to_nv12`]: валидные указатели и буферы заявленных размеров.
#[allow(clippy::too_many_arguments)]
pub unsafe fn sws_yuv_to_rgba(
    sws: *mut c_void,
    y: *const u8,
    ys: usize,
    u: *const u8,
    us: usize,
    v: *const u8,
    vs: usize,
    rgba: *mut u8,
    rgba_stride: usize,
    h: usize,
) -> i32 {
    unsafe {
        rl_sws_yuv_to_rgba(
            sws,
            y,
            ys as c_int,
            u,
            us as c_int,
            v,
            vs as c_int,
            rgba,
            rgba_stride as c_int,
            h as c_int,
        )
    }
}
