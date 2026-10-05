// Сборка: встраивает иконку приложения в `rat.exe`.
//
// Картинка рисуется тем же кодом, что и иконка окна (`src/icon.rs`), и
// складывается в `.ico` — отдельного файла-картинки в репозитории нет.

use std::io;
use std::path::{Path, PathBuf};

include!("src/icon.rs");

/// Размеры в одном `.ico`: Explorer и Alt+Tab берут подходящий сами.
const SIZES: [u32; 5] = [16, 32, 48, 64, 256];

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR не задан"));
    let ico = out.join("rat.ico");
    if let Err(error) = write_ico(&ico) {
        println!("cargo:warning=иконка не собрана: {error}");
        return;
    }

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico.to_str().expect("путь к .ico не в UTF-8"));
    if let Err(error) = res.compile() {
        println!("cargo:warning=иконка не встроена в exe: {error}");
    }
}

fn write_ico(path: &Path) -> io::Result<()> {
    let mut images: Vec<(u32, Vec<u8>)> = Vec::new();
    for size in SIZES {
        images.push((size, dib(size)));
    }

    let mut file = Vec::new();
    file.extend_from_slice(&0_u16.to_le_bytes()); // reserved
    file.extend_from_slice(&1_u16.to_le_bytes()); // тип: иконка
    file.extend_from_slice(&(images.len() as u16).to_le_bytes());

    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        let side = if *size >= 256 { 0_u8 } else { *size as u8 };
        file.extend_from_slice(&[side, side, 0, 0]);
        file.extend_from_slice(&1_u16.to_le_bytes()); // planes
        file.extend_from_slice(&32_u16.to_le_bytes()); // бит на пиксель
        file.extend_from_slice(&(data.len() as u32).to_le_bytes());
        file.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        file.extend_from_slice(data);
    }

    std::fs::write(path, file)
}

/// Одно изображение в формате DIB (BITMAPINFOHEADER + BGRA + AND-маска).
fn dib(size: u32) -> Vec<u8> {
    let rgba = rgba(size);
    let mut data = Vec::with_capacity((size * size * 5) as usize);

    data.extend_from_slice(&40_u32.to_le_bytes()); // biSize
    data.extend_from_slice(&(size as i32).to_le_bytes()); // biWidth
    data.extend_from_slice(&((size * 2) as i32).to_le_bytes()); // biHeight: XOR + AND
    data.extend_from_slice(&1_u16.to_le_bytes()); // biPlanes
    data.extend_from_slice(&32_u16.to_le_bytes()); // biBitCount
    data.extend_from_slice(&0_u32.to_le_bytes()); // biCompression: BI_RGB
    data.extend_from_slice(&0_u32.to_le_bytes()); // biSizeImage
    for _ in 0..4 {
        data.extend_from_slice(&0_u32.to_le_bytes()); // разрешение и палитра
    }

    // Пиксели: BGRA, строки снизу вверх.
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            data.push(rgba[i + 2]);
            data.push(rgba[i + 1]);
            data.push(rgba[i]);
            data.push(rgba[i + 3]);
        }
    }

    // AND-маска: 1 бит на пиксель, строки снизу вверх, выравнивание по 4 байта.
    let row = (size.div_ceil(32) * 4) as usize;
    for y in (0..size).rev() {
        let mut line = vec![0_u8; row];
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            if rgba[i + 3] < 128 {
                line[(x / 8) as usize] |= 0x80 >> (x % 8);
            }
        }
        data.extend_from_slice(&line);
    }

    data
}
