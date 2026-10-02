//! Сборка крейта `rat-session`.
//!
//! Без feature `ffmpeg` — no-op. С ней:
//! 1) линковка с FFmpeg (shared build, import-библиотеки MSVC);
//! 2) компиляция C-шима (cc сам находит MSVC через vswhere);
//! 3) копирование DLL FFmpeg рядом с exe целевого профиля.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_FFMPEG").is_none() {
        return;
    }
    println!("cargo:rerun-if-changed=src/ffmpeg/shim.c");

    let ff = std::env::var("FFMPEG_DIR").unwrap_or_else(|_| {
        r"E:\dev\Tools\ffmpeg-shared\ffmpeg-master-latest-win64-gpl-shared".into()
    });
    let ff = PathBuf::from(ff);
    let ff = if ff.join("include").exists() {
        ff
    } else {
        find_root(&ff).unwrap_or(ff)
    };

    let include = ff.join("include");
    let lib = ff.join("lib");
    let bin = ff.join("bin");
    assert!(
        include.exists() && lib.exists(),
        "FFMPEG не найден (include/lib): {}",
        ff.display()
    );

    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=avcodec");
    println!("cargo:rustc-link-lib=avutil");
    println!("cargo:rustc-link-lib=swscale");
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");

    cc::Build::new()
        .file("src/ffmpeg/shim.c")
        .include(&include)
        .flag_if_supported("/utf-8")
        .warnings(true)
        .compile("rlvideo_shim");

    // Копируем DLL ffmpeg рядом с exe (target/<profile>/).
    if let Ok(out) = std::env::var("OUT_DIR")
        && let Some(profile_dir) = Path::new(&out).ancestors().nth(3)
        && bin.exists()
        && let Ok(rd) = std::fs::read_dir(&bin)
    {
        let mut copied = 0;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "dll").unwrap_or(false)
                && std::fs::copy(&p, profile_dir.join(p.file_name().unwrap())).is_ok()
            {
                copied += 1;
            }
        }
        println!("cargo:warning=скопировано DLL ffmpeg: {copied}");
    }
}

fn find_root(dir: &Path) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() && p.join("include").exists() && p.join("lib").exists() {
            return Some(p);
        }
    }
    None
}
