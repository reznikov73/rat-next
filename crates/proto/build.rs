//! Кодогенерация protobuf-схемы (prost).
//!
//! protoc в системе не установлен — используем вендорный бинарник
//! (crate `protoc-bin-vendored`) и передаём его путь через переменную
//! окружения PROTOC, которую читает prost-build.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc_path = protoc_bin_vendored::protoc_bin_path()?;
    // SAFETY: build-скрипт однопоточен на момент установки переменной;
    // set_var вызывается один раз, до любой другой работы.
    unsafe {
        std::env::set_var("PROTOC", &protoc_path);
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(&["proto/rat/v0/core.proto"], &["proto"])?;

    println!("cargo:rerun-if-changed=proto");
    Ok(())
}
