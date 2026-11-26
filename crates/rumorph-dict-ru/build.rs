//! Файл словаря для `include_bytes!`: из пакета Python рабочего пространства (`make dictionary`)
//! или сжатый zstd из крейта (опубликованный крейт и CI перед публикацией): архив крейта
//! на crates.io ограничен 10 МиБ. Пустой словарь - только в рабочем пространстве и с переменной
//! `RUMORPH_EMPTY_DICTIONARY=1`, которую задает `.cargo/config.toml` репозитория: так проверяется
//! код, которому словарь не нужен, а крейт без словаря не упаковывается и зависимость из git без
//! словаря не собирается

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

fn main() {
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("ru.dict");
    let package = crate_dir.join("../../python/rumorph");
    let plain = package.join("data/ru.dict");
    let compressed = crate_dir.join("ru.dict.zst");
    watch(&plain, &package);
    watch(&compressed, &crate_dir);
    println!("cargo::rerun-if-env-changed=RUMORPH_EMPTY_DICTIONARY");
    let empty = std::env::var_os("RUMORPH_EMPTY_DICTIONARY").is_some_and(|value| value == "1");
    let (source, result) = if plain.is_file() {
        (&plain, std::fs::copy(&plain, &output).map(drop))
    } else if compressed.is_file() {
        (&compressed, decompress(&compressed, &output))
    } else if empty && package.is_dir() {
        println!(
            "cargo::warning=словарь не собран (make dictionary): крейт собран с пустым словарем"
        );
        (&output, std::fs::write(&output, b""))
    } else {
        panic!(
            "нет файла словаря ru.dict: соберите его в репозитории ruMorph (make dictionary) \
             или подключите крейт rumorph с crates.io"
        )
    };
    if let Err(err) = result {
        panic!("{} -> {}: {err}", source.display(), output.display());
    }
}

fn decompress(source: &Path, output: &Path) -> std::io::Result<()> {
    let input = BufReader::new(File::open(source)?);
    let mut decoder =
        ruzstd::decoding::StreamingDecoder::new(input).map_err(std::io::Error::other)?;
    std::io::copy(&mut decoder, &mut File::create(output)?).map(drop)
}

/// Пересборка при изменении файла; за отсутствующим файлом следит ближайший существующий
/// каталог не выше `root`: иначе cargo пересобирал бы крейт при каждой сборке, а выше `root`
/// у крейта, подключенного путем вне ruMorph, - каталоги проекта пользователя
fn watch(path: &Path, root: &Path) {
    let existing = path
        .ancestors()
        .take_while(|ancestor| ancestor.starts_with(root))
        .find(|ancestor| ancestor.exists());
    if let Some(existing) = existing {
        println!("cargo::rerun-if-changed={}", existing.display());
    }
}
