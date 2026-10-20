//! Сборка словаря ruMorph из выгрузки pymorphy3-dicts-ru (`scripts/export_pymorphy.py`)

mod ud;

use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use rumorph::{Builder, Dictionary, Entry};

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
    let (tags, prefixes, suffixes) = (
        strings("tags.json")?,
        strings("prefixes.json")?,
        strings("suffixes.json")?,
    );
    let mut builder = Builder::new(tags.clone(), prefixes.clone(), suffixes.clone());
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

    // Тег UD и признак «местоимение без PronType» по тегу OpenCorpora и классу леммы
    let mut ud_numbers: HashMap<(u16, String), (u16, bool)> = HashMap::new();
    let mut untyped_pronouns = BTreeSet::new();
    for (number, line) in read("words.tsv")?.lines().enumerate() {
        let invalid = || format!("words.tsv:{}: неверная строка {line:?}", number + 1);
        let (word, records) = line.split_once('\t').ok_or_else(invalid)?;
        let mut entries = Vec::new();
        for record in records.split(' ') {
            let (paradigm, form) = record
                .split_once(':')
                .and_then(|(p, f)| Some((p.parse::<u16>().ok()?, f.parse::<u16>().ok()?)))
                .ok_or_else(invalid)?;
            let layout = paradigms.get(paradigm as usize).ok_or_else(invalid)?;
            let count = layout.len() / 3;
            if usize::from(form) >= count {
                return Err(invalid().into());
            }
            let affixes = |form: usize| {
                (
                    prefixes[layout[2 * count + form] as usize].as_str(),
                    suffixes[layout[form] as usize].as_str(),
                )
            };
            let tag_of = |form: usize| tags[layout[count + form] as usize].as_str();
            let (prefix, suffix) = affixes(form.into());
            let stem = word
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(suffix))
                .ok_or_else(invalid)?;
            let (prefix, suffix) = affixes(0);
            let lemma = [prefix, stem, suffix].concat();
            let tag = tag_of(form.into());
            let lemma_form = match ud::lemma_form(tag, &lemma, word) {
                Some(ud::LemmaForm::Own) => usize::from(form),
                Some(ud::LemmaForm::With(target)) => (0..count)
                    .filter(|&candidate| {
                        let grammemes: Vec<&str> = tag_of(candidate).split([',', ' ']).collect();
                        target.iter().all(|grammeme| grammemes.contains(grammeme))
                    })
                    .max_by_key(|&candidate| {
                        let (prefix, suffix) = affixes(candidate);
                        let spelling = [prefix, stem, suffix].concat();
                        let common = spelling
                            .chars()
                            .zip(word.chars())
                            .take_while(|(a, b)| a == b)
                            .count();
                        (common, std::cmp::Reverse(candidate))
                    })
                    .unwrap_or(0),
                None => 0,
            };
            // Перевод зависит от леммы UD: у «лучшей» это «лучший», а не «хороший» формы 0
            let (prefix, suffix) = affixes(lemma_form);
            let ud_lemma = [prefix, stem, suffix].concat();
            let class = ud::lemma_class(&ud_lemma).to_owned();
            let key = (layout[count + usize::from(form)], class);
            let (ud, untyped) = match ud_numbers.get(&key) {
                Some(&value) => value,
                None => {
                    let converted = ud::convert(tag, &key.1);
                    let labels: Vec<&str> = converted.labels.iter().map(String::as_str).collect();
                    let ud = builder.add_ud_tag(
                        converted.pos,
                        &ud::feats_string(&converted.feats),
                        &labels,
                    )?;
                    let untyped = matches!(converted.pos, "PRON" | "DET")
                        && !converted.feats.contains_key("PronType");
                    ud_numbers.insert(key, (ud, untyped));
                    (ud, untyped)
                }
            };
            if untyped {
                untyped_pronouns.insert(format!("{ud_lemma} ({tag})"));
            }
            let lemma_form = lemma_form as u16;
            entries.push(Entry {
                paradigm,
                form,
                lemma_form,
                ud,
            });
        }
        builder.add_word(word, &entries)?;
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

    // В SynTagRus у каждого местоимения и местоименного прилагательного есть PronType
    if !untyped_pronouns.is_empty() {
        let list: Vec<&str> = untyped_pronouns.iter().map(String::as_str).collect();
        return Err(format!("местоимения без PronType: {}", list.join(", ")).into());
    }

    let data = builder.build()?;
    let dictionary = Dictionary::from_bytes(&data)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, &data).map_err(|err| format!("{}: {err}", output.display()))?;
    eprintln!(
        "{}: {} словоформ, {} тегов UD, {:.1} МБ, {:.1} с",
        output.display(),
        dictionary.word_count(),
        dictionary.ud_tags().len(),
        data.len() as f64 / 1e6,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
