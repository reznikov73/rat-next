//! Спайк P0: захват экрана через DXGI Desktop Duplication (windows-rs, D3D11).
//!
//! Снимает ~5 секунд кадров, считает доставленные кадры и таймауты
//! (статичный рабочий стол не даёт новых кадров), сохраняет до 3 кадров в PNG (out/).
//!
//! Запуск (из корня репозитория):
//!   . .\scripts\devshell.ps1
//!   cd spikes\dxgi_capture
//!   cargo run

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
    DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_DESC, DXGI_OUTDUPL_FRAME_INFO,
};

const CAPTURE_SECONDS: u64 = 5;
const SKIP_FRAMES: u64 = 3; // первые кадры после инициализации могут быть пустыми
const MAX_SAVED_FRAMES: u32 = 3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::path::Path::new("out");
    std::fs::create_dir_all(out_dir)?;

    unsafe {
        // D3D11 device (hardware).
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
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
        )?;
        let device = device.ok_or("D3D11 device not created")?;
        let context = context.ok_or("D3D11 context not created")?;
        println!("[dxgi] D3D11 device created (hardware)");

        let (dupl, width, height) = create_duplication(&device)?;
        println!("[dxgi] duplication OK: {width}x{height}");

        let staging = create_staging(&device, width, height)?;

        let mut frames: u64 = 0;
        let mut timeouts: u64 = 0;
        let mut saved: u32 = 0;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(CAPTURE_SECONDS) {
            let mut frame_info: DXGI_OUTDUPL_FRAME_INFO = std::mem::zeroed();
            let mut resource: Option<IDXGIResource> = None;
            match dupl.AcquireNextFrame(500, &mut frame_info, &mut resource) {
                Ok(()) => {
                    frames += 1;
                    if let Some(res) = resource {
                        if frames > SKIP_FRAMES && saved < MAX_SAVED_FRAMES {
                            let texture: ID3D11Texture2D = res.cast()?;
                            copy_and_save(&context, &staging, &texture, width, height, saved)?;
                            saved += 1;
                        }
                    }
                    dupl.ReleaseFrame()?;
                }
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    // Нет новых кадров за 500 мс — нормально для статичного экрана.
                    timeouts += 1;
                }
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    println!(
                        "[dxgi] DXGI_ERROR_ACCESS_LOST после {frames} кадров (смена режима/сессии)"
                    );
                    break;
                }
                Err(e) => return Err(format!("AcquireNextFrame: {e}").into()),
            }
        }
        let secs = start.elapsed().as_secs_f64();
        println!(
            "[dxgi] frames={frames}, timeouts={timeouts} за {secs:.1}s, сохранено {saved} PNG -> out/",
        );
        println!("DXGI_OK");
    }
    Ok(())
}

/// Создать duplication для первого output главного адаптера.
unsafe fn create_duplication(
    device: &ID3D11Device,
) -> Result<(IDXGIOutputDuplication, u32, u32), Box<dyn std::error::Error>> {
    let dxgi_device: IDXGIDevice = device.cast()?;
    let adapter: IDXGIAdapter = dxgi_device.GetAdapter()?;
    let output = adapter.EnumOutputs(0)?;
    let output1: IDXGIOutput1 = output.cast()?;
    let dupl = output1.DuplicateOutput(device)?;
    let desc: DXGI_OUTDUPL_DESC = dupl.GetDesc();
    Ok((dupl, desc.ModeDesc.Width, desc.ModeDesc.Height))
}

/// Staging-текстура BGRA8 для чтения кадров на CPU.
unsafe fn create_staging(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D, Box<dyn std::error::Error>> {
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
    device.CreateTexture2D(&desc, None, Some(&mut texture))?;
    texture.ok_or_else(|| "CreateTexture2D failed".into())
}

/// Скопировать кадр в staging, прочитать и сохранить как PNG (BGRA -> RGBA).
unsafe fn copy_and_save(
    context: &ID3D11DeviceContext,
    staging: &ID3D11Texture2D,
    source: &ID3D11Texture2D,
    width: u32,
    height: u32,
    index: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let staging_res: ID3D11Resource = staging.cast()?;
    let source_res: ID3D11Resource = source.cast()?;
    context.CopyResource(&staging_res, &source_res);

    let mut mapped: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
    context.Map(&staging_res, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

    let row_pitch = mapped.RowPitch as usize;
    let w = width as usize;
    let h = height as usize;
    let mut rgba = vec![0u8; w * h * 4];
    let base = mapped.pData as *const u8;
    for y in 0..h {
        let src_row = base.add(y * row_pitch);
        let dst_row = rgba.as_mut_ptr().add(y * w * 4);
        for x in 0..w {
            // BGRA -> RGBA
            *dst_row.add(x * 4) = *src_row.add(x * 4 + 2);
            *dst_row.add(x * 4 + 1) = *src_row.add(x * 4 + 1);
            *dst_row.add(x * 4 + 2) = *src_row.add(x * 4);
            *dst_row.add(x * 4 + 3) = *src_row.add(x * 4 + 3);
        }
    }
    context.Unmap(&staging_res, 0);

    let img =
        image::RgbaImage::from_raw(width, height, rgba).ok_or("RgbaImage::from_raw failed")?;
    let path = format!("out/frame_{index}.png");
    img.save(&path)?;
    println!("[dxgi] saved {path}");
    Ok(())
}
