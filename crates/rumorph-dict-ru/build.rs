//! Файл словаря для `include_bytes!`: из пакета Python рабочего пространства (`make dictionary`)
//! или из крейта (опубликованный крейт и CI перед публикацией). Пустой словарь допустим только
//! в рабочем пространстве: так проверяется код, которому словарь не нужен, а крейт без словаря
//! не упаковывается

use std::path::{Path, PathBuf};

fn main() {
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("ru.dict");
    let package = crate_dir.join("../../python/rumorph");
    let candidates = [package.join("data/ru.dict"), crate_dir.join("ru.dict")];
    watch(&candidates[0], &package);
    watch(&candidates[1], &crate_dir);
    let result = match candidates.iter().find(|path| path.is_file()) {
        Some(found) => std::fs::copy(found, &output).map(drop),
        None if package.is_dir() => {
            println!(
                "cargo::warning=словарь не собран (make dictionary): крейт собран с пустым словарем"
            );
            std::fs::write(&output, b"")
        }
        None => panic!("нет файла словаря ru.dict в {}", crate_dir.display()),
    };
    if let Err(err) = result {
        panic!("{}: {err}", output.display());
    }
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
