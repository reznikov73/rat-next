//! Захват экрана через DXGI Desktop Duplication (Windows).
//!
//! Конверсия пикселей — вне модуля: `with_frame` отдаёт снимок BGRA в замыкание,
//! которое может применить любую конверсию (в пилоте — swscale в NV12).

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter,
    IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
};
use windows::core::Interface;

use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use crate::input::RectI;

/// Захват экрана: D3D11-девайс + дупликация вывода 0 + staging-текстура.
pub struct Capture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dupl: IDXGIOutputDuplication,
    staging: ID3D11Texture2D,
    output_index: u32,
    pub width: usize,
    pub height: usize,
    /// Левый верхний угол захваченного вывода на виртуальном рабочем столе.
    pub origin: (i32, i32),
    /// Виртуальный рабочий стол (все мониторы).
    pub virt: RectI,
}

// SAFETY: захват живёт и используется только в одном потоке;
// Send нужен лишь чтобы передать владение в этот поток.
unsafe impl Send for Capture {}

impl Capture {
    pub fn new() -> Result<Self, String> {
        Self::new_with_output(0)
    }

    /// Захват вывода `index` (0 — обычно основной монитор; см. `--monitor` у агента).
    pub fn new_with_output(index: u32) -> Result<Self, String> {
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
            let (dupl, w, h, origin) = create_duplication(&device, index)?;
            let staging = create_staging(&device, w, h)?;
            let virt = RectI {
                x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                w: GetSystemMetrics(SM_CXVIRTUALSCREEN),
                h: GetSystemMetrics(SM_CYVIRTUALSCREEN),
            };
            Ok(Self {
                device,
                context,
                dupl,
                staging,
                output_index: index,
                width: w as usize,
                height: h as usize,
                origin,
                virt,
            })
        }
    }

    /// Пересоздать дупликацию после ACCESS_LOST (например, смена разрешения).
    pub fn try_recreate(&mut self) -> bool {
        unsafe {
            if let Ok((dupl, w, h, _origin)) = create_duplication(&self.device, self.output_index) {
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

    /// Забрать кадр (ждать до `timeout_ms`) и передать его снимок BGRA
    /// в замыкание `f` (слайс `bgra`, шаг строки `pitch`).
    /// `Ok(false)` — кадр не пришёл (таймаут).
    pub fn with_frame<F>(&mut self, timeout_ms: u32, f: F) -> Result<bool, String>
    where
        F: FnOnce(&[u8], usize) -> Result<(), String>,
    {
        unsafe {
            let mut frame_info: DXGI_OUTDUPL_FRAME_INFO = std::mem::zeroed();
            let mut res: Option<IDXGIResource> = None;
            match self
                .dupl
                .AcquireNextFrame(timeout_ms, &mut frame_info, &mut res)
            {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(false),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    return Err("ACCESS_LOST".to_string());
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
            let pitch = mapped.RowPitch as usize;
            let slice = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * self.height);
            let out = f(slice, pitch);
            self.context.Unmap(&staging_res, 0);
            let released = self.dupl.ReleaseFrame();
            out?;
            released.map_err(|e| e.to_string())?;
            Ok(true)
        }
    }
}

unsafe fn create_duplication(
    device: &ID3D11Device,
    index: u32,
) -> Result<(IDXGIOutputDuplication, u32, u32, (i32, i32)), String> {
    unsafe {
        let dxgi_device: IDXGIDevice = device.cast().map_err(|e| e.to_string())?;
        let adapter: IDXGIAdapter = dxgi_device.GetAdapter().map_err(|e| e.to_string())?;
        let output = adapter
            .EnumOutputs(index)
            .map_err(|e| format!("EnumOutputs({index}): {e}"))?;
        let desc = output.GetDesc().map_err(|e| e.to_string())?;
        let origin = (desc.DesktopCoordinates.left, desc.DesktopCoordinates.top);
        let output1: IDXGIOutput1 = output.cast().map_err(|e| e.to_string())?;
        let dupl = output1.DuplicateOutput(device).map_err(|e| e.to_string())?;
        let dupl_desc = dupl.GetDesc();
        Ok((
            dupl,
            dupl_desc.ModeDesc.Width,
            dupl_desc.ModeDesc.Height,
            origin,
        ))
    }
}

unsafe fn create_staging(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D, String> {
    unsafe {
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
}
