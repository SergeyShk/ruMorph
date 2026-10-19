use std::collections::HashMap;
use std::path::Path;

use fst::Map;
use fst::raw::{Fst, Node, Output};

use crate::Error;
use crate::format::{self, Sections};

/// Откуда взят разбор
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Словоформа есть в словаре
    Dictionary,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Dictionary => "dictionary",
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
    /// Номер тега OpenCorpora в `Dictionary::tags`
    pub tag: u16,
    /// Вероятность разбора; сумма по всем разборам словоформы равна 1
    pub prob: f64,
    pub source: Source,
    /// Номер парадигмы и номер формы в ней
    pub paradigm: u16,
    pub form: u16,
}

/// Словарь словоформ
pub struct Dictionary {
    meta: String,
    tags: Vec<String>,
    ud_tags: Vec<UdTag>,
    /// Номер пары «часть речи, признаки» тега UD: разборы, которые отличаются только пометами, сливаются
    ud_core: Vec<u16>,
    prefixes: Vec<String>,
    suffixes: Vec<String>,
    paradigm_offsets: Vec<u32>,
    paradigms: Vec<u16>,
    words: Map<Vec<u8>>,
    records: Vec<u32>,
    probabilities: Map<Vec<u8>>,
    probability_records: Vec<u32>,
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
        let ud_tags = sections
            .ud_tags
            .iter()
            .map(|line| UdTag::parse(line))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Error::Format("неверный тег UD".into()))?;
        let mut cores: HashMap<(&str, String), u16> = HashMap::new();
        let ud_core = ud_tags
            .iter()
            .map(|tag| {
                let next = cores.len() as u16;
                *cores.entry((&tag.pos, tag.feats_string())).or_insert(next)
            })
            .collect();
        let fst_error = |err: fst::Error| Error::Format(err.to_string());
        let words = Map::new(sections.words).map_err(fst_error)?;
        let probabilities = Map::new(sections.probabilities).map_err(fst_error)?;
        // Поврежденный автомат грузится без ошибок, но паникует при обходе
        for map in [&words, &probabilities] {
            map.as_fst().verify().map_err(fst_error)?;
        }
        Ok(Dictionary {
            words,
            probabilities,
            meta: sections.meta,
            tags: sections.tags,
            ud_tags,
            ud_core,
            prefixes: sections.prefixes,
            suffixes: sections.suffixes,
            paradigm_offsets: sections.paradigm_offsets,
            paradigms: sections.paradigms,
            records: sections.records,
            probability_records: sections.probability_records,
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

    /// Число словоформ в словаре
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// Разборы словоформы по убыванию вероятности; регистр не важен, «е» может обозначать «ё»
    ///
    /// Разборы OpenCorpora, которые совпали в написании, лемме, части речи и признаках UD,
    /// сливаются в один с суммой вероятностей и пометами более вероятного
    pub fn analyze(&self, word: &str) -> Vec<Analysis> {
        let lower = word.to_lowercase();
        let mut analyses = Vec::new();
        for (spelling, offset) in self.lookup(&lower) {
            for &[position, target] in self.records_at(offset).unwrap_or_default() {
                let (paradigm, form) = ((position >> 16) as u16, position as u16);
                let (lemma_form, ud) = ((target >> 16) as u16, target as u16);
                let (Some(lemma), Some(tag), true) = (
                    self.lemma(paradigm, form, lemma_form, &spelling),
                    self.tag(paradigm, form),
                    usize::from(ud) < self.ud_tags.len(),
                ) else {
                    continue;
                };
                analyses.push(Analysis {
                    word: spelling.clone(),
                    lemma,
                    ud,
                    tag,
                    prob: 0.0,
                    source: Source::Dictionary,
                    paradigm,
                    form,
                });
            }
        }
        self.assign_probabilities(&lower, &mut analyses);
        self.merge(analyses)
    }

    /// Разборы OpenCorpora до перевода в UD: написание, тег и начальная форма парадигмы
    pub fn opencorpora(&self, word: &str) -> Vec<(String, &str, String)> {
        let mut analyses = Vec::new();
        for (spelling, offset) in self.lookup(&word.to_lowercase()) {
            for &[position, _] in self.records_at(offset).unwrap_or_default() {
                let (paradigm, form) = ((position >> 16) as u16, position as u16);
                if let (Some(lemma), Some(tag)) = (
                    self.lemma(paradigm, form, 0, &spelling),
                    self.tag(paradigm, form),
                ) {
                    analyses.push((spelling.clone(), self.tags[tag as usize].as_str(), lemma));
                }
            }
        }
        analyses
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

    fn tag(&self, paradigm: u16, form: u16) -> Option<u16> {
        let forms = self.paradigm(paradigm)?;
        let count = forms.len() / 3;
        (usize::from(form) < count).then(|| forms[count + usize::from(form)])
    }

    /// Префикс и суффикс формы
    fn affixes(&self, forms: &[u16], form: usize) -> (&str, &str) {
        let count = forms.len() / 3;
        (
            &self.prefixes[forms[2 * count + form] as usize],
            &self.suffixes[forms[form] as usize],
        )
    }

    /// Лемма: основа словоформы с префиксом и суффиксом формы леммы
    fn lemma(&self, paradigm: u16, form: u16, lemma_form: u16, word: &str) -> Option<String> {
        let forms = self.paradigm(paradigm)?;
        let count = forms.len() / 3;
        if usize::from(form) >= count || usize::from(lemma_form) >= count {
            return None;
        }
        let (prefix, suffix) = self.affixes(forms, form.into());
        let stem = word.strip_prefix(prefix)?.strip_suffix(suffix)?;
        let (prefix, suffix) = self.affixes(forms, lemma_form.into());
        Some([prefix, stem, suffix].concat())
    }

    /// Пары «форма, цель» списка разборов
    fn records_at(&self, offset: u64) -> Option<&[[u32; 2]]> {
        let start = usize::try_from(offset).ok()?;
        let count = *self.records.get(start)? as usize;
        let end = count.checked_mul(2)?.checked_add(start)?.checked_add(1)?;
        let (pairs, rest) = self.records.get(start + 1..end)?.as_chunks::<2>();
        rest.is_empty().then_some(pairs)
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
                .find(|[known, _]| *known == u32::from(tag))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Builder, Entry};

    const TAGS: [&str; 6] = [
        "NOUN,anim,masc sing,nomn",
        "NOUN,anim,masc sing,gent",
        "VERB,perf,intr plur,past,indc",
        "NOUN,inan,femn sing,gent",
        "NOUN,inan,femn plur,nomn",
        "ADJF,Supr,Qual masc,sing,nomn",
    ];

    /// Разбор с формой леммы 0 и тегом UD `ud`
    fn entry(paradigm: u16, form: u16, ud: u16) -> Entry {
        Entry {
            paradigm,
            form,
            lemma_form: 0,
            ud,
        }
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
        let hedgehog = builder.add_paradigm(&[(1, 0, 0), (2, 1, 0)]).unwrap();
        let become_ = builder.add_paradigm(&[(3, 2, 0), (4, 2, 0)]).unwrap();
        let steel = builder
            .add_paradigm(&[(5, 3, 0), (6, 3, 0), (6, 4, 0)])
            .unwrap();
        let big = builder.add_paradigm(&[(7, 5, 0), (8, 5, 1)]).unwrap();
        let plain = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        builder.add_word("ёж", &[entry(hedgehog, 0, 0)]).unwrap();
        builder.add_word("ежа", &[entry(hedgehog, 1, 1)]).unwrap();
        builder
            .add_word(
                "стали",
                &[entry(steel, 1, 3), entry(steel, 2, 4), entry(become_, 1, 2)],
            )
            .unwrap();
        builder.add_word("наибольший", &[entry(big, 1, 5)]).unwrap();
        builder.add_word("все", &[entry(plain, 0, 0)]).unwrap();
        builder.add_word("всё", &[entry(plain, 0, 0)]).unwrap();
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
            .map(|a| (a.word.as_str(), a.lemma.as_str(), a.tag))
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
        let surname = builder
            .add_paradigm(&[(0, 0, 0), (1, 1, 0), (2, 1, 0)])
            .unwrap();
        let feminine = Entry {
            lemma_form: 1,
            ..entry(surname, 2, 1)
        };
        builder.add_word("петровой", &[feminine]).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        assert_eq!(dict.analyze("петровой")[0].lemma, "петрова");
        assert_eq!(
            dict.opencorpora("петровой"),
            [("петровой".to_string(), "NOUN,femn", "петров".to_string())]
        );
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
        let first = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        let second = builder.add_paradigm(&[(1, 1, 0), (0, 0, 0)]).unwrap();
        let third = builder.add_paradigm(&[(2, 1, 0), (0, 0, 0)]).unwrap();
        builder
            .add_word(
                "лук",
                &[entry(first, 0, 0), entry(second, 1, 0), entry(third, 1, 0)],
            )
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
        let adverb = builder.add_paradigm(&[(0, 1, 0)]).unwrap();
        builder
            .add_word(
                "так",
                &[entry(conjunction, 0, plain), entry(adverb, 0, labelled)],
            )
            .unwrap();
        builder.add_probability("так", 1, 750_000).unwrap();
        builder.add_probability("так", 0, 250_000).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let analyses = dict.analyze("так");
        assert_eq!(analyses.len(), 1);
        assert_eq!(dict.ud_tags()[analyses[0].ud as usize].labels, ["Dmns"]);
        assert!((analyses[0].prob - 1.0).abs() < 1e-12);
        assert_eq!(dict.opencorpora("так").len(), 2);
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
        let personal = builder.add_paradigm(&[(0, 2, 0)]).unwrap();
        builder
            .add_word(
                "нем",
                &[
                    entry(adjective, 0, abbreviation),
                    entry(adjective, 1, abbreviation),
                    entry(personal, 0, pronoun),
                ],
            )
            .unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let probs: Vec<f64> = dict.analyze("нем").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [0.5, 0.5]);
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
        let dative = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        let locative = builder.add_paradigm(&[(0, 1, 0)]).unwrap();
        builder.add_word("алеше", &[entry(dative, 0, 0)]).unwrap();
        builder
            .add_word("алёше", &[entry(dative, 0, 0), entry(locative, 0, 1)])
            .unwrap();
        builder.add_probability("алёше", 0, 666_667).unwrap();
        builder.add_probability("алёше", 1, 333_333).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let summary: Vec<(String, u16, f64)> = dict
            .analyze("алеше")
            .into_iter()
            .map(|a| (a.word, a.tag, (a.prob * 1000.0).round() / 1000.0))
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
        let adverb = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        let particle = builder.add_paradigm(&[(0, 1, 0)]).unwrap();
        builder
            .add_word("ещё", &[entry(adverb, 0, 0), entry(particle, 0, 1)])
            .unwrap();
        builder.add_probability("ещё", 1, 750_000).unwrap();
        builder.add_probability("ещё", 0, 250_000).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        for word in ["ещё", "еще"] {
            let analyses = dict.analyze(word);
            let tags: Vec<u16> = analyses.iter().map(|a| a.tag).collect();
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
            words: fst::MapBuilder::memory().into_inner().unwrap(),
            records: vec![],
            probabilities: fst::MapBuilder::memory().into_inner().unwrap(),
            probability_records: vec![],
        }
    }

    #[test]
    fn rejects_broken_paradigms_and_ud_tags() {
        assert!(Dictionary::from_bytes(&format::write(&sections())).is_ok());
        let broken: [fn(&mut Sections); 8] = [
            |s| s.paradigms = vec![0, 1, 0],
            |s| s.paradigms = vec![0, 0, 1],
            |s| s.paradigm_offsets = vec![0, 2],
            |s| s.paradigm_offsets = vec![1, 3],
            |s| s.paradigm_offsets = vec![0, 9, 3],
            |s| s.ud_tags = vec!["NOUN".into()],
            |s| s.ud_tags = vec!["\tCase=Gen\t".into()],
            |s| s.ud_tags = vec!["NOUN\tCase\t".into()],
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
        let paradigm = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        builder.add_word("кот", &[entry(paradigm, 0, 0)]).unwrap();
        let data = builder.build().unwrap();
        for records in [vec![5, 0, 0], vec![1, 0, 7], vec![1, 0, 0x0005_0000]] {
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
