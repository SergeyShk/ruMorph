//! Сборка словаря ruMorph из выгрузки pymorphy3-dicts-ru (`scripts/export_pymorphy.py`)

mod lexemes;
mod ud;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use rumorph::{Builder, Dictionary, Entry, GuessIndex};

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

/// Разборы строки выгрузки: парадигма, форма и основа
type Records = Vec<(u16, u16, String)>;

/// Лексема выгрузки: парадигма, основа и пары «форма леммы, тег UD» ее форм
type SourceLexeme = (u16, String, Vec<(u16, u16)>);

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

    let prefix_numbers: HashMap<&str, u16> = prefixes
        .iter()
        .enumerate()
        .map(|(number, prefix)| (prefix.as_str(), number as u16))
        .collect();
    let spell = |paradigm: u16, stem: &str, form: usize| {
        let layout = &paradigms[paradigm as usize];
        let count = layout.len() / 3;
        let (prefix, suffix) = (
            prefixes[layout[2 * count + form] as usize].as_str(),
            suffixes[layout[form] as usize].as_str(),
        );
        [prefix, stem, suffix].concat()
    };
    let text = read("words.tsv")?;
    // Строка выгрузки: словоформа и ее разборы - парадигма, форма и основа
    let parse = |number: usize, line: &str| -> Result<(String, Records), String> {
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
                return Err(invalid());
            }
            let form_index = usize::from(form);
            let (prefix, suffix) = (
                prefixes[layout[2 * count + form_index] as usize].as_str(),
                suffixes[layout[form_index] as usize].as_str(),
            );
            let stem = word
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(suffix))
                .ok_or_else(invalid)?;
            entries.push((paradigm, form, stem.to_owned()));
        }
        Ok((word.to_owned(), entries))
    };

    // Лексема выгрузки - парадигма и основа; у каждой формы - форма леммы и тег UD. Тег UD
    // и признак «местоимение без PronType» - по тегу OpenCorpora и классу леммы
    let mut ud_numbers: HashMap<(u16, String, bool), (u16, bool)> = HashMap::new();
    // Номер пары «часть речи, признаки» тега UD: разборы с одинаковыми ею и леммой сливаются
    let (mut ud_cores, mut core_numbers): (Vec<usize>, HashMap<String, usize>) = Default::default();
    // Индекс угадывания по тегу UD: имена собственные, нарицательные слова открытых классов
    // или никакого (служебные слова, аббревиатуры)
    let mut ud_guesses: Vec<Option<GuessIndex>> = Vec::new();
    let mut lexeme_numbers: HashMap<(u16, String), usize> = HashMap::new();
    let mut lexemes: Vec<SourceLexeme> = Vec::new();
    let mut untyped_pronouns = BTreeSet::new();
    // Лексемы с одной начальной формой и общей словоформой, у которой в них одинаковые лемма UD
    // и часть речи с признаками, объединяются: иначе часть форм слитого разбора недоступна
    let mut parents: Vec<usize> = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let (_, entries) = parse(number, line)?;
        let mut keys = Vec::with_capacity(entries.len());
        for (paradigm, form, stem) in entries {
            let layout = &paradigms[paradigm as usize];
            let count = layout.len() / 3;
            let lexeme = match lexeme_numbers.get(&(paradigm, stem.clone())) {
                Some(&lexeme) => lexeme,
                None => {
                    let tag_of = |form: usize| tags[layout[count + form] as usize].as_str();
                    let lemma = spell(paradigm, &stem, 0);
                    let mut cells = Vec::with_capacity(count);
                    for form in 0..count {
                        let (word, tag) = (spell(paradigm, &stem, form), tag_of(form));
                        let lemma_form = match ud::lemma_form(tag, &lemma, &word) {
                            Some(ud::LemmaForm::Own) => form,
                            Some(ud::LemmaForm::With(target)) => (0..count)
                                .filter(|&candidate| {
                                    let grammemes: Vec<&str> =
                                        tag_of(candidate).split([',', ' ']).collect();
                                    target.iter().all(|grammeme| grammemes.contains(grammeme))
                                })
                                .max_by_key(|&candidate| {
                                    let common = spell(paradigm, &stem, candidate)
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
                        let ud_lemma = spell(paradigm, &stem, lemma_form);
                        let class = ud::lemma_class(&ud_lemma).to_owned();
                        let counting = ud::counting_form(tag, &ud_lemma, &word);
                        let key = (layout[count + form], class, counting);
                        let (ud, untyped) = match ud_numbers.get(&key) {
                            Some(&value) => value,
                            None => {
                                let mut converted = ud::convert(tag, &key.1);
                                if counting {
                                    converted.labels.push("Coun".into());
                                    converted.labels.sort();
                                }
                                let labels: Vec<&str> =
                                    converted.labels.iter().map(String::as_str).collect();
                                let feats = ud::feats_string(&converted.feats);
                                let ud = builder.add_ud_tag(converted.pos, &feats, &labels)?;
                                if usize::from(ud) == ud_cores.len() {
                                    let next = core_numbers.len();
                                    let core = format!("{}\t{feats}", converted.pos);
                                    ud_cores.push(*core_numbers.entry(core).or_insert(next));
                                    ud_guesses.push(match converted.pos {
                                        _ if converted.feats.contains_key("Abbr") => None,
                                        "PROPN" => Some(GuessIndex::Proper),
                                        "NOUN" | "ADJ" | "VERB" | "ADV" => Some(GuessIndex::Common),
                                        _ => None,
                                    });
                                }
                                let untyped = matches!(converted.pos, "PRON" | "DET")
                                    && !converted.feats.contains_key("PronType");
                                ud_numbers.insert(key, (ud, untyped));
                                (ud, untyped)
                            }
                        };
                        if untyped {
                            untyped_pronouns.insert(format!("{ud_lemma} ({tag})"));
                        }
                        cells.push((lemma_form as u16, ud));
                    }
                    lexemes.push((paradigm, stem.clone(), cells));
                    parents.push(lexemes.len() - 1);
                    lexeme_numbers.insert((paradigm, stem.clone()), lexemes.len() - 1);
                    lexemes.len() - 1
                }
            };
            let (lemma_form, ud) = lexemes[lexeme].2[usize::from(form)];
            let lemmas = (
                spell(paradigm, &stem, lemma_form.into()),
                spell(paradigm, &stem, 0),
            );
            keys.push((lexeme, lemmas, ud_cores[usize::from(ud)]));
        }
        for (i, first) in keys.iter().enumerate() {
            for second in &keys[i + 1..] {
                if first.0 != second.0 && first.1 == second.1 && first.2 == second.2 {
                    let (a, b) = (root(&mut parents, first.0), root(&mut parents, second.0));
                    parents[a.max(b)] = a.min(b);
                }
            }
        }
    }

    // Таблица словоизменения каждой лексемы и номера ее форм в таблице (пусто - те же номера)
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for lexeme in 0..lexemes.len() {
        groups
            .entry(root(&mut parents, lexeme))
            .or_default()
            .push(lexeme);
    }
    let mut placed: Vec<(u16, Vec<u16>)> = vec![(0, Vec::new()); lexemes.len()];
    let mut combined_count = 0;
    for members in groups.values() {
        if let [lexeme] = members.as_slice() {
            let (paradigm, _, cells) = &lexemes[*lexeme];
            placed[*lexeme] = (builder.add_table(*paradigm, cells)?, Vec::new());
            continue;
        }
        combined_count += members.len();
        let sources: Vec<lexemes::Lexeme> = members
            .iter()
            .map(|&lexeme| {
                let (paradigm, stem, cells) = &lexemes[lexeme];
                let layout = &paradigms[*paradigm as usize];
                let count = layout.len() / 3;
                lexemes::Lexeme {
                    stem: stem.clone(),
                    forms: (0..count)
                        .map(|form| lexemes::Form {
                            prefix: prefixes[layout[2 * count + form] as usize].clone(),
                            suffix: suffixes[layout[form] as usize].clone(),
                            tag: layout[count + form],
                            lemma_form: cells[form].0,
                            ud: cells[form].1,
                        })
                        .collect(),
                }
            })
            .collect();
        let (combined, numbers) = lexemes::combine(&sources);
        let mut forms = Vec::with_capacity(combined.forms.len());
        for form in &combined.forms {
            forms.push((
                builder.add_suffix(&form.suffix)?,
                form.tag,
                prefix_numbers[form.prefix.as_str()],
            ));
        }
        let paradigm = builder.add_paradigm(&forms)?;
        let cells: Vec<(u16, u16)> = combined
            .forms
            .iter()
            .map(|form| (form.lemma_form, form.ud))
            .collect();
        let table = builder.add_table(paradigm, &cells)?;
        for (&lexeme, mapped) in members.iter().zip(numbers) {
            placed[lexeme] = (table, mapped);
        }
    }

    // Угадывание незнакомых слов: число словоформ с каждым окончанием у каждой формы таблицы;
    // редкие таблицы (меньше трех лексем) - чаще исключения, чем образец
    let mut popularity: HashMap<u16, usize> = HashMap::new();
    for (table, _) in &placed {
        *popularity.entry(*table).or_default() += 1;
    }
    // Окончание с «ё» хранится дважды: как в словаре и через «е», потому что в текстах «ё»
    // часто пишут как «е»
    let mut guesses: HashMap<(GuessIndex, String), HashMap<Entry, u32>> = HashMap::new();
    for (number, line) in text.lines().enumerate() {
        let (word, records) = parse(number, line)?;
        let mut entries = Vec::with_capacity(records.len());
        for (paradigm, form, stem) in records {
            let lexeme = lexeme_numbers[&(paradigm, stem)];
            let (table, mapped) = &placed[lexeme];
            let entry = Entry {
                table: *table,
                form: mapped.get(usize::from(form)).copied().unwrap_or(form),
            };
            entries.push(entry);
            let ud = lexemes[lexeme].2[usize::from(form)].1;
            let Some(index) = ud_guesses[usize::from(ud)] else {
                continue;
            };
            if popularity[table] < 3 {
                continue;
            }
            let spelling = line.split_once('\t').map_or(line, |(spelling, _)| spelling);
            let starts: Vec<usize> = spelling.char_indices().map(|(at, _)| at).collect();
            for length in 1..=GuessIndex::MAX_ENDING.min(starts.len().saturating_sub(1)) {
                let ending = &spelling[starts[starts.len() - length]..];
                let plain = ending.replace('ё', "е");
                if plain != ending {
                    *guesses
                        .entry((index, ending.to_owned()))
                        .or_default()
                        .entry(entry)
                        .or_default() += 1;
                }
                *guesses
                    .entry((index, plain))
                    .or_default()
                    .entry(entry)
                    .or_default() += 1;
            }
        }
        builder.add_word(&word, &entries)?;
    }
    // Окончание, которое встретилось меньше трех раз, - скорее примета одного слова, чем образец
    let lists: HashMap<(GuessIndex, String), Vec<(Entry, u32)>> = guesses
        .into_iter()
        .filter(|(_, counts)| counts.values().sum::<u32>() >= lexemes::MIN_ENDING_COUNT)
        .map(|(key, counts)| {
            let mut forms: Vec<(Entry, u32)> = counts.into_iter().collect();
            forms
                .sort_by_key(|&(entry, count)| (std::cmp::Reverse(count), entry.table, entry.form));
            forms.truncate(lexemes::MAX_GUESSES);
            (key, forms)
        })
        .collect();
    let mut guess_endings = 0;
    for ((index, ending), forms) in &lists {
        let (index, ending) = (*index, ending.as_str());
        // Окончание, у которого список тот же, что у окончания на букву короче, ничего не меняет:
        // угадывание по нему перейдет к более короткому
        let shorter = ending.char_indices().nth(1).map(|(at, _)| &ending[at..]);
        if shorter.is_some_and(|shorter| lists.get(&(index, shorter.to_owned())) == Some(forms)) {
            continue;
        }
        builder.add_guesses(index, ending, forms)?;
        guess_endings += 1;
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
        "{}: {} словоформ, {} тегов UD, {} таблиц словоизменения, {combined_count} лексем \
         объединено, {guess_endings} окончаний для угадывания, {:.1} МБ, {:.1} с",
        output.display(),
        dictionary.word_count(),
        dictionary.ud_tags().len(),
        placed
            .iter()
            .map(|(table, _)| table)
            .collect::<BTreeSet<_>>()
            .len(),
        data.len() as f64 / 1e6,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Корень множества лексемы в системе непересекающихся множеств
fn root(parents: &mut [usize], mut lexeme: usize) -> usize {
    while parents[lexeme] != lexeme {
        parents[lexeme] = parents[parents[lexeme]];
        lexeme = parents[lexeme];
    }
    lexeme
}
