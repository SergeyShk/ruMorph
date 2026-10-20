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

/// Ключ перевода в UD: тег OpenCorpora, класс леммы, счетная форма и вид аббревиатуры
type UdKey = (u16, String, bool, Option<Abbreviation>);

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
    builder.meta(read("meta.json")?);
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
                        let key = (layout[count + form], class, counting, abbreviation);
                        let (ud, untyped) = match ud_numbers.get(&key) {
                            Some(&value) => value,
                            None => {
                                let mut converted = ud::convert(tag, &key.1);
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
    let mut combined_count = 0;
    for members in groups.values_mut() {
        // Основное слово раньше варианта: его форма - начальная форма объединенной лексемы
        members.sort_by_key(|lexeme| variant_mains.contains_key(lexeme));
        let members = &*members;
        if let [lexeme] = members.as_slice() {
            let (paradigm, _, cells) = &lexemes[*lexeme];
            let layout = &paradigms[*paradigm as usize];
            let count = layout.len() / 3;
            let form_tags: Vec<&str> = (0..count)
                .map(|form| tags[layout[count + form] as usize].as_str())
                .collect();
            let copies = female_plurals(&form_tags, cells);
            if copies.is_empty() {
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
            let paradigm = add_paradigm(&mut builder, &mut paradigm_numbers, forms)?;
            placed[*lexeme] = (builder.add_table(paradigm, &cells)?, Vec::new());
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
    let mut pronoun_entries: HashMap<(usize, u16), Vec<Entry>> = HashMap::new();
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
            let layout_forms: Vec<(u16, u16, u16)> = forms
                .iter()
                .map(|&(form, _)| (layout[form], layout[count + form], layout[2 * count + form]))
                .collect();
            let pronoun_paradigm = add_paradigm(&mut builder, &mut paradigm_numbers, layout_forms)?;
            let mut cells = Vec::with_capacity(forms.len());
            for (_, converted) in forms {
                let labels: Vec<&str> = converted.labels.iter().map(String::as_str).collect();
                let feats = ud::feats_string(&converted.feats);
                cells.push((
                    lemma_form,
                    builder.add_ud_tag(converted.pos, &feats, &labels)?,
                ));
            }
            let table = builder.add_table(pronoun_paradigm, &cells)?;
            for (number, &(form, _)) in forms.iter().enumerate() {
                pronoun_entries
                    .entry((lexeme, form as u16))
                    .or_default()
                    .push(Entry {
                        table,
                        form: number as u16,
                    });
            }
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
            if let Some(pronouns) = pronoun_entries.get(&(lexeme, form)) {
                entries.extend(pronouns);
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
