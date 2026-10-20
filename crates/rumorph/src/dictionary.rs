use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use fst::Map;
use fst::raw::{Fst, Node, Output};

use crate::format::{self, Sections};
use crate::{Entry, Error};

mod guess;
mod rules;

/// Откуда взят разбор
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Словоформа есть в словаре
    Dictionary,
    /// Токена нет в словаре, разбор - по правилу: число, знак, латиница, римское число, инициал,
    /// аббревиатура или сокращение с точкой
    Rule,
    /// Слова нет в словаре, разбор угадан по окончанию
    Guess,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Dictionary => "dictionary",
            Source::Rule => "rule",
            Source::Guess => "guess",
        }
    }
}

/// Часть речи, признаки и пометы Universal Dependencies
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UdTag {
    pub pos: String,
    /// Признаки по алфавиту: («Case», «Gen»)
    pub feats: Vec<(String, String)>,
    /// Пометы словаря, у которых нет признака UD
    pub labels: Vec<String>,
}

impl UdTag {
    /// Тег из записи словаря «часть речи\tпризнаки CoNLL-U\tпометы через запятую»
    fn parse(line: &str) -> Option<Self> {
        let mut fields = line.split('\t');
        let (pos, feats, labels) = (fields.next()?, fields.next()?, fields.next()?);
        if pos.is_empty() || fields.next().is_some() {
            return None;
        }
        let split = |text: &str, separator: char| {
            text.split(separator)
                .filter(|part| !part.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let feats = split(feats, '|')
            .iter()
            .map(|feat| {
                let (name, value) = feat.split_once('=')?;
                Some((name.to_owned(), value.to_owned()))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(UdTag {
            pos: pos.to_owned(),
            feats,
            labels: split(labels, ','),
        })
    }

    /// Признаки в записи CoNLL-U: «Case=Gen|Number=Sing»
    pub fn feats_string(&self) -> String {
        self.feats
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// Разбор словоформы
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    /// Словоформа в написании словаря: с буквой «ё», если она есть в словаре
    pub word: String,
    /// Начальная форма по соглашениям UD
    pub lemma: String,
    /// Номер тега UD в `Dictionary::ud_tags`
    pub ud: u16,
    /// Номер тега OpenCorpora в `Dictionary::tags`; у разборов по правилам его может не быть
    pub tag: Option<u16>,
    /// Вероятность разбора; сумма по всем разборам словоформы равна 1
    pub prob: f64,
    pub source: Source,
    /// Форма таблицы словоизменения; у разборов без лексемы (число, знак, латиница) ее нет
    pub entry: Option<Entry>,
}

/// Тег UD для подбора формы: часть речи, коды значений признаков из `Dictionary::features`
/// по порядку (0 - признака нет) и номера помет
struct Codes {
    pos: u16,
    values: Vec<u16>,
    labels: Vec<u16>,
}

/// Лексема: формы парадигмы с общей основой и их пары «форма леммы, тег UD» из таблицы
struct Lexeme<'a> {
    table: u16,
    /// Номера суффиксов, тегов и префиксов форм тремя блоками
    forms: &'a [u16],
    /// `(форма леммы << 16) | тег UD` каждой формы
    cells: &'a [u32],
    stem: &'a str,
}

impl Lexeme<'_> {
    fn lemma_form(&self, form: u16) -> u16 {
        (self.cells[usize::from(form)] >> 16) as u16
    }

    fn ud(&self, form: u16) -> u16 {
        self.cells[usize::from(form)] as u16
    }

    fn tag(&self, form: u16) -> u16 {
        self.forms[self.cells.len() + usize::from(form)]
    }

    /// Номера префикса и суффикса формы
    fn affixes(&self, form: u16) -> (usize, usize) {
        let form = usize::from(form);
        (
            usize::from(self.forms[2 * self.cells.len() + form]),
            usize::from(self.forms[form]),
        )
    }
}

/// Словарь словоформ
pub struct Dictionary {
    meta: String,
    tags: Vec<String>,
    ud_tags: Vec<UdTag>,
    /// Номер пары «часть речи, признаки» тега UD: разборы, которые отличаются только пометами, сливаются
    ud_core: Vec<u16>,
    ud_codes: Vec<Codes>,
    rule_tags: rules::Tags,
    /// Номера частей речи и помет в `ud_codes`
    code_ids: HashMap<String, u16>,
    features: BTreeMap<String, BTreeSet<String>>,
    prefixes: Vec<String>,
    suffixes: Vec<String>,
    paradigm_offsets: Vec<u32>,
    paradigms: Vec<u16>,
    tables: Vec<u32>,
    /// Начало каждой таблицы словоизменения в `tables`
    table_offsets: Vec<u32>,
    words: Map<Vec<u8>>,
    records: Vec<u32>,
    probabilities: Map<Vec<u8>>,
    probability_records: Vec<u32>,
    guesses: Map<Vec<u8>>,
    guess_records: Vec<u32>,
}

impl Dictionary {
    /// Читает словарь из файла
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::from_bytes(&std::fs::read(path)?)
    }

    /// Читает словарь из байтов файла словаря
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error> {
        let sections = format::read(data)?;
        check_paradigms(&sections)?;
        let mut ud_tags = sections
            .ud_tags
            .iter()
            .map(|line| UdTag::parse(line))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Error::Format("неверный тег UD".into()))?;
        let table_offsets = check_tables(&sections, ud_tags.len())?;
        let rule_tags = rules::Tags::add(&mut ud_tags);
        let mut cores: HashMap<(&str, String), u16> = HashMap::new();
        let ud_core = ud_tags
            .iter()
            .map(|tag| {
                let next = cores.len() as u16;
                *cores.entry((&tag.pos, tag.feats_string())).or_insert(next)
            })
            .collect();
        let pairs: HashSet<&(String, String)> = ud_tags.iter().flat_map(|tag| &tag.feats).collect();
        let mut features: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (name, value) in pairs {
            features
                .entry(name.clone())
                .or_default()
                .insert(value.clone());
        }
        let (ud_codes, code_ids) = codes(&ud_tags, &features);
        let fst_error = |err: fst::Error| Error::Format(err.to_string());
        let words = Map::new(sections.words).map_err(fst_error)?;
        let probabilities = Map::new(sections.probabilities).map_err(fst_error)?;
        let guesses = Map::new(sections.guesses).map_err(fst_error)?;
        // Поврежденный автомат грузится без ошибок, но паникует при обходе
        for map in [&words, &probabilities, &guesses] {
            map.as_fst().verify().map_err(fst_error)?;
        }
        Ok(Dictionary {
            words,
            probabilities,
            meta: sections.meta,
            tags: sections.tags,
            ud_tags,
            ud_core,
            ud_codes,
            rule_tags,
            code_ids,
            features,
            prefixes: sections.prefixes,
            suffixes: sections.suffixes,
            paradigm_offsets: sections.paradigm_offsets,
            paradigms: sections.paradigms,
            tables: sections.tables,
            table_offsets,
            records: sections.records,
            probability_records: sections.probability_records,
            guesses,
            guess_records: sections.guess_records,
        })
    }

    /// Описание источника словаря
    pub fn meta(&self) -> &str {
        &self.meta
    }

    /// Теги OpenCorpora: `Analysis::tag` - номер в этой таблице
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Теги UD: `Analysis::ud` - номер в этой таблице
    pub fn ud_tags(&self) -> &[UdTag] {
        &self.ud_tags
    }

    /// Признаки UD, которые встречаются в словаре, и их значения
    pub fn features(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.features
    }

    /// Число словоформ в словаре
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// Разборы словоформы по убыванию вероятности; регистр не важен, «е» может обозначать «ё»
    ///
    /// Разборы OpenCorpora, которые совпали в написании, лемме, части речи и признаках UD,
    /// сливаются в один с суммой вероятностей и пометами более вероятного. Токен, которого нет
    /// в словаре, разбирается по правилам (`Source::Rule`), а незнакомое слово кириллицей
    /// угадывается по окончанию (`Source::Guess`)
    pub fn analyze(&self, word: &str) -> Vec<Analysis> {
        let mut analyses = self.merge(self.candidates(word));
        if !analyses.is_empty() {
            self.reweight_numerals(&mut analyses);
            return analyses;
        }
        let analyses = self.rules(word);
        if !analyses.is_empty() {
            return analyses;
        }
        self.guess(word)
    }

    /// Разборы словаря до слияния с вероятностями P(тег | словоформа); без частот вероятности нулевые
    fn candidates(&self, word: &str) -> Vec<Analysis> {
        let lower = word.to_lowercase();
        let mut analyses = Vec::new();
        for (spelling, offset) in self.lookup(&lower) {
            for &entry in self.records_at(offset).unwrap_or_default() {
                let (table, form) = ((entry >> 16) as u16, entry as u16);
                let Some(lexeme) = self.lexeme(table, form, &spelling) else {
                    continue;
                };
                analyses.push(Analysis {
                    lemma: self.spell(&lexeme, lexeme.lemma_form(form)),
                    ud: lexeme.ud(form),
                    tag: Some(lexeme.tag(form)),
                    word: spelling.clone(),
                    prob: 0.0,
                    source: Source::Dictionary,
                    entry: Some(Entry { table, form }),
                });
            }
        }
        self.assign_probabilities(&lower, &mut analyses);
        analyses
    }

    /// Разборы OpenCorpora до перевода в UD и слияния: написание, тег, начальная форма парадигмы
    /// и вероятность (без частот - равная доля)
    pub fn opencorpora(&self, word: &str) -> Vec<(String, &str, String, f64)> {
        let analyses = self.candidates(word);
        let uniform = analyses.iter().all(|analysis| analysis.prob == 0.0);
        let share = 1.0 / analyses.len() as f64;
        analyses
            .into_iter()
            .filter_map(|analysis| {
                let entry = analysis.entry?;
                let lexeme = self.lexeme(entry.table, entry.form, &analysis.word)?;
                let lemma = self.spell(&lexeme, 0);
                let prob = if uniform { share } else { analysis.prob };
                Some((
                    analysis.word,
                    self.tags[usize::from(analysis.tag?)].as_str(),
                    lemma,
                    prob,
                ))
            })
            .collect()
    }

    /// Слияние разборов с одинаковыми написанием, леммой, частью речи и признаками; без частот
    /// каждый слитый разбор получает равную долю, и порядок словаря сохраняется
    fn merge(&self, analyses: Vec<Analysis>) -> Vec<Analysis> {
        let mut merged: Vec<Analysis> = Vec::with_capacity(analyses.len());
        for analysis in analyses {
            let core = self.ud_core[usize::from(analysis.ud)];
            match merged.iter_mut().find(|known| {
                self.ud_core[usize::from(known.ud)] == core
                    && known.lemma == analysis.lemma
                    && known.word == analysis.word
            }) {
                Some(known) => known.prob += analysis.prob,
                None => merged.push(analysis),
            }
        }
        if merged.iter().all(|analysis| analysis.prob == 0.0) {
            let prob = 1.0 / merged.len() as f64;
            for analysis in &mut merged {
                analysis.prob = prob;
            }
        } else {
            merged.sort_by(|a, b| b.prob.total_cmp(&a.prob));
        }
        merged
    }

    /// Разбор словоформы `word` как формы `entry` таблицы словоизменения с источником `source`;
    /// вероятность - 1 (для восстановления сохраненного разбора). Словарный разбор должен быть
    /// в словаре, у угаданного или разобранного по правилу проверяется только, что словоформа
    /// подходит к форме таблицы
    pub fn analysis(&self, word: &str, entry: Entry, source: Source) -> Option<Analysis> {
        let packed = (u32::from(entry.table) << 16) | u32::from(entry.form);
        if source == Source::Dictionary
            && !self
                .words
                .get(word)
                .and_then(|offset| self.records_at(offset))?
                .contains(&packed)
        {
            return None;
        }
        // Лемма - в нижнем регистре, как у угадывания; словоформа - как сохранена
        let lower = word.to_lowercase();
        let lexeme = self.lexeme(entry.table, entry.form, &lower)?;
        Some(Analysis {
            word: word.to_owned(),
            lemma: self.spell(&lexeme, lexeme.lemma_form(entry.form)),
            ud: lexeme.ud(entry.form),
            tag: Some(lexeme.tag(entry.form)),
            prob: 1.0,
            source,
            entry: Some(entry),
        })
    }

    /// Формы лексемы разбора в порядке парадигмы: формы с той же леммой, вероятность каждой - 1
    ///
    /// Формы, которые совпали в написании, части речи и признаках, сливаются, как в `analyze`
    pub fn forms(&self, analysis: &Analysis) -> Vec<Analysis> {
        if let Some(lower) = self.lowercased(analysis) {
            return self
                .forms(&lower)
                .into_iter()
                .map(|form| capitalized_like(&analysis.word, form))
                .collect();
        }
        if analysis.entry.is_none() {
            return vec![Analysis {
                prob: 1.0,
                ..analysis.clone()
            }];
        }
        let Some((lexeme, forms)) = self.lexeme_forms(analysis) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        forms
            .into_iter()
            .filter(|&form| {
                seen.insert((
                    lexeme.affixes(form),
                    self.ud_core[usize::from(lexeme.ud(form))],
                ))
            })
            .map(|form| self.form(analysis, &lexeme, form))
            .collect()
    }

    /// Форма лексемы с признаками `feats`; из подходящих - ближайшая к разбору по части речи,
    /// остальным признакам и пометам, при равенстве - первая в парадигме
    pub fn inflect(&self, analysis: &Analysis, feats: &[(&str, &str)]) -> Option<Analysis> {
        if let Some(lower) = self.lowercased(analysis) {
            return self
                .inflect(&lower, feats)
                .map(|form| capitalized_like(&analysis.word, form));
        }
        if analysis.entry.is_none() {
            let tag = self.ud_tags.get(usize::from(analysis.ud))?;
            let own = feats
                .iter()
                .all(|&(name, value)| tag.feats.iter().any(|(n, v)| n == name && v == value));
            return own.then(|| Analysis {
                prob: 1.0,
                ..analysis.clone()
            });
        }
        let (lexeme, forms) = self.lexeme_forms(analysis)?;
        let form = self.closest(analysis, &lexeme, &forms, feats, &[])?;
        Some(self.form(analysis, &lexeme, form))
    }

    /// Форма, согласованная с количеством `number`: «1 рубль», «2 рубля», «5 рублей»
    ///
    /// Согласуются существительные, прилагательные, местоименные прилагательные и причастия
    /// в любом падеже: «о 2 рублях», «вижу двух котов», «две новые книги». Существительные
    /// с адъективным склонением согласуются как прилагательные («два учёных»), из форм
    /// родительного падежа множественного числа берется счетная («5 вольт»). У слов без
    /// единственного числа («сутки») берется множественное: «1 сутки», «2 суток»
    pub fn agree(&self, analysis: &Analysis, number: u64) -> Option<Analysis> {
        if let Some(lower) = self.lowercased(analysis) {
            return self
                .agree(&lower, number)
                .map(|form| capitalized_like(&analysis.word, form));
        }
        let tag = self.ud_tags.get(usize::from(analysis.ud))?;
        let feat = |name: &str| {
            tag.feats
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.as_str())
        };
        // Разделительный падеж («чаю») согласуется как родительный
        let case = match feat("Case")? {
            "Par" => "Gen",
            case => case,
        };
        let (lexeme, forms) = self.lexeme_forms(analysis)?;
        let nominal = matches!(tag.pos.as_str(), "NOUN" | "PROPN");
        let adjectival = matches!(tag.pos.as_str(), "ADJ" | "DET")
            || feat("VerbForm") == Some("Part")
            || (nominal && self.adjectival_declension(&lexeme, &forms));
        if !adjectival && !nominal {
            return None;
        }
        let one = number % 10 == 1 && number % 100 != 11;
        let few = (2..=4).contains(&(number % 10)) && !(12..=14).contains(&(number % 100));
        let animate = feat("Animacy") == Some("Anim");
        let sing = [("Case", case), ("Number", "Sing")];
        let plur = [("Case", case), ("Number", "Plur")];
        let gen_sing = [("Case", "Gen"), ("Number", "Sing")];
        let gen_plur = [("Case", "Gen"), ("Number", "Plur")];
        let plur_inanimate = [("Case", case), ("Number", "Plur"), ("Animacy", "Inan")];
        let targets: Vec<&[(&str, &str)]> = if one {
            vec![&sing, &plur]
        } else if !matches!(case, "Nom" | "Acc") {
            vec![&plur]
        } else if case == "Acc" && animate && (adjectival || (few && number < 5)) {
            // «вижу двух котов», «вижу двадцать два учёных», но «вижу двадцать два кота»
            vec![&gen_plur]
        } else if adjectival && few && feat("Gender") == Some("Fem") {
            vec![&plur_inanimate, &plur]
        } else if few && !adjectival {
            vec![&gen_sing, &gen_plur]
        } else {
            vec![&gen_plur]
        };
        let counting: Vec<u16> = self.code_ids.get("Coun").copied().into_iter().collect();
        let form = targets
            .into_iter()
            .find_map(|feats| self.closest(analysis, &lexeme, &forms, feats, &counting))?;
        Some(self.form(analysis, &lexeme, form))
    }

    /// Склоняется ли существительное как прилагательное («учёный», «столовая», «Толстой»):
    /// по окончаниям именительного и родительного падежей единственного числа
    fn adjectival_declension(&self, lexeme: &Lexeme, forms: &[u16]) -> bool {
        let ends = |case: &str, endings: &[&str]| {
            let Some(required) = [("Case", case), ("Number", "Sing")]
                .into_iter()
                .map(|(name, value)| self.code(name, value))
                .collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            forms
                .iter()
                .find(|&&form| {
                    let codes = &self.ud_codes[usize::from(lexeme.ud(form))];
                    required
                        .iter()
                        .all(|&(name, value)| codes.values[name] == value)
                })
                .is_some_and(|&form| {
                    let word = self.spell(lexeme, form);
                    endings.iter().any(|ending| word.ends_with(ending))
                })
        };
        ends("Nom", &["ый", "ий", "ой", "ое", "ее", "ая", "яя"])
            && ends("Gen", &["ого", "его", "ой", "ей"])
    }

    /// Форма из `forms` с признаками `feats`, ближайшая к разбору: больше общих с ним части речи,
    /// признаков и помет, а также желательных помет `prefer`, затем меньше лишних помет и краткой
    /// формы, если ее не просили («кошкой», а не «кошкою»; «мешаемый», а не «мешаем»), затем
    /// раньше в парадигме - поэтому без рода и числа берется мужской род единственного числа
    fn closest(
        &self,
        analysis: &Analysis,
        lexeme: &Lexeme,
        forms: &[u16],
        feats: &[(&str, &str)],
        prefer: &[u16],
    ) -> Option<u16> {
        let required = feats
            .iter()
            .map(|&(name, value)| self.code(name, value))
            .collect::<Option<Vec<_>>>()?;
        let current = self.ud_codes.get(usize::from(analysis.ud))?;
        let mut wanted = current.values.clone();
        for &(name, value) in &required {
            wanted[name] = value;
        }
        let mut labels = current.labels.clone();
        for &label in prefer {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        let short = self.code("Variant", "Short");
        let mut best: Option<(u16, (usize, Reverse<usize>))> = None;
        for &form in forms {
            let codes = &self.ud_codes[usize::from(lexeme.ud(form))];
            if required
                .iter()
                .any(|&(name, value)| codes.values[name] != value)
            {
                continue;
            }
            let same = wanted
                .iter()
                .zip(&codes.values)
                .filter(|&(wanted, value)| *wanted != 0 && wanted == value)
                .count()
                + labels
                    .iter()
                    .filter(|label| codes.labels.contains(label))
                    .count()
                + usize::from(codes.pos == current.pos);
            let extra = codes
                .labels
                .iter()
                .filter(|label| !labels.contains(label))
                .count()
                + usize::from(short.is_some_and(|(name, value)| {
                    codes.values[name] == value && wanted[name] != value
                }));
            let score = (same, Reverse(extra));
            if best.is_none_or(|(_, top)| score > top) {
                best = Some((form, score));
            }
        }
        best.map(|(form, _)| form)
    }

    /// Номер признака в `features` и код значения
    fn code(&self, name: &str, value: &str) -> Option<(usize, u16)> {
        let (index, (_, values)) = self
            .features
            .iter()
            .enumerate()
            .find(|(_, (known, _))| *known == name)?;
        let position = values.iter().position(|known| known == value)?;
        Some((index, position as u16 + 1))
    }

    /// Лексема разбора и номера ее форм с той же леммой по порядку
    fn lexeme_forms<'a>(&'a self, analysis: &'a Analysis) -> Option<(Lexeme<'a>, Vec<u16>)> {
        let entry = analysis.entry?;
        let lexeme = self.lexeme(entry.table, entry.form, &analysis.word)?;
        let own = lexeme.lemma_form(entry.form);
        // Лемма формы та же, если ее форма леммы пишется так же
        let (prefix, suffix) = lexeme.affixes(own);
        let (prefix, suffix) = (&self.prefixes[prefix], &self.suffixes[suffix]);
        let forms = (0..lexeme.cells.len() as u16)
            .filter(|&form| {
                let lemma_form = lexeme.lemma_form(form);
                if lemma_form == own {
                    return true;
                }
                let (other_prefix, other_suffix) = lexeme.affixes(lemma_form);
                self.prefixes[other_prefix] == *prefix && self.suffixes[other_suffix] == *suffix
            })
            .collect();
        Some((lexeme, forms))
    }

    /// Разбор со словоформой в нижнем регистре, если лексема строится только по ней («2005-ГО»)
    fn lowercased(&self, analysis: &Analysis) -> Option<Analysis> {
        let entry = analysis.entry?;
        if self
            .lexeme(entry.table, entry.form, &analysis.word)
            .is_some()
        {
            return None;
        }
        let word = analysis.word.to_lowercase();
        (word != analysis.word && self.lexeme(entry.table, entry.form, &word).is_some()).then(
            || Analysis {
                word,
                ..analysis.clone()
            },
        )
    }

    /// Лексема словоформы `word` - формы `form` таблицы `table`
    fn lexeme<'a>(&'a self, table: u16, form: u16, word: &'a str) -> Option<Lexeme<'a>> {
        let start = *self.table_offsets.get(usize::from(table))? as usize;
        let forms = self.paradigm(self.tables[start] as u16)?;
        let cells = &self.tables[start + 1..start + 1 + forms.len() / 3];
        if usize::from(form) >= cells.len() {
            return None;
        }
        let mut lexeme = Lexeme {
            table,
            forms,
            cells,
            stem: "",
        };
        let (prefix, suffix) = lexeme.affixes(form);
        lexeme.stem = word
            .strip_prefix(self.prefixes[prefix].as_str())?
            .strip_suffix(self.suffixes[suffix].as_str())?;
        Some(lexeme)
    }

    /// Написание формы лексемы
    fn spell(&self, lexeme: &Lexeme, form: u16) -> String {
        let (prefix, suffix) = lexeme.affixes(form);
        [
            self.prefixes[prefix].as_str(),
            lexeme.stem,
            self.suffixes[suffix].as_str(),
        ]
        .concat()
    }

    /// Разбор формы лексемы разбора `analysis`
    fn form(&self, analysis: &Analysis, lexeme: &Lexeme, form: u16) -> Analysis {
        let (prefix, suffix) = lexeme.affixes(form);
        let prefix = self.prefixes[prefix].as_str();
        let mut stem = lexeme.stem.chars();
        // У слова с заглавной буквы заглавной становится приставка формы: «Покринжовее»
        let word = match (prefix.chars().next(), stem.next()) {
            (Some(head), Some(first)) if first.is_uppercase() => [
                head.to_uppercase().to_string().as_str(),
                &prefix[head.len_utf8()..],
                first.to_lowercase().to_string().as_str(),
                stem.as_str(),
                self.suffixes[suffix].as_str(),
            ]
            .concat(),
            _ => self.spell(lexeme, form),
        };
        Analysis {
            word,
            lemma: analysis.lemma.clone(),
            ud: lexeme.ud(form),
            tag: Some(lexeme.tag(form)),
            prob: 1.0,
            source: analysis.source,
            entry: Some(Entry {
                table: lexeme.table,
                form,
            }),
        }
    }

    /// Есть ли словоформа в словаре
    pub fn is_known(&self, word: &str) -> bool {
        !self.lookup(&word.to_lowercase()).is_empty()
    }

    fn paradigm(&self, paradigm: u16) -> Option<&[u16]> {
        let i = paradigm as usize;
        let (start, end) = (
            *self.paradigm_offsets.get(i)?,
            *self.paradigm_offsets.get(i + 1)?,
        );
        self.paradigms.get(start as usize..end as usize)
    }

    /// Список разборов: `(таблица << 16) | форма` каждого
    fn records_at(&self, offset: u64) -> Option<&[u32]> {
        let start = usize::try_from(offset).ok()?;
        let count = *self.records.get(start)? as usize;
        let end = start.checked_add(1)?.checked_add(count)?;
        self.records.get(start + 1..end)
    }

    fn probabilities_for(&self, word: &str) -> Option<&[u32]> {
        let start = usize::try_from(self.probabilities.get(word)?).ok()?;
        let count = *self.probability_records.get(start)? as usize;
        let end = count.checked_mul(2)?.checked_add(start)?.checked_add(1)?;
        self.probability_records.get(start + 1..end)
    }

    /// P(тег | словоформа) берется для написания из запроса, а без частот у него - для написания
    /// разбора в словаре («еще» - частоты «ещё»); вероятность тега делится поровну между разборами
    /// с этим тегом и той же таблицей частот и нормируется. Без частот вероятности остаются
    /// нулевыми: равные доли раздает `merge` после слияния
    fn assign_probabilities(&self, word: &str, analyses: &mut [Analysis]) {
        let input = self.probabilities_for(word);
        let mut total = 0.0;
        for i in 0..analyses.len() {
            let (tag, spelling) = (analyses[i].tag, analyses[i].word.as_str());
            let table = match input {
                Some(table) => table,
                None if spelling == word => &[],
                None => self.probabilities_for(spelling).unwrap_or_default(),
            };
            let share = analyses
                .iter()
                .filter(|other| other.tag == tag && (input.is_some() || other.word == spelling))
                .count() as f64;
            let prob = table
                .as_chunks::<2>()
                .0
                .iter()
                .find(|[known, _]| Some(*known) == tag.map(u32::from))
                .map_or(0.0, |[_, millionths]| f64::from(*millionths));
            analyses[i].prob = prob / share;
            total += analyses[i].prob;
        }
        if total > 0.0 {
            for analysis in analyses.iter_mut() {
                analysis.prob /= total;
            }
            analyses.sort_by(|a, b| b.prob.total_cmp(&a.prob));
        }
    }

    /// Словоформы словаря, совпадающие с `key` с точностью до «е» на месте «ё»:
    /// сначала точное совпадение, затем варианты с «ё» по позициям слева направо
    fn lookup(&self, key: &str) -> Vec<(String, u64)> {
        let fst = self.words.as_fst();
        lookup_from(fst, String::new(), key, 0, fst.root(), Output::zero())
    }
}

fn lookup_from<'f>(
    fst: &'f Fst<Vec<u8>>,
    prefix: String,
    key: &str,
    start: usize,
    mut node: Node<'f>,
    mut output: Output,
) -> Vec<(String, u64)> {
    let mut found = Vec::new();
    let mut complete = true;
    for (offset, ch) in key[start..].char_indices() {
        let pos = start + offset;
        if ch == 'е'
            && let Some((next, next_output)) = follow(fst, node, output, "ё".as_bytes())
        {
            let variant = [prefix.as_str(), &key[start..pos], "ё"].concat();
            found.extend(lookup_from(
                fst,
                variant,
                key,
                pos + ch.len_utf8(),
                next,
                next_output,
            ));
        }
        let mut buf = [0u8; 4];
        match follow(fst, node, output, ch.encode_utf8(&mut buf).as_bytes()) {
            Some((next, next_output)) => (node, output) = (next, next_output),
            None => {
                complete = false;
                break;
            }
        }
    }
    if complete && node.is_final() {
        let value = output.cat(node.final_output()).value();
        found.insert(0, (prefix + &key[start..], value));
    }
    found
}

fn follow<'f>(
    fst: &'f Fst<Vec<u8>>,
    mut node: Node<'f>,
    mut output: Output,
    bytes: &[u8],
) -> Option<(Node<'f>, Output)> {
    for &byte in bytes {
        let transition = node.transition(node.find_input(byte)?);
        output = output.cat(transition.out);
        node = fst.node(transition.addr);
    }
    Some((node, output))
}

/// Парадигмы ссылаются только на существующие суффиксы, теги и префиксы
fn check_paradigms(sections: &Sections) -> Result<(), Error> {
    let offsets = &sections.paradigm_offsets;
    let invalid = |msg: &str| Err(Error::Format(msg.into()));
    if offsets.first() != Some(&0)
        || offsets.last().map(|&end| end as usize) != Some(sections.paradigms.len())
    {
        return invalid("границы парадигм не совпадают с таблицей парадигм");
    }
    if offsets.len() - 1 > usize::from(u16::MAX) + 1 {
        return invalid("больше 65 536 парадигм");
    }
    for bounds in offsets.windows(2) {
        let (start, end) = (bounds[0] as usize, bounds[1] as usize);
        if end <= start || (end - start) % 3 != 0 {
            return invalid("парадигма неверной длины");
        }
        let Some(forms) = sections.paradigms.get(start..end) else {
            return invalid("парадигма за пределами таблицы парадигм");
        };
        let count = forms.len() / 3;
        let tables = [
            sections.suffixes.len(),
            sections.tags.len(),
            sections.prefixes.len(),
        ];
        for (block, limit) in forms.chunks_exact(count).zip(tables) {
            if block.iter().any(|&id| usize::from(id) >= limit) {
                return invalid("парадигма ссылается за пределы таблицы");
            }
        }
    }
    Ok(())
}

/// Форма с заглавной первой буквой, если с нее начинается слово, от которого строились формы
/// в нижнем регистре («Покринжовее» → «Кринжового»)
fn capitalized_like(original: &str, form: Analysis) -> Analysis {
    let upper = original.chars().next().is_some_and(char::is_uppercase);
    let mut letters = form.word.chars();
    match letters.next() {
        Some(first) if upper && first.is_lowercase() => Analysis {
            word: first.to_uppercase().chain(letters).collect(),
            ..form
        },
        _ => form,
    }
}

/// Таблицы словоизменения ссылаются на существующие парадигмы, формы и теги UD; вывод - начала таблиц
fn check_tables(sections: &Sections, ud_count: usize) -> Result<Vec<u32>, Error> {
    let invalid = |msg: &str| Err(Error::Format(msg.into()));
    let tables = &sections.tables;
    let mut offsets = Vec::new();
    let mut start = 0;
    while start < tables.len() {
        if offsets.len() > usize::from(u16::MAX) {
            return invalid("больше 65 536 таблиц словоизменения");
        }
        let paradigm = tables[start] as usize;
        let Some(&[begin, end]) = paradigm
            .checked_add(2)
            .and_then(|stop| sections.paradigm_offsets.get(paradigm..stop))
        else {
            return invalid("таблица ссылается на несуществующую парадигму");
        };
        let count = (end - begin) as usize / 3;
        let Some(cells) = tables.get(start + 1..start + 1 + count) else {
            return invalid("таблица словоизменения обрезана");
        };
        if cells
            .iter()
            .any(|&cell| (cell >> 16) as usize >= count || (cell & 0xFFFF) as usize >= ud_count)
        {
            return invalid("таблица ссылается на несуществующую форму или тег UD");
        }
        offsets.push(start as u32);
        start += 1 + count;
    }
    Ok(offsets)
}

/// Коды тегов UD для подбора формы и номера частей речи и помет
fn codes(
    ud_tags: &[UdTag],
    features: &BTreeMap<String, BTreeSet<String>>,
) -> (Vec<Codes>, HashMap<String, u16>) {
    let mut values: HashMap<(&str, &str), (usize, u16)> = HashMap::new();
    for (index, (name, known)) in features.iter().enumerate() {
        for (position, value) in known.iter().enumerate() {
            values.insert((name, value), (index, position as u16 + 1));
        }
    }
    // Части речи и пометы сравниваются только между собой, поэтому номера у них общие
    let mut ids: HashMap<&str, u16> = HashMap::new();
    let mut id = |item| {
        let next = ids.len() as u16;
        *ids.entry(item).or_insert(next)
    };
    let codes = ud_tags
        .iter()
        .map(|tag| {
            let mut codes = vec![0; features.len()];
            for (name, value) in &tag.feats {
                let (index, code) = values[&(name.as_str(), value.as_str())];
                codes[index] = code;
            }
            Codes {
                pos: id(&tag.pos),
                values: codes,
                labels: tag.labels.iter().map(|label| id(label)).collect(),
            }
        })
        .collect();
    let ids = ids
        .into_iter()
        .map(|(item, id)| (item.to_owned(), id))
        .collect();
    (codes, ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Builder, Entry, GuessIndex};

    const TAGS: [&str; 6] = [
        "NOUN,anim,masc sing,nomn",
        "NOUN,anim,masc sing,gent",
        "VERB,perf,intr plur,past,indc",
        "NOUN,inan,femn sing,gent",
        "NOUN,inan,femn plur,nomn",
        "ADJF,Supr,Qual masc,sing,nomn",
    ];

    fn entry(table: u16, form: u16) -> Entry {
        Entry { table, form }
    }

    /// Парадигма из форм `(суффикс, тег, префикс)` и ее таблица: лемма - форма 0, тег UD - номер
    /// тега OpenCorpora; вывод - номер таблицы
    fn lexeme_table(builder: &mut Builder, forms: &[(u16, u16, u16)]) -> u16 {
        let paradigm = builder.add_paradigm(forms).unwrap();
        let cells: Vec<(u16, u16)> = forms.iter().map(|&(_, tag, _)| (0, tag)).collect();
        builder.add_table(paradigm, &cells).unwrap()
    }

    /// Сборщик с тегами UD «X|Tag=номер»: номер тега UD совпадает с номером тега OpenCorpora
    fn builder(tags: &[&str], prefixes: &[&str], suffixes: &[&str]) -> Builder {
        let owned = |items: &[&str]| items.iter().map(|item| item.to_string()).collect();
        let mut builder = Builder::new(owned(tags), owned(prefixes), owned(suffixes));
        for number in 0..tags.len() {
            builder
                .add_ud_tag("X", &format!("Tag={number}"), &[])
                .unwrap();
        }
        builder
    }

    /// ёж (ежа), стать (стали), сталь (стали), большой (наибольший), все и всё
    fn dictionary_bytes() -> Vec<u8> {
        let mut builder = builder(
            &TAGS,
            &["", "наи"],
            &["", "ёж", "ежа", "ть", "ли", "ь", "и", "ой", "ий"],
        );
        builder.meta("тест");
        let hedgehog = lexeme_table(&mut builder, &[(1, 0, 0), (2, 1, 0)]);
        let become_ = lexeme_table(&mut builder, &[(3, 2, 0), (4, 2, 0)]);
        let steel = lexeme_table(&mut builder, &[(5, 3, 0), (6, 3, 0), (6, 4, 0)]);
        let big = lexeme_table(&mut builder, &[(7, 5, 0), (8, 5, 1)]);
        let plain = lexeme_table(&mut builder, &[(0, 0, 0)]);
        builder.add_word("ёж", &[entry(hedgehog, 0)]).unwrap();
        builder.add_word("ежа", &[entry(hedgehog, 1)]).unwrap();
        builder
            .add_word(
                "стали",
                &[entry(steel, 1), entry(steel, 2), entry(become_, 1)],
            )
            .unwrap();
        builder.add_word("наибольший", &[entry(big, 1)]).unwrap();
        builder.add_word("все", &[entry(plain, 0)]).unwrap();
        builder.add_word("всё", &[entry(plain, 0)]).unwrap();
        builder.add_probability("стали", 2, 970_000).unwrap();
        builder.add_probability("стали", 3, 20_000).unwrap();
        builder.add_probability("стали", 4, 10_000).unwrap();
        builder.build().unwrap()
    }

    fn dictionary() -> Dictionary {
        Dictionary::from_bytes(&dictionary_bytes()).unwrap()
    }

    fn summary(analyses: &[Analysis]) -> Vec<(&str, &str, u16)> {
        analyses
            .iter()
            .map(|a| (a.word.as_str(), a.lemma.as_str(), a.tag.unwrap()))
            .collect()
    }

    #[test]
    fn lemma_from_paradigm() {
        let dict = dictionary();
        assert_eq!(summary(&dict.analyze("ежа")), [("ежа", "ёж", 1)]);
        assert_eq!(
            summary(&dict.analyze("наибольший")),
            [("наибольший", "большой", 5)]
        );
    }

    #[test]
    fn lemma_from_another_form() {
        let mut builder = builder(&["NOUN,masc", "NOUN,femn"], &[""], &["", "а", "ой"]);
        let paradigm = builder
            .add_paradigm(&[(0, 0, 0), (1, 1, 0), (2, 1, 0)])
            .unwrap();
        // Женские формы - с леммой «петрова», формой 1
        let surname = builder
            .add_table(paradigm, &[(0, 0), (1, 1), (1, 1)])
            .unwrap();
        builder.add_word("петровой", &[entry(surname, 2)]).unwrap();
        builder.add_word("петрова", &[entry(surname, 1)]).unwrap();
        builder.add_word("петров", &[entry(surname, 0)]).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        assert_eq!(dict.analyze("петровой")[0].lemma, "петрова");
        let words = |word: &str| -> Vec<String> {
            let analysis = &dict.analyze(word)[0];
            dict.forms(analysis)
                .into_iter()
                .map(|form| form.word)
                .collect()
        };
        assert_eq!(words("петровой"), ["петрова", "петровой"]);
        assert_eq!(words("петров"), ["петров"]);
        assert_eq!(
            dict.opencorpora("петровой"),
            [(
                "петровой".to_string(),
                "NOUN,femn",
                "петров".to_string(),
                1.0
            )]
        );
    }

    /// Формы лексемы: окончание и признаки UD
    type Forms = Vec<(&'static str, String)>;

    /// Лексемы «кот», «учёный», «сутки», «молоко», «новый» и «нести»
    fn lexicon() -> Dictionary {
        const NOUN: [&str; 12] = [
            "Case=Nom|Number=Sing",
            "Case=Gen|Number=Sing",
            "Case=Dat|Number=Sing",
            "Case=Acc|Number=Sing",
            "Case=Ins|Number=Sing",
            "Case=Loc|Number=Sing",
            "Case=Nom|Number=Plur",
            "Case=Gen|Number=Plur",
            "Case=Dat|Number=Plur",
            "Case=Acc|Number=Plur",
            "Case=Ins|Number=Plur",
            "Case=Loc|Number=Plur",
        ];
        let noun = |animacy: &str, endings: [&'static str; 12]| -> Forms {
            endings
                .into_iter()
                .zip(NOUN)
                .filter(|(ending, _)| *ending != "-")
                .map(|(ending, feats)| (ending, format!("Animacy={animacy}|{feats}")))
                .collect()
        };
        let lexemes: Vec<(&str, &str, Forms)> = vec![
            (
                "NOUN",
                "кот",
                noun(
                    "Anim",
                    [
                        "", "а", "у", "а", "ом", "е", "ы", "ов", "ам", "ов", "ами", "ах",
                    ],
                ),
            ),
            (
                "NOUN",
                "учён",
                noun(
                    "Anim",
                    [
                        "ый", "ого", "ому", "ого", "ым", "ом", "ые", "ых", "ым", "ых", "ыми", "ых",
                    ],
                ),
            ),
            (
                "NOUN",
                "сут",
                noun(
                    "Inan",
                    [
                        "-", "-", "-", "-", "-", "-", "ки", "ок", "кам", "ки", "ками", "ках",
                    ],
                ),
            ),
            (
                "NOUN",
                "молок",
                noun(
                    "Inan",
                    ["о", "а", "у", "о", "ом", "е", "-", "-", "-", "-", "-", "-"],
                ),
            ),
            (
                "ADJ",
                "нов",
                [
                    ("ый", "Case=Nom|Gender=Masc|Number=Sing"),
                    ("ая", "Case=Nom|Gender=Fem|Number=Sing"),
                    ("ую", "Case=Acc|Gender=Fem|Number=Sing"),
                    ("ые", "Case=Nom|Number=Plur"),
                    ("ых", "Case=Gen|Number=Plur"),
                    ("ых", "Animacy=Anim|Case=Acc|Number=Plur"),
                    ("ые", "Animacy=Inan|Case=Acc|Number=Plur"),
                ]
                .map(|(ending, feats)| (ending, feats.to_string()))
                .to_vec(),
            ),
            (
                "VERB",
                "нес",
                vec![
                    ("ти", "VerbForm=Inf".into()),
                    ("ла", "Gender=Fem|Tense=Past".into()),
                ],
            ),
        ];
        let mut tags: Vec<String> = Vec::new();
        let mut suffixes: Vec<String> = Vec::new();
        for (pos, _, forms) in &lexemes {
            for (ending, feats) in forms {
                let tag = format!("{pos} {feats}");
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
                if !suffixes.iter().any(|suffix| suffix == ending) {
                    suffixes.push(ending.to_string());
                }
            }
        }
        let mut builder = Builder::new(tags.clone(), vec![String::new()], suffixes.clone());
        for tag in &tags {
            let (pos, feats) = tag.split_once(' ').unwrap();
            builder.add_ud_tag(pos, feats, &[]).unwrap();
        }
        let id = |items: &[String], item: &str| {
            items.iter().position(|known| known == item).unwrap() as u16
        };
        for (pos, stem, forms) in &lexemes {
            let paradigm: Vec<(u16, u16, u16)> = forms
                .iter()
                .map(|(ending, feats)| {
                    (
                        id(&suffixes, ending),
                        id(&tags, &format!("{pos} {feats}")),
                        0,
                    )
                })
                .collect();
            let table = lexeme_table(&mut builder, &paradigm);
            for (form, (ending, _)) in forms.iter().enumerate() {
                builder
                    .add_word(&format!("{stem}{ending}"), &[entry(table, form as u16)])
                    .unwrap();
            }
        }
        Dictionary::from_bytes(&builder.build().unwrap()).unwrap()
    }

    /// Разбор словоформы с признаками `feats`
    fn find(dict: &Dictionary, word: &str, feats: &str) -> Analysis {
        dict.analyze(word)
            .into_iter()
            .find(|analysis| {
                dict.ud_tags()[usize::from(analysis.ud)]
                    .feats_string()
                    .ends_with(feats)
            })
            .unwrap()
    }

    #[test]
    fn forms_of_lexeme() {
        let dict = lexicon();
        let forms: Vec<String> = dict
            .forms(&dict.analyze("коту")[0])
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(
            forms,
            [
                "кот",
                "кота",
                "коту",
                "кота",
                "котом",
                "коте",
                "коты",
                "котов",
                "котам",
                "котов",
                "котами",
                "котах"
            ]
        );
        assert!(
            dict.forms(&dict.analyze("коту")[0])
                .iter()
                .all(|form| form.prob == 1.0 && form.lemma == "кот")
        );
    }

    #[test]
    fn inflect_keeps_other_features() {
        let dict = lexicon();
        let inflect = |word: &str, feats: &[(&str, &str)]| {
            dict.inflect(&dict.analyze(word)[0], feats)
                .map(|form| form.word)
        };
        assert_eq!(inflect("кот", &[("Case", "Gen")]).as_deref(), Some("кота"));
        assert_eq!(
            inflect("кот", &[("Case", "Gen"), ("Number", "Plur")]).as_deref(),
            Some("котов")
        );
        assert_eq!(
            inflect("котам", &[("Number", "Sing")]).as_deref(),
            Some("коту")
        );
        assert_eq!(inflect("кот", &[]).as_deref(), Some("кот"));
        assert_eq!(inflect("кот", &[("Tense", "Past")]), None);
        assert_eq!(inflect("сутки", &[("Number", "Sing")]), None);
        assert_eq!(
            inflect("нести", &[("Tense", "Past")]).as_deref(),
            Some("несла")
        );
    }

    #[test]
    fn agree_with_number() {
        let dict = lexicon();
        let agree = |analysis: &Analysis, numbers: &[u64]| -> Vec<Option<String>> {
            numbers
                .iter()
                .map(|&number| dict.agree(analysis, number).map(|form| form.word))
                .collect()
        };
        let words = |words: &[&str]| -> Vec<Option<String>> {
            words
                .iter()
                .map(|word| (*word != "-").then(|| word.to_string()))
                .collect()
        };
        let numbers = [0, 1, 2, 5, 11, 12, 21, 22, 25, 101, 112, 1_000_002];
        assert_eq!(
            agree(&dict.analyze("кот")[0], &numbers),
            words(&[
                "котов",
                "кот",
                "кота",
                "котов",
                "котов",
                "котов",
                "кот",
                "кота",
                "котов",
                "кот",
                "котов",
                "кота"
            ])
        );
        // Винительный падеж одушевленных: «вижу двух котов», но «вижу двадцать два кота»
        assert_eq!(
            agree(&find(&dict, "кота", "Case=Acc|Number=Sing"), &[1, 2, 5, 22]),
            words(&["кота", "котов", "котов", "кота"])
        );
        assert_eq!(
            agree(&dict.analyze("коту")[0], &[1, 2, 21]),
            words(&["коту", "котам", "коту"])
        );
        assert_eq!(
            agree(&dict.analyze("сутки")[0], &[1, 2, 5]),
            words(&["сутки", "суток", "суток"])
        );
        assert_eq!(
            agree(&dict.analyze("молоко")[0], &[1, 2, 5]),
            words(&["молоко", "молока", "-"])
        );
        assert_eq!(
            agree(&dict.analyze("новый")[0], &[1, 2, 5]),
            words(&["новый", "новых", "новых"])
        );
        assert_eq!(
            agree(&dict.analyze("новая")[0], &[1, 2, 5]),
            words(&["новая", "новые", "новых"])
        );
        assert_eq!(agree(&dict.analyze("новую")[0], &[2]), words(&["новые"]));
        assert_eq!(agree(&dict.analyze("нести")[0], &[2]), words(&["-"]));
        // Существительное с адъективным склонением согласуется как прилагательное
        assert_eq!(
            agree(&dict.analyze("учёный")[0], &[1, 2, 5, 22]),
            words(&["учёный", "учёных", "учёных", "учёных"])
        );
        assert_eq!(
            agree(&find(&dict, "учёного", "Case=Acc|Number=Sing"), &[2, 22]),
            words(&["учёных", "учёных"])
        );
        assert_eq!(agree(&dict.analyze("кот")[0], &[22]), words(&["кота"]));
    }

    #[test]
    fn merged_analyses_share_forms() {
        // «мешать» непереходный и переходный - одна таблица, как их объединяет сборщик;
        // причастие есть только у переходного
        let mut builder = Builder::new(
            vec!["INFN".into(), "VERB".into(), "PRTF".into()],
            vec![String::new()],
            vec!["ть".into(), "л".into(), "емый".into()],
        );
        let tags = [
            ("VerbForm=Inf", "intr"),
            ("Tense=Past|VerbForm=Fin", "intr"),
            ("VerbForm=Inf", "tran"),
            ("Tense=Past|VerbForm=Fin", "tran"),
            ("VerbForm=Part|Voice=Pass", "tran"),
        ];
        let cells: Vec<(u16, u16)> = tags
            .iter()
            .map(|&(feats, label)| (0, builder.add_ud_tag("VERB", feats, &[label]).unwrap()))
            .collect();
        let paradigm = builder
            .add_paradigm(&[(0, 0, 0), (1, 1, 0), (0, 0, 0), (1, 1, 0), (2, 2, 0)])
            .unwrap();
        let table = builder.add_table(paradigm, &cells).unwrap();
        builder
            .add_word("мешать", &[entry(table, 0), entry(table, 2)])
            .unwrap();
        builder
            .add_word("мешал", &[entry(table, 1), entry(table, 3)])
            .unwrap();
        builder.add_word("мешаемый", &[entry(table, 4)]).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let analyses = dict.analyze("мешать");
        assert_eq!(analyses.len(), 1);
        for word in ["мешать", "мешаемый"] {
            let words: Vec<String> = dict
                .forms(&dict.analyze(word)[0])
                .into_iter()
                .map(|form| form.word)
                .collect();
            assert_eq!(words, ["мешать", "мешал", "мешаемый"], "{word}");
        }
        let found = dict.inflect(&analyses[0], &[("Voice", "Pass")]).unwrap();
        assert_eq!(found.word, "мешаемый");
        let analysis = |word: &str, form: u16, source: Source| {
            dict.analysis(word, Entry { table, form }, source)
        };
        assert_eq!(analysis("мешаемый", 4, Source::Dictionary), Some(found));
        assert_eq!(analysis("мешаемый", 1, Source::Dictionary), None);
        assert_eq!(analysis("мешал", 4, Source::Dictionary), None);
        // Угаданному разбору словоформа в словаре не нужна
        let guessed = analysis("Зашаемый", 4, Source::Guess).unwrap();
        assert_eq!(
            (guessed.lemma.as_str(), guessed.source),
            ("зашать", Source::Guess)
        );
        assert_eq!(analysis("Зашаемый", 4, Source::Dictionary), None);
    }

    /// Словарь для угадывания: нарицательное «кот» (кот, кота) и фамилия «петров» (петров, петрова)
    fn guessing() -> Dictionary {
        let mut builder = Builder::new(
            vec!["NOUN,nomn".into(), "NOUN,gent".into()],
            vec![String::new()],
            vec![String::new(), "а".into()],
        );
        let nominative = builder.add_ud_tag("NOUN", "Case=Nom", &[]).unwrap();
        let genitive = builder.add_ud_tag("NOUN", "Case=Gen", &[]).unwrap();
        let surname = builder
            .add_ud_tag("PROPN", "Case=Gen|NameType=Sur", &[])
            .unwrap();
        let surname_nominative = builder
            .add_ud_tag("PROPN", "Case=Nom|NameType=Sur", &[])
            .unwrap();
        let paradigm = builder.add_paradigm(&[(0, 0, 0), (1, 1, 0)]).unwrap();
        let common = builder
            .add_table(paradigm, &[(0, nominative), (0, genitive)])
            .unwrap();
        let proper = builder
            .add_table(paradigm, &[(0, surname_nominative), (0, surname)])
            .unwrap();
        builder.add_word("кот", &[entry(common, 0)]).unwrap();
        builder.add_word("кота", &[entry(common, 1)]).unwrap();
        builder.add_word("петров", &[entry(proper, 0)]).unwrap();
        builder.add_word("петрова", &[entry(proper, 1)]).unwrap();
        builder
            .add_guesses(GuessIndex::Common, "а", &[(entry(common, 1), 5)])
            .unwrap();
        builder
            .add_guesses(GuessIndex::Common, "ова", &[(entry(common, 1), 1)])
            .unwrap();
        builder
            .add_guesses(GuessIndex::Proper, "ова", &[(entry(proper, 1), 3)])
            .unwrap();
        Dictionary::from_bytes(&builder.build().unwrap()).unwrap()
    }

    #[test]
    fn guess_by_longest_ending() {
        let dict = guessing();
        let analyses = dict.analyze("зумова");
        let summary: Vec<(&str, &str, Source)> = analyses
            .iter()
            .map(|a| (a.word.as_str(), a.lemma.as_str(), a.source))
            .collect();
        assert_eq!(summary, [("зумова", "зумов", Source::Guess)]);
        assert_eq!(dict.ud_tags()[usize::from(analyses[0].ud)].pos, "NOUN");
        assert_eq!(dict.analyze("зума")[0].lemma, "зум");
        assert!(dict.analyze("зумы").is_empty());
        assert!(dict.analyze("зумова1").is_empty());
    }

    #[test]
    fn capitalized_guess_prefers_proper_nouns() {
        let dict = guessing();
        let analyses = dict.analyze("Зумова");
        let summary: Vec<(&str, &str, &str)> = analyses
            .iter()
            .map(|a| {
                let tag = &dict.ud_tags()[usize::from(a.ud)];
                (a.word.as_str(), a.lemma.as_str(), tag.pos.as_str())
            })
            .collect();
        assert_eq!(
            summary,
            [("Зумова", "зумов", "PROPN"), ("Зумова", "зумов", "NOUN")]
        );
        assert!((analyses[0].prob - 0.89).abs() < 1e-12);
        // Формы угаданного слова сохраняют заглавную букву
        let forms: Vec<String> = dict
            .forms(&analyses[0])
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["Зумов", "Зумова"]);
        assert!(dict.analyze("ЗуМова").is_empty());
    }

    #[test]
    fn guess_with_yo_in_suffix() {
        // «котёнок» - суффикс с «ё», «баранка» - с «е»; окончание через «е» находит обе, но первой
        // идет «баранка», окончание с «ё» - только «котёнок»
        let mut builder = Builder::new(
            vec!["NOUN,nomn".into(), "NOUN,gent".into()],
            vec![String::new()],
            vec!["ёнок".into(), "ёнка".into(), "енка".into()],
        );
        let nominative = builder.add_ud_tag("NOUN", "Case=Nom", &[]).unwrap();
        let genitive = builder.add_ud_tag("NOUN", "Case=Gen", &[]).unwrap();
        let young = builder.add_paradigm(&[(0, 0, 0), (1, 1, 0)]).unwrap();
        let young = builder
            .add_table(young, &[(0, nominative), (0, genitive)])
            .unwrap();
        let plain = builder.add_paradigm(&[(2, 0, 0)]).unwrap();
        let plain = builder.add_table(plain, &[(0, nominative)]).unwrap();
        builder.add_word("котёнок", &[entry(young, 0)]).unwrap();
        builder.add_word("котёнка", &[entry(young, 1)]).unwrap();
        builder.add_word("баренка", &[entry(plain, 0)]).unwrap();
        builder
            .add_guesses(
                GuessIndex::Common,
                "енка",
                &[(entry(plain, 0), 5), (entry(young, 1), 3)],
            )
            .unwrap();
        builder
            .add_guesses(GuessIndex::Common, "ёнка", &[(entry(young, 1), 3)])
            .unwrap();
        builder
            .add_guesses(GuessIndex::Common, "нка", &[(entry(young, 1), 3)])
            .unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let summary = |word: &str| -> Vec<(String, String)> {
            dict.analyze(word)
                .into_iter()
                .map(|a| (a.word, a.lemma))
                .collect()
        };
        assert_eq!(
            summary("слонёнка"),
            [("слонёнка".to_string(), "слонёнок".to_string())]
        );
        let written_with_e = summary("слоненка");
        assert_eq!(
            written_with_e[1],
            ("слонёнка".to_string(), "слонёнок".to_string())
        );
        let forms: Vec<String> = dict
            .forms(&dict.analyze("слонёнка")[0])
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["слонёнок", "слонёнка"]);
        // Суффикс на все слово: регистр букв сохраняется, меняется только «е» на месте «ё»
        let whole = dict.analyze("Енка");
        assert_eq!(
            (whole[0].word.as_str(), whole[0].lemma.as_str()),
            ("Ёнка", "ёнок")
        );
        let forms: Vec<String> = dict
            .forms(&whole[0])
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["Ёнок", "Ёнка"]);
    }

    #[test]
    fn capitalized_guess_with_prefixed_form() {
        // Сравнительная степень с приставкой «по»: «Покринжовее», а не «поКринжовее»
        let mut builder = Builder::new(
            vec!["ADJF".into(), "COMP".into()],
            vec![String::new(), "по".into()],
            vec!["ый".into(), "ее".into()],
        );
        let positive = builder.add_ud_tag("ADJ", "Degree=Pos", &[]).unwrap();
        let comparative = builder.add_ud_tag("ADJ", "Degree=Cmp", &[]).unwrap();
        let paradigm = builder.add_paradigm(&[(0, 0, 0), (1, 1, 1)]).unwrap();
        let table = builder
            .add_table(paradigm, &[(0, positive), (0, comparative)])
            .unwrap();
        builder.add_word("новый", &[entry(table, 0)]).unwrap();
        builder.add_word("поновее", &[entry(table, 1)]).unwrap();
        builder
            .add_guesses(GuessIndex::Common, "ый", &[(entry(table, 0), 3)])
            .unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let analysis = &dict.analyze("Кринжовый")[0];
        let forms: Vec<String> = dict
            .forms(analysis)
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["Кринжовый", "Покринжовее"]);
        let comparative = dict.inflect(analysis, &[("Degree", "Cmp")]).unwrap();
        assert_eq!(comparative.word, "Покринжовее");
        let entry = comparative.entry.unwrap();
        let restored = dict.analysis("Покринжовее", entry, Source::Guess).unwrap();
        assert_eq!(restored.lemma, "кринжовый");
        // Формы формы с приставкой: заглавная первая буква сохраняется
        let forms: Vec<String> = dict
            .forms(&restored)
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["Кринжовый", "Покринжовее"]);
        let positive = dict.inflect(&restored, &[("Degree", "Pos")]).unwrap();
        assert_eq!(positive.word, "Кринжовый");
    }

    #[test]
    fn broken_guess_records_give_no_guesses() {
        let data = guessing_bytes();
        assert_eq!(
            Dictionary::from_bytes(&data).unwrap().analyze("зума")[0].lemma,
            "зума"
        );
        // Нулевое число словоформ в поврежденном файле не дает вероятности NaN
        for records in [vec![9, 0], vec![1, 0x0007_0000, 2], vec![1, 0, 0], vec![]] {
            let mut sections = format::read(&data).unwrap();
            sections.guess_records = records;
            let dict = Dictionary::from_bytes(&format::write(&sections)).unwrap();
            assert!(dict.analyze("зума").is_empty());
        }
    }

    /// Словарь с одним окончанием для угадывания
    fn guessing_bytes() -> Vec<u8> {
        let mut builder = Builder::new(
            vec!["NOUN".into()],
            vec![String::new()],
            vec![String::new(), "а".into()],
        );
        builder.add_ud_tag("NOUN", "", &[]).unwrap();
        let table = lexeme_table(&mut builder, &[(1, 0, 0)]);
        builder.add_word("кота", &[entry(table, 0)]).unwrap();
        builder
            .add_guesses(GuessIndex::Common, "а", &[(entry(table, 0), 2)])
            .unwrap();
        builder.build().unwrap()
    }

    #[test]
    fn yo_spelled_as_e_and_case() {
        let dict = dictionary();
        assert_eq!(summary(&dict.analyze("еж")), [("ёж", "ёж", 0)]);
        assert_eq!(summary(&dict.analyze("ЁЖ")), [("ёж", "ёж", 0)]);
        let words: Vec<_> = dict.analyze("все").iter().map(|a| a.word.clone()).collect();
        assert_eq!(words, ["все", "всё"]);
        assert_eq!(summary(&dict.analyze("всё")), [("всё", "всё", 0)]);
    }

    #[test]
    fn probabilities_order_analyses() {
        let dict = dictionary();
        let analyses = dict.analyze("Стали");
        assert_eq!(
            summary(&analyses),
            [
                ("стали", "стать", 2),
                ("стали", "сталь", 3),
                ("стали", "сталь", 4)
            ]
        );
        let probs: Vec<f64> = analyses.iter().map(|a| a.prob).collect();
        assert!((probs[0] - 0.97).abs() < 1e-12 && (probs[2] - 0.01).abs() < 1e-12);
        assert!((probs.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn uniform_without_probabilities() {
        let dict = dictionary();
        let probs: Vec<f64> = dict.analyze("все").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [0.5, 0.5]);
    }

    #[test]
    fn same_tag_shares_probability() {
        let mut builder = builder(&["NOUN", "VERB"], &[""], &["", "а", "о"]);
        let first = lexeme_table(&mut builder, &[(0, 0, 0)]);
        let second = lexeme_table(&mut builder, &[(1, 1, 0), (0, 0, 0)]);
        let third = lexeme_table(&mut builder, &[(2, 1, 0), (0, 0, 0)]);
        builder
            .add_word("лук", &[entry(first, 0), entry(second, 1), entry(third, 1)])
            .unwrap();
        builder.add_probability("лук", 0, 600_000).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let probs: Vec<f64> = dict.analyze("лук").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [1.0 / 3.0; 3]);
    }

    #[test]
    fn same_ud_analyses_merge() {
        let mut builder = Builder::new(
            vec!["CONJ".into(), "ADVB".into()],
            vec![String::new()],
            vec![String::new()],
        );
        let plain = builder.add_ud_tag("ADV", "Degree=Pos", &[]).unwrap();
        let labelled = builder.add_ud_tag("ADV", "Degree=Pos", &["Dmns"]).unwrap();
        let conjunction = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        let conjunction = builder.add_table(conjunction, &[(0, plain)]).unwrap();
        let adverb = builder.add_paradigm(&[(0, 1, 0)]).unwrap();
        let adverb = builder.add_table(adverb, &[(0, labelled)]).unwrap();
        builder
            .add_word("так", &[entry(conjunction, 0), entry(adverb, 0)])
            .unwrap();
        builder.add_probability("так", 1, 750_000).unwrap();
        builder.add_probability("так", 0, 250_000).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let analyses = dict.analyze("так");
        assert_eq!(analyses.len(), 1);
        assert_eq!(dict.ud_tags()[analyses[0].ud as usize].labels, ["Dmns"]);
        assert!((analyses[0].prob - 1.0).abs() < 1e-12);
        let probs: Vec<f64> = dict.opencorpora("так").iter().map(|a| a.3).collect();
        assert_eq!(probs, [0.75, 0.25]);
    }

    #[test]
    fn uniform_probabilities_after_merge() {
        let mut builder = Builder::new(
            vec!["ADJF,gent".into(), "ADJF,datv".into(), "NPRO".into()],
            vec![String::new()],
            vec![String::new()],
        );
        let abbreviation = builder.add_ud_tag("ADJ", "Abbr=Yes", &[]).unwrap();
        let pronoun = builder.add_ud_tag("PRON", "Case=Loc", &[]).unwrap();
        let adjective = builder.add_paradigm(&[(0, 0, 0), (0, 1, 0)]).unwrap();
        let adjective = builder
            .add_table(adjective, &[(0, abbreviation), (0, abbreviation)])
            .unwrap();
        let personal = builder.add_paradigm(&[(0, 2, 0)]).unwrap();
        let personal = builder.add_table(personal, &[(0, pronoun)]).unwrap();
        builder
            .add_word(
                "нем",
                &[entry(adjective, 0), entry(adjective, 1), entry(personal, 0)],
            )
            .unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let probs: Vec<f64> = dict.analyze("нем").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [0.5, 0.5]);
        let probs: Vec<f64> = dict.opencorpora("нем").iter().map(|a| a.3).collect();
        assert_eq!(probs, [1.0 / 3.0; 3]);
    }

    #[test]
    fn ud_tags() {
        let mut builder = Builder::new(
            vec!["NOUN".into()],
            vec![String::new()],
            vec![String::new()],
        );
        builder
            .add_ud_tag("NOUN", "Case=Gen|Number=Sing", &["Infr", "Slng"])
            .unwrap();
        builder.add_ud_tag("PART", "", &[]).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let noun = &dict.ud_tags()[0];
        assert_eq!(noun.pos, "NOUN");
        assert_eq!(noun.feats_string(), "Case=Gen|Number=Sing");
        assert_eq!(noun.labels, ["Infr", "Slng"]);
        assert!(dict.ud_tags()[1].feats.is_empty() && dict.ud_tags()[1].labels.is_empty());
    }

    #[test]
    fn tag_probability_shared_within_spelling() {
        let mut builder = builder(&["NOUN,datv", "NOUN,loct"], &[""], &[""]);
        let dative = lexeme_table(&mut builder, &[(0, 0, 0)]);
        let locative = lexeme_table(&mut builder, &[(0, 1, 0)]);
        builder.add_word("алеше", &[entry(dative, 0)]).unwrap();
        builder
            .add_word("алёше", &[entry(dative, 0), entry(locative, 0)])
            .unwrap();
        builder.add_probability("алёше", 0, 666_667).unwrap();
        builder.add_probability("алёше", 1, 333_333).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let summary: Vec<(String, u16, f64)> = dict
            .analyze("алеше")
            .into_iter()
            .map(|a| (a.word, a.tag.unwrap(), (a.prob * 1000.0).round() / 1000.0))
            .collect();
        assert_eq!(
            summary,
            [
                ("алёше".to_string(), 0, 0.667),
                ("алёше".to_string(), 1, 0.333),
                ("алеше".to_string(), 0, 0.0),
            ]
        );
    }

    #[test]
    fn probabilities_of_dictionary_spelling() {
        let mut builder = builder(&["ADVB", "PRCL"], &[""], &[""]);
        let adverb = lexeme_table(&mut builder, &[(0, 0, 0)]);
        let particle = lexeme_table(&mut builder, &[(0, 1, 0)]);
        builder
            .add_word("ещё", &[entry(adverb, 0), entry(particle, 0)])
            .unwrap();
        builder.add_probability("ещё", 1, 750_000).unwrap();
        builder.add_probability("ещё", 0, 250_000).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        for word in ["ещё", "еще"] {
            let analyses = dict.analyze(word);
            let tags: Vec<u16> = analyses.iter().map(|a| a.tag.unwrap()).collect();
            assert_eq!(tags, [1, 0], "{word}");
            assert!((analyses[0].prob - 0.75).abs() < 1e-12, "{word}");
        }
    }

    #[test]
    fn unknown_words() {
        let dict = dictionary();
        assert!(dict.analyze("вайб").is_empty());
        assert!(dict.analyze("").is_empty());
        assert!(dict.analyze("ежаа").is_empty());
        assert!(!dict.is_known("вайб"));
        assert!(dict.is_known("Еж"));
    }

    #[test]
    fn metadata() {
        let dict = dictionary();
        assert_eq!(dict.meta(), "тест");
        assert_eq!(dict.word_count(), 6);
        assert_eq!(dict.tags()[5], TAGS[5]);
        assert_eq!(dict.ud_tags()[5].feats_string(), "Tag=5");
    }

    fn sections() -> Sections {
        Sections {
            meta: String::new(),
            tags: vec!["NOUN".into()],
            ud_tags: vec!["NOUN\t\t".into()],
            prefixes: vec![String::new()],
            suffixes: vec![String::new()],
            paradigm_offsets: vec![0, 3],
            paradigms: vec![0, 0, 0],
            tables: vec![0, 0],
            words: fst::MapBuilder::memory().into_inner().unwrap(),
            records: vec![],
            probabilities: fst::MapBuilder::memory().into_inner().unwrap(),
            probability_records: vec![],
            guesses: fst::MapBuilder::memory().into_inner().unwrap(),
            guess_records: vec![],
        }
    }

    #[test]
    fn rejects_broken_paradigms_and_ud_tags() {
        assert!(Dictionary::from_bytes(&format::write(&sections())).is_ok());
        let broken: [fn(&mut Sections); 12] = [
            |s| s.paradigms = vec![0, 1, 0],
            |s| s.paradigms = vec![0, 0, 1],
            |s| s.paradigm_offsets = vec![0, 2],
            |s| s.paradigm_offsets = vec![1, 3],
            |s| s.paradigm_offsets = vec![0, 9, 3],
            |s| s.ud_tags = vec!["NOUN".into()],
            |s| s.ud_tags = vec!["\tCase=Gen\t".into()],
            |s| s.ud_tags = vec!["NOUN\tCase\t".into()],
            |s| s.tables = vec![1, 0],
            |s| s.tables = vec![0],
            |s| s.tables = vec![0, 0x0001_0000],
            |s| s.tables = vec![0, 1],
        ];
        for breaker in broken {
            let mut sections = sections();
            breaker(&mut sections);
            let result = Dictionary::from_bytes(&format::write(&sections));
            assert!(matches!(result, Err(Error::Format(_))));
        }
    }

    #[test]
    fn broken_records_give_no_analyses() {
        let mut builder = builder(&["NOUN"], &[""], &[""]);
        let table = lexeme_table(&mut builder, &[(0, 0, 0)]);
        builder.add_word("кот", &[entry(table, 0)]).unwrap();
        let data = builder.build().unwrap();
        for records in [vec![5, 0], vec![1, 7], vec![1, 0x0005_0000]] {
            let mut sections = format::read(&data).unwrap();
            sections.records = records;
            let dict = Dictionary::from_bytes(&format::write(&sections)).unwrap();
            assert!(dict.analyze("кот").is_empty());
        }
    }

    #[test]
    fn rejects_corrupted_automaton() {
        let mut sections = format::read(&dictionary_bytes()).unwrap();
        let middle = sections.words.len() / 2;
        sections.words[middle] ^= 1;
        let result = Dictionary::from_bytes(&format::write(&sections));
        assert!(matches!(result, Err(Error::Format(_))));
    }

    #[test]
    fn broken_probability_offset_gives_uniform_probabilities() {
        let mut sections = format::read(&dictionary_bytes()).unwrap();
        let mut probabilities = fst::MapBuilder::memory();
        probabilities.insert("стали", u64::MAX).unwrap();
        sections.probabilities = probabilities.into_inner().unwrap();
        let dict = Dictionary::from_bytes(&format::write(&sections)).unwrap();
        let probs: Vec<f64> = dict.analyze("стали").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [1.0 / 3.0; 3]);
    }
}
