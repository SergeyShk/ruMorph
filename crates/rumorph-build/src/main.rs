//! Сборка словаря ruMorph из выгрузки pymorphy3-dicts-ru (`scripts/export_pymorphy.py`)

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use rumorph::{Builder, Dictionary};

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [export, output] = args.as_slice() else {
        eprintln!("Использование: rumorph-build <каталог выгрузки> <файл словаря>");
        return ExitCode::from(2);
    };
    match build(export, output) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("rumorph-build: {err}");
            ExitCode::FAILURE
        }
    }
}

fn build(export: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let read =
        |name: &str| fs::read_to_string(export.join(name)).map_err(|err| format!("{name}: {err}"));
    let strings = |name: &str| -> Result<Vec<String>, Box<dyn Error>> {
        Ok(serde_json::from_str(&read(name)?).map_err(|err| format!("{name}: {err}"))?)
    };
    let mut builder = Builder::new(
        strings("tags.json")?,
        strings("prefixes.json")?,
        strings("suffixes.json")?,
    );
    builder.meta(read("meta.json")?);

    // В выгрузке парадигма - номера суффиксов, тегов и префиксов форм тремя блоками
    let paradigms: Vec<Vec<u16>> = serde_json::from_str(&read("paradigms.json")?)
        .map_err(|err| format!("paradigms.json: {err}"))?;
    for (number, layout) in paradigms.iter().enumerate() {
        let count = layout.len() / 3;
        if layout.len() % 3 != 0 {
            return Err(format!("paradigms.json: длина парадигмы {number} не кратна 3").into());
        }
        let forms: Vec<_> = (0..count)
            .map(|form| (layout[form], layout[count + form], layout[2 * count + form]))
            .collect();
        builder.add_paradigm(&forms)?;
    }

    for (number, line) in read("words.tsv")?.lines().enumerate() {
        let invalid = || format!("words.tsv:{}: неверная строка {line:?}", number + 1);
        let (word, records) = line.split_once('\t').ok_or_else(invalid)?;
        let analyses = records
            .split(' ')
            .map(|record| {
                let (paradigm, form) = record.split_once(':')?;
                Some((paradigm.parse().ok()?, form.parse().ok()?))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(invalid)?;
        builder.add_word(word, &analyses)?;
    }

    for (number, line) in read("probabilities.tsv")?.lines().enumerate() {
        let invalid = || format!("probabilities.tsv:{}: неверная строка {line:?}", number + 1);
        let fields: Vec<&str> = line.split('\t').collect();
        let [word, tag, millionths] = fields.as_slice() else {
            return Err(invalid().into());
        };
        let tag = tag.parse().map_err(|_| invalid())?;
        let millionths = millionths.parse().map_err(|_| invalid())?;
        builder.add_probability(word, tag, millionths)?;
    }

    let data = builder.build()?;
    let dictionary = Dictionary::from_bytes(&data)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, &data).map_err(|err| format!("{}: {err}", output.display()))?;
    eprintln!(
        "{}: {} словоформ, {:.1} МБ, {:.1} с",
        output.display(),
        dictionary.word_count(),
        data.len() as f64 / 1e6,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
