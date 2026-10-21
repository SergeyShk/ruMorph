//! Файл словаря для `include_bytes!`: из крейта (опубликованный крейт и CI перед публикацией)
//! или из пакета Python рабочего пространства (`make dictionary`). Без словаря крейт собирается
//! с пустым словарем: так проверяется код рабочего пространства, которому словарь не нужен

use std::path::{Path, PathBuf};

fn main() {
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("ru.dict");
    let candidates = [
        crate_dir.join("ru.dict"),
        crate_dir.join("../../python/rumorph/data/ru.dict"),
    ];
    for candidate in &candidates {
        println!("cargo::rerun-if-changed={}", candidate.display());
    }
    let result = match candidates.iter().find(|path| path.is_file()) {
        Some(found) => copy(found, &output),
        None => {
            println!(
                "cargo::warning=словарь не собран (make dictionary): крейт собран с пустым словарем"
            );
            std::fs::write(&output, b"")
        }
    };
    if let Err(err) = result {
        panic!("{}: {err}", output.display());
    }
}

fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::copy(from, to).map(drop)
}
