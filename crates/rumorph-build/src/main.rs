//! Сборка словаря ruMorph из выгрузки pymorphy3-dicts-ru (`scripts/export_pymorphy.py`),
//! счетчиков UD (`scripts/export_ud.py`) и весов контекстной модели (`scripts/train_context.py`)

mod frequencies;
mod lexemes;
mod ud;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use rumorph::{Builder, Dictionary, Entry, GuessIndex};

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    // Без весов собирается словарь без контекстной модели: на нем она и учится
    let (export, ud, output, context) = match args.as_slice() {
        [export, ud, output] => (export, ud, output, None),
        [export, ud, output, context] => (export, ud, output, Some(context.as_path())),
        _ => {
            eprintln!(
                "Использование: rumorph-build <выгрузка pymorphy3> <выгрузка UD> <файл словаря> \
                 [веса контекстной модели]"
            );
            return ExitCode::from(2);
        }
    };
    match build(export, ud, output, context) {
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

/// Ключ перевода в UD: тег OpenCorpora, класс леммы, счетная форма, вид аббревиатуры и род
/// существительного, у которого есть только множественное число
type UdKey = (
    u16,
    String,
    bool,
    Option<Abbreviation>,
    Option<&'static str>,
);

/// Аббревиатура, у которой признаки UD отличаются от тега OpenCorpora
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Abbreviation {
    /// Сокращение слова внутри его лексемы («им» в лексеме «имя»): без падежа, числа и рода
    WordForm,
    /// Склоняемая аббревиатура («вуз», «МИД»): Abbr - помета, как в SynTagRus
    Declinable,
}

/// Варианты служебных слов, у которых в SynTagRus лемма основного слова: часть речи OpenCorpora,
/// вариант и основное слово. Лексема варианта объединяется с лексемой основного слова
const VARIANT_LEMMAS: &[(&str, &str, &str)] = &[
    ("CONJ", "ж", "же"),
    ("CONJ", "чтоб", "чтобы"),
    ("PRCL", "б", "бы"),
    ("PRCL", "ж", "же"),
    ("PRCL", "чтоб", "чтобы"),
];

fn build(
    export: &Path,
    ud: &Path,
    output: &Path,
    context: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let read =
        |name: &str| fs::read_to_string(export.join(name)).map_err(|err| format!("{name}: {err}"));
    let read_ud =
        |name: &str| fs::read_to_string(ud.join(name)).map_err(|err| format!("{name}: {err}"));
    let strings = |name: &str| -> Result<Vec<String>, Box<dyn Error>> {
        Ok(serde_json::from_str(&read(name)?).map_err(|err| format!("{name}: {err}"))?)
    };
    let (tags, prefixes, suffixes) = (
        strings("tags.json")?,
        strings("prefixes.json")?,
        strings("suffixes.json")?,
    );
    let unknown: BTreeSet<&str> = tags
        .iter()
        .flat_map(|tag| ud::unknown_grammemes(tag))
        .collect();
    if !unknown.is_empty() {
        let list: Vec<&str> = unknown.into_iter().collect();
        eprintln!(
            "rumorph-build: предупреждение: неизвестные граммемы OpenCorpora {} - часть речи \
             переводится в X, граммема - в помету",
            list.join(", ")
        );
    }
    let mut builder = Builder::new(tags.clone(), prefixes.clone(), suffixes.clone());
    // Словарь производен и от трибанков UD: лицензия - их CC BY-SA 4.0, допустимая для OpenCorpora
    let json = |name: &str, text: String| -> Result<serde_json::Value, String> {
        serde_json::from_str(&text).map_err(|err| format!("{name}: {err}"))
    };
    let mut meta = json("meta.json", read("meta.json")?)?;
    if let Some(fields) = meta.as_object_mut() {
        fields.insert("license".into(), "CC BY-SA 4.0".into());
        fields.insert(
            "frequencies".into(),
            json("sources.json", read_ud("sources.json")?)?,
        );
    }
    builder.meta(meta.to_string());
    // Парадигмы по формам: новые парадигмы (объединенные лексемы, копии форм) не повторяются
    let mut paradigm_numbers: HashMap<Vec<(u16, u16, u16)>, u16> = HashMap::new();

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
        let number = builder.add_paradigm(&forms)?;
        paradigm_numbers.entry(forms).or_insert(number);
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
        let grammemes = |&(paradigm, form, _): &(u16, u16, String)| -> Vec<&str> {
            let layout = &paradigms[paradigm as usize];
            tags[layout[layout.len() / 3 + usize::from(form)] as usize]
                .split([',', ' '])
                .collect()
        };
        // Вводный союз при другом разборе словоформы в SynTagRus не бывает CCONJ («кстати», «скажем»)
        let parenthetical = |entry: &(u16, u16, String)| {
            let grammemes = grammemes(entry);
            grammemes[0] == "CONJ"
                && grammemes.contains(&"Prnt")
                && !ud::has_pos_exception("CONJ", &spell(entry.0, &entry.2, 0))
        };
        if entries.iter().any(|entry| !parenthetical(entry)) {
            entries.retain(|entry| !parenthetical(entry));
        }
        // Предикатив при кратком прилагательном среднего рода в SynTagRus - прилагательное («нужно»)
        if entries.iter().any(|entry| {
            let grammemes = grammemes(entry);
            grammemes[0] == "ADJS" && grammemes.contains(&"neut") && grammemes.contains(&"sing")
        }) {
            entries.retain(|entry| grammemes(entry)[0] != "PRED");
        }
        // При равных вероятностях несовершенный вид раньше совершенного: «использовать» в SynTagRus чаще НСВ
        // Разборы глагола стоят группой на месте его первого разбора
        let lemma = |&(paradigm, _, ref stem): &(u16, u16, String)| spell(paradigm, stem, 0);
        let lemmas: Vec<String> = entries.iter().map(lemma).collect();
        let imperfective: Vec<&String> = lemmas
            .iter()
            .zip(&entries)
            .filter(|(_, entry)| grammemes(entry).contains(&"impf"))
            .map(|(lemma, _)| lemma)
            .collect();
        let keys: Vec<(usize, bool)> = entries
            .iter()
            .zip(&lemmas)
            .map(|(entry, lemma)| {
                let first = lemmas.iter().position(|known| known == lemma).unwrap_or(0);
                let perfective =
                    grammemes(entry).contains(&"perf") && imperfective.contains(&lemma);
                (first, perfective)
            })
            .collect();
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by_key(|&i| keys[i]);
        let entries: Vec<_> = order.into_iter().map(|i| entries[i].clone()).collect();
        Ok((word.to_owned(), entries))
    };

    // Лексема выгрузки - парадигма и основа; у каждой формы - форма леммы и тег UD. Тег UD
    // и признак «местоимение без PronType» - по тегу OpenCorpora и классу леммы
    let mut ud_numbers: HashMap<UdKey, (u16, bool)> = HashMap::new();
    // Номер пары «часть речи, признаки» тега UD: разборы с одинаковыми ею и леммой сливаются
    let (mut ud_cores, mut core_numbers): (Vec<usize>, HashMap<String, usize>) = Default::default();
    // Индекс угадывания по тегу UD: имена собственные, нарицательные слова открытых классов
    // или никакого (служебные слова, аббревиатуры)
    let mut ud_guesses: Vec<Option<GuessIndex>> = Vec::new();
    let mut lexeme_numbers: HashMap<(u16, String), usize> = HashMap::new();
    let mut lexemes: Vec<SourceLexeme> = Vec::new();
    let mut untyped_pronouns = BTreeSet::new();
    // Лексемы вариантов и основных слов из VARIANT_LEMMAS по части речи и форме 0
    let mut variant_lexemes: HashMap<(String, String), usize> = HashMap::new();
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
                    let grammemes_of = |form: usize| {
                        let mut grammemes: Vec<&str> = tag_of(form).split([',', ' ']).collect();
                        grammemes.sort();
                        grammemes
                    };
                    // У вариантной формы 0 («счастие», помета V-ie) лемма - форма с тем же тегом
                    // без пометы («счастье»), как в SynTagRus
                    let zero = grammemes_of(0);
                    let default_lemma = zero
                        .iter()
                        .find(|grammeme| grammeme.starts_with("V-"))
                        .and_then(|variant| {
                            (1..count).find(|&candidate| {
                                let mut plain = zero.clone();
                                plain.retain(|grammeme| grammeme != variant);
                                grammemes_of(candidate) == plain
                            })
                        })
                        .unwrap_or(0);
                    let pos = tag_of(0).split([',', ' ']).next().unwrap_or_default();
                    let plural_gender = ud::plural_only_gender(
                        &(0..count)
                            .map(|form| (tag_of(form), spell(paradigm, &stem, form)))
                            .collect::<Vec<_>>(),
                    );
                    let variant_main = VARIANT_LEMMAS
                        .iter()
                        .find(|&&(variant_pos, variant, _)| variant_pos == pos && variant == lemma)
                        .map(|&(_, _, main)| main);
                    if variant_main.is_some()
                        || VARIANT_LEMMAS
                            .iter()
                            .any(|&(main_pos, _, main)| main_pos == pos && main == lemma)
                    {
                        variant_lexemes
                            .entry((pos.to_owned(), lemma.clone()))
                            .or_insert(lexemes.len());
                    }
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
                            None => default_lemma,
                        };
                        // Перевод зависит от леммы UD: у «лучшей» это «лучший», а не «хороший» формы 0;
                        // у варианта - лемма основного слова
                        let ud_lemma = variant_main
                            .map_or_else(|| spell(paradigm, &stem, lemma_form), str::to_owned);
                        let class = ud::lemma_class(&ud_lemma).to_owned();
                        let counting = ud::counting_form(tag, &ud_lemma, &word);
                        // Сокращение существительного внутри его лексемы («им» в лексеме «имя»)
                        // и склоняемая аббревиатура («вуз»: Abbr и у формы 0, без Fixd)
                        let noun_abbreviation = tag.starts_with("NOUN") && tag.contains("Abbr");
                        let abbreviation = match (noun_abbreviation, tag_of(0).contains("Abbr")) {
                            (false, _) => None,
                            (true, false) => Some(Abbreviation::WordForm),
                            (true, true) if !tag.contains("Fixd") => Some(Abbreviation::Declinable),
                            (true, true) => None,
                        };
                        let gender =
                            plural_gender.filter(|_| tag.contains("Pltm") && tag.contains("GNdr"));
                        let key = (layout[count + form], class, counting, abbreviation, gender);
                        let (ud, untyped) = match ud_numbers.get(&key) {
                            Some(&value) => value,
                            None => {
                                let mut converted = ud::convert(tag, &key.1);
                                if let Some(gender) = gender {
                                    converted.feats.insert("Gender", gender);
                                }
                                if counting {
                                    converted.labels.push("Coun".into());
                                    converted.labels.sort();
                                }
                                match abbreviation {
                                    Some(Abbreviation::WordForm) => {
                                        ud::strip_abbreviation(&mut converted)
                                    }
                                    Some(Abbreviation::Declinable) => {
                                        ud::abbreviation_label(&mut converted)
                                    }
                                    None => {}
                                }
                                let labels: Vec<&str> =
                                    converted.labels.iter().map(String::as_str).collect();
                                let feats = ud::feats_string(&converted.feats);
                                let ud = builder.add_ud_tag(converted.pos, &feats, &labels)?;
                                if usize::from(ud) == ud_cores.len() {
                                    let next = core_numbers.len();
                                    let core = format!("{}\t{feats}", converted.pos);
                                    ud_cores.push(*core_numbers.entry(core).or_insert(next));
                                    let abbreviation = converted.feats.contains_key("Abbr")
                                        || converted.labels.iter().any(|label| label == "Abbr");
                                    ud_guesses.push(match converted.pos {
                                        _ if abbreviation => None,
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

    // Вариант и основное слово - одна лексема: «б» - форма лексемы «бы» с ее леммой
    let mut variant_mains: HashMap<usize, usize> = HashMap::new();
    for &(pos, variant, main) in VARIANT_LEMMAS {
        let find = |word: &str| {
            variant_lexemes
                .get(&(pos.to_owned(), word.to_owned()))
                .copied()
        };
        let (Some(variant), Some(main)) = (find(variant), find(main)) else {
            return Err(format!("нет лексем «{variant}» и «{main}» ({pos})").into());
        };
        variant_mains.insert(variant, main);
        let (a, b) = (root(&mut parents, variant), root(&mut parents, main));
        parents[a.max(b)] = a.min(b);
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
    // Разборы словоформы сверх ее записи в выгрузке: копии форм с другим разбором
    let mut extra_entries: HashMap<(usize, u16), Vec<Entry>> = HashMap::new();
    let mut combined_count = 0;
    for members in groups.values_mut() {
        // Основное слово раньше варианта: его форма - начальная форма объединенной лексемы
        members.sort_by_key(|lexeme| variant_mains.contains_key(lexeme));
        let members = &*members;
        if let [lexeme] = members.as_slice() {
            let (paradigm, stem, cells) = &lexemes[*lexeme];
            let layout = &paradigms[*paradigm as usize];
            let count = layout.len() / 3;
            let form_tags: Vec<&str> = (0..count)
                .map(|form| tags[layout[count + form] as usize].as_str())
                .collect();
            let copies = female_plurals(&form_tags, cells);
            // Копии форм с другим родом («двух»): на них, в отличие от копий фамилий, ссылается словоформа
            let base = spell(*paradigm, stem, 0);
            let mut genders = Vec::new();
            for (form, tag) in form_tags.iter().enumerate() {
                for converted in ud::numeral_genders(&base, tag) {
                    let labels: Vec<&str> = converted.labels.iter().map(String::as_str).collect();
                    let feats = ud::feats_string(&converted.feats);
                    genders.push((form, builder.add_ud_tag(converted.pos, &feats, &labels)?));
                }
            }
            if copies.is_empty() && genders.is_empty() {
                placed[*lexeme] = (builder.add_table(*paradigm, cells)?, Vec::new());
                continue;
            }
            let mut forms: Vec<(u16, u16, u16)> = (0..count)
                .map(|form| (layout[form], layout[count + form], layout[2 * count + form]))
                .collect();
            let mut cells = cells.clone();
            for &(form, lemma_form) in &copies {
                forms.push(forms[form]);
                cells.push((lemma_form, cells[form].1));
            }
            let mut referenced = Vec::with_capacity(genders.len());
            for &(form, ud) in &genders {
                referenced.push((form as u16, cells.len() as u16));
                forms.push(forms[form]);
                cells.push((cells[form].0, ud));
            }
            let paradigm = add_paradigm(&mut builder, &mut paradigm_numbers, forms)?;
            let table = builder.add_table(paradigm, &cells)?;
            placed[*lexeme] = (table, Vec::new());
            for (form, copy) in referenced {
                extra_entries
                    .entry((*lexeme, form))
                    .or_default()
                    .push(Entry { table, form: copy });
            }
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
        let (mut combined, numbers) = lexemes::combine(&sources);
        for (position, lexeme) in members.iter().enumerate() {
            let Some(main) = variant_mains.get(lexeme) else {
                continue;
            };
            let main_position = members
                .iter()
                .position(|member| member == main)
                .ok_or("вариант без основного слова в лексеме")?;
            let main_lemma = numbers[main_position][usize::from(lexemes[*main].2[0].0)];
            for &form in &numbers[position] {
                combined.forms[usize::from(form)].lemma_form = main_lemma;
            }
        }
        let form_tags: Vec<&str> = combined
            .forms
            .iter()
            .map(|form| tags[usize::from(form.tag)].as_str())
            .collect();
        let cells: Vec<(u16, u16)> = combined
            .forms
            .iter()
            .map(|form| (form.lemma_form, form.ud))
            .collect();
        for (form, lemma_form) in female_plurals(&form_tags, &cells) {
            combined.forms.push(lexemes::Form {
                lemma_form,
                ..combined.forms[form].clone()
            });
        }
        let mut forms = Vec::with_capacity(combined.forms.len());
        for form in &combined.forms {
            forms.push((
                builder.add_suffix(&form.suffix)?,
                form.tag,
                prefix_numbers[form.prefix.as_str()],
            ));
        }
        let paradigm = add_paradigm(&mut builder, &mut paradigm_numbers, forms)?;
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

    // Местоимения «то», «всё», «все» (PRON) - отдельные лексемы поверх форм «тот» и «весь»,
    // которые в OpenCorpora есть только как местоименные прилагательные; словоформа получает
    // и разбор местоимения
    for (lexeme, (paradigm, stem, _)) in lexemes.iter().enumerate() {
        let layout = &paradigms[*paradigm as usize];
        let count = layout.len() / 3;
        let base = spell(*paradigm, stem, 0);
        let mut pronouns: BTreeMap<&str, Vec<(usize, ud::Ud)>> = BTreeMap::new();
        for form in 0..count {
            let tag = &tags[layout[count + form] as usize];
            if let Some((lemma, converted)) = ud::substantive_pronoun(&base, tag) {
                pronouns.entry(lemma).or_default().push((form, converted));
            }
        }
        for forms in pronouns.values() {
            let lemma_form = forms
                .iter()
                .position(|&(form, _)| tags[layout[count + form] as usize].contains("nomn"))
                .unwrap_or(0) as u16;
            let table = copy_table(
                &mut builder,
                &mut paradigm_numbers,
                layout,
                forms,
                lemma_form,
            )?;
            for (number, &(form, _)) in forms.iter().enumerate() {
                extra_entries
                    .entry((lexeme, form as u16))
                    .or_default()
                    .push(Entry {
                        table,
                        form: number as u16,
                    });
            }
        }
    }

    // Разборы, которые различает только контекст («есть» - AUX и VERB, «один» - NUM и DET), -
    // лексемы из копий форм с леммой исходной лексемы: словоформа получает и разбор копии
    for (lexeme, (paradigm, stem, cells)) in lexemes.iter().enumerate() {
        let layout = &paradigms[*paradigm as usize];
        let count = layout.len() / 3;
        let base = spell(*paradigm, stem, 0);
        let forms: Vec<(usize, ud::Ud)> = (0..count)
            .filter_map(|form| {
                ud::context_copy(&base, &tags[layout[count + form] as usize])
                    .map(|converted| (form, converted))
            })
            .collect();
        let Some(&(first, _)) = forms.first() else {
            continue;
        };
        let lemma = usize::from(cells[first].0);
        let Some(lemma_form) = forms.iter().position(|&(form, _)| form == lemma) else {
            continue;
        };
        let table = copy_table(
            &mut builder,
            &mut paradigm_numbers,
            layout,
            &forms,
            lemma_form as u16,
        )?;
        for (number, &(form, _)) in forms.iter().enumerate() {
            extra_entries
                .entry((lexeme, form as u16))
                .or_default()
                .push(Entry {
                    table,
                    form: number as u16,
                });
        }
    }

    // Сравнительная степень наречия («раньше» - «рано»): лексема из наречия из одной формы и копий
    // форм COMP прилагательного, словоформа получает и разбор наречия. Таблица самого наречия
    // не меняется: свое слово по образцу наречия получает одну форму, как раньше
    let mut adverbs: HashMap<String, usize> = HashMap::new();
    for (lexeme, (paradigm, stem, _)) in lexemes.iter().enumerate() {
        let layout = &paradigms[*paradigm as usize];
        if layout.len() == 3 && tags[layout[1] as usize].starts_with("ADVB") {
            adverbs.entry(spell(*paradigm, stem, 0)).or_insert(lexeme);
        }
    }
    let mut comparatives: BTreeMap<usize, Vec<(usize, Vec<usize>)>> = BTreeMap::new();
    for (lexeme, (paradigm, stem, _)) in lexemes.iter().enumerate() {
        let layout = &paradigms[*paradigm as usize];
        let count = layout.len() / 3;
        let tag_of = |form: usize| tags[layout[count + form] as usize].as_str();
        if !tag_of(0).starts_with("ADJF") {
            continue;
        }
        let forms: Vec<usize> = (0..count)
            .filter(|&form| tag_of(form).starts_with("COMP"))
            .collect();
        if forms.is_empty() {
            continue;
        }
        let short: Vec<String> = (0..count)
            .filter(|&form| {
                let grammemes: Vec<&str> = tag_of(form).split([',', ' ']).collect();
                grammemes[0] == "ADJS" && grammemes.contains(&"neut") && grammemes.contains(&"sing")
            })
            .map(|form| spell(*paradigm, stem, form))
            .collect();
        let lemma = spell(*paradigm, stem, 0);
        if let Some(&adverb) = ud::positive_adverbs(&lemma, &short)
            .iter()
            .find_map(|word| adverbs.get(word))
        {
            comparatives
                .entry(adverb)
                .or_default()
                .push((lexeme, forms));
        }
    }
    let source_lexeme = |lexeme: usize, forms: &[usize], ud: &dyn Fn(usize) -> u16| {
        let (paradigm, stem, cells) = &lexemes[lexeme];
        let layout = &paradigms[*paradigm as usize];
        let count = layout.len() / 3;
        lexemes::Lexeme {
            stem: stem.clone(),
            forms: forms
                .iter()
                .map(|&form| lexemes::Form {
                    prefix: prefixes[layout[2 * count + form] as usize].clone(),
                    suffix: suffixes[layout[form] as usize].clone(),
                    tag: layout[count + form],
                    lemma_form: cells[form].0,
                    ud: ud(form),
                })
                .collect(),
        }
    };

    // Пассив на -ся («используется» - и «использовать» с Voice=Pass): лексема из инфинитива
    // невозвратного глагола и копий форм возвратного глагола несовершенного вида. Только у глаголов,
    // пассив которых размечен в трибанках (в счетчиках - словоформа на -ся с невозвратной леммой):
    // у других копия лишь делила бы долю с Mid
    let counts = frequencies::read(&read_ud("counts.tsv")?)?;
    let reflexive = |word: &str| word.ends_with("ся") || word.ends_with("сь");
    let attested: HashSet<&str> = counts
        .iter()
        .filter(|(word, _)| reflexive(word))
        .flat_map(|(_, pairs)| pairs)
        .filter(|(pos, lemma, ..)| pos == "VERB" && !reflexive(lemma))
        .map(|(_, lemma, ..)| lemma.as_str())
        .collect();
    let mut infinitives: HashMap<String, usize> = HashMap::new();
    for (lexeme, (paradigm, stem, _)) in lexemes.iter().enumerate() {
        let layout = &paradigms[*paradigm as usize];
        if tags[layout[layout.len() / 3] as usize].starts_with("INFN") {
            infinitives
                .entry(spell(*paradigm, stem, 0))
                .or_insert(lexeme);
        }
    }
    let mut passives: BTreeMap<usize, Vec<(usize, Vec<usize>)>> = BTreeMap::new();
    for (lexeme, (paradigm, stem, _)) in lexemes.iter().enumerate() {
        let layout = &paradigms[*paradigm as usize];
        let count = layout.len() / 3;
        let tag_of = |form: usize| tags[layout[count + form] as usize].as_str();
        let first: Vec<&str> = tag_of(0).split([',', ' ']).collect();
        if first[0] != "INFN" || !first.contains(&"impf") {
            continue;
        }
        let lemma = spell(*paradigm, stem, 0);
        let Some(base) = lemma
            .strip_suffix("ся")
            .or_else(|| lemma.strip_suffix("сь"))
        else {
            continue;
        };
        let Some(&active) = infinitives.get(base).filter(|_| attested.contains(base)) else {
            continue;
        };
        let forms: Vec<usize> = (0..count)
            .filter(|&form| ud::reflexive_passive(tag_of(form), &lemma).is_some())
            .collect();
        passives.entry(active).or_default().push((lexeme, forms));
    }

    // Копии форм с леммой другой лексемы из одной формы: наречия и инфинитива
    let jobs = (comparatives
        .iter()
        .map(|(lexeme, sources)| (*lexeme, sources, false)))
    .chain(
        passives
            .iter()
            .map(|(lexeme, sources)| (*lexeme, sources, true)),
    );
    for (target, sources, passive) in jobs {
        let word = spell(lexemes[target].0, &lexemes[target].1, 0);
        let mut members = vec![source_lexeme(target, &[0], &|_| lexemes[target].2[0].1)];
        for (lexeme, forms) in sources {
            let (paradigm, stem, _) = &lexemes[*lexeme];
            let layout = &paradigms[*paradigm as usize];
            let count = layout.len() / 3;
            let lemma = spell(*paradigm, stem, 0);
            let mut uds = HashMap::new();
            for &form in forms {
                let tag = &tags[layout[count + form] as usize];
                let converted = match passive {
                    true => ud::reflexive_passive(tag, &lemma).ok_or("пассив не глагола")?,
                    false => ud::comparative_adverb(tag, &word),
                };
                let labels: Vec<&str> = converted.labels.iter().map(String::as_str).collect();
                let feats = ud::feats_string(&converted.feats);
                uds.insert(form, builder.add_ud_tag(converted.pos, &feats, &labels)?);
            }
            members.push(source_lexeme(*lexeme, forms, &|form| uds[&form]));
        }
        let (mut combined, numbers) = lexemes::combine(&members);
        let lemma_form = numbers[0][0];
        for form in &mut combined.forms {
            form.lemma_form = lemma_form;
        }
        let mut forms = Vec::with_capacity(combined.forms.len());
        for form in &combined.forms {
            forms.push((
                builder.add_suffix(&form.suffix)?,
                form.tag,
                prefix_numbers[form.prefix.as_str()],
            ));
        }
        let paradigm = add_paradigm(&mut builder, &mut paradigm_numbers, forms)?;
        let cells: Vec<(u16, u16)> = combined
            .forms
            .iter()
            .map(|form| (form.lemma_form, form.ud))
            .collect();
        let table = builder.add_table(paradigm, &cells)?;
        for ((lexeme, forms), mapped) in sources.iter().zip(&numbers[1..]) {
            for (&form, &number) in forms.iter().zip(mapped) {
                extra_entries
                    .entry((*lexeme, form as u16))
                    .or_default()
                    .push(Entry {
                        table,
                        form: number,
                    });
            }
        }
    }

    // Имена собственные из `ud::PROPER_COPIES` («Земля», «Луна») - лексемы из форм единственного
    // числа нарицательного существительного: словоформа получает и разбор PROPN
    for (lexeme, (paradigm, stem, cells)) in lexemes.iter().enumerate() {
        let layout = &paradigms[*paradigm as usize];
        let count = layout.len() / 3;
        let tag_of = |form: usize| tags[layout[count + form] as usize].as_str();
        let lemma = spell(*paradigm, stem, 0);
        let Some(&(_, name_type)) = ud::PROPER_COPIES.iter().find(|(word, _)| *word == lemma)
        else {
            continue;
        };
        if !tag_of(0).starts_with("NOUN,") {
            continue;
        }
        let forms: Vec<usize> = (0..count)
            .filter(|&form| {
                tag_of(form)
                    .split([',', ' '])
                    .any(|grammeme| grammeme == "sing")
            })
            .collect();
        let layout_forms: Vec<(u16, u16, u16)> = forms
            .iter()
            .map(|&form| (layout[form], layout[count + form], layout[2 * count + form]))
            .collect();
        let copy_paradigm = add_paradigm(&mut builder, &mut paradigm_numbers, layout_forms)?;
        let mut copy_cells = Vec::with_capacity(forms.len());
        for &form in &forms {
            let mut converted = ud::convert(tag_of(form), ud::lemma_class(&lemma));
            converted.pos = "PROPN";
            converted.feats.insert("NameType", name_type);
            let labels: Vec<&str> = converted.labels.iter().map(String::as_str).collect();
            let feats = ud::feats_string(&converted.feats);
            let lemma_form = forms
                .iter()
                .position(|&known| known == usize::from(cells[form].0))
                .unwrap_or(0);
            copy_cells.push((
                lemma_form as u16,
                builder.add_ud_tag(converted.pos, &feats, &labels)?,
            ));
        }
        let table = builder.add_table(copy_paradigm, &copy_cells)?;
        for (number, &form) in forms.iter().enumerate() {
            extra_entries
                .entry((lexeme, form as u16))
                .or_default()
                .push(Entry {
                    table,
                    form: number as u16,
                });
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
            if let Some(extra) = extra_entries.get(&(lexeme, form)) {
                entries.extend(extra);
            }
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

    // Доли разборов по счетчикам UD считаются по разборам уже собранного словаря
    let first = Dictionary::from_bytes(&builder.build()?)?;
    let blended = frequencies::blend(&first, &counts);
    for (word, list) in &blended {
        builder.set_entry_probabilities(word, list)?;
    }
    if let Some(context) = context {
        let weights = fs::read(context).map_err(|err| format!("{}: {err}", context.display()))?;
        builder.context(weights)?;
    }

    let data = builder.build()?;
    let dictionary = Dictionary::from_bytes(&data)?;
    // Тег, которого не было в словаре обучения, модель не знает: ее нужно переобучить
    if context.is_some() {
        let gaps = dictionary.context_gaps()?;
        if !gaps.is_empty() {
            return Err(format!(
                "контекстная модель обучена на словаре без тегов {}: соберите словарь без весов \
                 и переобучите ее (scripts/train_context.py)",
                gaps.join(", ")
            )
            .into());
        }
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    // Новый файл заменяет старый переименованием: процесс, который держит старый словарь
    // отображенным в память, дочитывает старый файл, а не падает
    let mut temporary = output.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, &data)
        .and_then(|()| fs::rename(&temporary, output))
        .map_err(|err| format!("{}: {err}", output.display()))?;
    eprintln!(
        "{}: {} словоформ, {} тегов UD, {} таблиц словоизменения, {combined_count} лексем \
         объединено, {guess_endings} окончаний для угадывания, {} словоформ с долями по UD, \
         {:.1} МБ, {:.1} с",
        output.display(),
        dictionary.word_count(),
        dictionary.ud_tags().len(),
        placed
            .iter()
            .map(|(table, _)| table)
            .collect::<BTreeSet<_>>()
            .len(),
        blended.len(),
        data.len() as f64 / 1e6,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Формы множественного числа фамилии («Ивановы») в OpenCorpora - общего рода и с мужской леммой,
/// поэтому в женской лексеме («Иванова») их нет. Вывод - номера таких форм и форма женской леммы:
/// их копии добавляются в таблицу, словоформы на копии не ссылаются
fn female_plurals(tags: &[&str], cells: &[(u16, u16)]) -> Vec<(usize, u16)> {
    let has = |form: usize, grammemes: &[&str]| {
        let tag: Vec<&str> = tags[form].split([',', ' ']).collect();
        grammemes.iter().all(|grammeme| tag.contains(grammeme))
    };
    let Some(female) = (0..tags.len())
        .find(|&form| has(form, &["femn", "Surn"]) && usize::from(cells[form].0) == form)
    else {
        return Vec::new();
    };
    (0..tags.len())
        .filter(|&form| has(form, &["plur", "Surn"]) && usize::from(cells[form].0) != female)
        .map(|form| (form, female as u16))
        .collect()
}

/// Таблица лексемы из копий форм `forms` парадигмы `layout` с разборами копий и формой леммы
/// `lemma_form` среди них
fn copy_table(
    builder: &mut Builder,
    paradigm_numbers: &mut HashMap<Vec<(u16, u16, u16)>, u16>,
    layout: &[u16],
    forms: &[(usize, ud::Ud)],
    lemma_form: u16,
) -> Result<u16, rumorph::Error> {
    let count = layout.len() / 3;
    let layout_forms: Vec<(u16, u16, u16)> = forms
        .iter()
        .map(|&(form, _)| (layout[form], layout[count + form], layout[2 * count + form]))
        .collect();
    let paradigm = add_paradigm(builder, paradigm_numbers, layout_forms)?;
    let mut cells = Vec::with_capacity(forms.len());
    for (_, converted) in forms {
        let labels: Vec<&str> = converted.labels.iter().map(String::as_str).collect();
        let feats = ud::feats_string(&converted.feats);
        cells.push((
            lemma_form,
            builder.add_ud_tag(converted.pos, &feats, &labels)?,
        ));
    }
    builder.add_table(paradigm, &cells)
}

/// Номер парадигмы с формами `forms`: существующей или новой
fn add_paradigm(
    builder: &mut Builder,
    numbers: &mut HashMap<Vec<(u16, u16, u16)>, u16>,
    forms: Vec<(u16, u16, u16)>,
) -> Result<u16, rumorph::Error> {
    if let Some(&number) = numbers.get(&forms) {
        return Ok(number);
    }
    let number = builder.add_paradigm(&forms)?;
    numbers.insert(forms, number);
    Ok(number)
}

/// Корень множества лексемы в системе непересекающихся множеств
fn root(parents: &mut [usize], mut lexeme: usize) -> usize {
    while parents[lexeme] != lexeme {
        parents[lexeme] = parents[parents[lexeme]];
        lexeme = parents[lexeme];
    }
    lexeme
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn female_plurals_copy_plural_forms() {
        let tags = [
            "NOUN,anim,masc,Sgtm,Surn sing,nomn",
            "NOUN,anim,femn,Sgtm,Surn sing,nomn",
            "NOUN,anim,femn,Sgtm,Surn sing,gent",
            "NOUN,anim,ms-f,Pltm,Surn plur,nomn",
        ];
        // Женские формы - с леммой формы 1, множественное - с мужской леммой формы 0
        let cells = [(0, 10), (1, 11), (1, 12), (0, 13)];
        assert_eq!(female_plurals(&tags, &cells), [(3, 1)]);
        assert!(female_plurals(&tags[..1], &cells[..1]).is_empty());
    }
}
