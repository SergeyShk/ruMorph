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

/// Разбор словоформы
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    /// Словоформа в написании словаря: с буквой «ё», если она есть в словаре
    pub word: String,
    /// Начальная форма
    pub lemma: String,
    /// Номер тега в таблице тегов словаря
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

    /// Таблица тегов: `Analysis::tag` - номер в ней
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Число словоформ в словаре
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// Разборы словоформы по убыванию вероятности; регистр не важен, «е» может обозначать «ё»
    pub fn analyze(&self, word: &str) -> Vec<Analysis> {
        let lower = word.to_lowercase();
        let mut analyses = Vec::new();
        for (spelling, offset) in self.lookup(&lower) {
            for &record in self.records_at(offset).unwrap_or_default() {
                let (paradigm, form) = ((record >> 16) as u16, record as u16);
                let (Some(lemma), Some(tag)) = (
                    self.lemma(paradigm, form, &spelling),
                    self.tag(paradigm, form),
                ) else {
                    continue;
                };
                analyses.push(Analysis {
                    word: spelling.clone(),
                    lemma,
                    tag,
                    prob: 0.0,
                    source: Source::Dictionary,
                    paradigm,
                    form,
                });
            }
        }
        self.assign_probabilities(&lower, &mut analyses);
        analyses
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

    /// Начальная форма: основа словоформы с префиксом и суффиксом формы 0
    fn lemma(&self, paradigm: u16, form: u16, word: &str) -> Option<String> {
        let forms = self.paradigm(paradigm)?;
        if usize::from(form) >= forms.len() / 3 {
            return None;
        }
        let (prefix, suffix) = self.affixes(forms, form.into());
        let stem = word.strip_prefix(prefix)?.strip_suffix(suffix)?;
        let (prefix, suffix) = self.affixes(forms, 0);
        Some([prefix, stem, suffix].concat())
    }

    fn records_at(&self, offset: u64) -> Option<&[u32]> {
        let start = usize::try_from(offset).ok()?;
        let count = *self.records.get(start)? as usize;
        self.records
            .get(start + 1..start.checked_add(count)?.checked_add(1)?)
    }

    fn probabilities_for(&self, word: &str) -> Option<&[u32]> {
        let start = usize::try_from(self.probabilities.get(word)?).ok()?;
        let count = *self.probability_records.get(start)? as usize;
        let end = count.checked_mul(2)?.checked_add(start)?.checked_add(1)?;
        self.probability_records.get(start + 1..end)
    }

    /// P(тег | словоформа) берется для написания из запроса, а без частот у него - для написания
    /// разбора в словаре («еще» - частоты «ещё»); вероятность тега делится поровну между разборами
    /// с этим тегом и той же таблицей частот и нормируется. Без частот разборы равновероятны
    /// и идут в порядке словаря
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
        } else {
            let prob = 1.0 / analyses.len() as f64;
            for analysis in analyses.iter_mut() {
                analysis.prob = prob;
            }
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
    use crate::Builder;

    const TAGS: [&str; 6] = [
        "NOUN,anim,masc sing,nomn",
        "NOUN,anim,masc sing,gent",
        "VERB,perf,intr plur,past,indc",
        "NOUN,inan,femn sing,gent",
        "NOUN,inan,femn plur,nomn",
        "ADJF,Supr,Qual masc,sing,nomn",
    ];

    /// ёж (ежа), стать (стали), сталь (стали), большой (наибольший), все и всё
    fn dictionary_bytes() -> Vec<u8> {
        let mut builder = Builder::new(
            TAGS.iter().map(|tag| tag.to_string()).collect(),
            vec![String::new(), "наи".into()],
            ["", "ёж", "ежа", "ть", "ли", "ь", "и", "ой", "ий"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        builder.meta("тест");
        let hedgehog = builder.add_paradigm(&[(1, 0, 0), (2, 1, 0)]).unwrap();
        let become_ = builder.add_paradigm(&[(3, 2, 0), (4, 2, 0)]).unwrap();
        let steel = builder
            .add_paradigm(&[(5, 3, 0), (6, 3, 0), (6, 4, 0)])
            .unwrap();
        let big = builder.add_paradigm(&[(7, 5, 0), (8, 5, 1)]).unwrap();
        let plain = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        builder.add_word("ёж", &[(hedgehog, 0)]).unwrap();
        builder.add_word("ежа", &[(hedgehog, 1)]).unwrap();
        builder
            .add_word("стали", &[(steel, 1), (steel, 2), (become_, 1)])
            .unwrap();
        builder.add_word("наибольший", &[(big, 1)]).unwrap();
        builder.add_word("все", &[(plain, 0)]).unwrap();
        builder.add_word("всё", &[(plain, 0)]).unwrap();
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
        let mut builder = Builder::new(
            vec!["NOUN".into(), "VERB".into()],
            vec![String::new()],
            vec!["".into(), "а".into(), "о".into()],
        );
        let first = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        let second = builder.add_paradigm(&[(1, 1, 0), (0, 0, 0)]).unwrap();
        let third = builder.add_paradigm(&[(2, 1, 0), (0, 0, 0)]).unwrap();
        builder
            .add_word("лук", &[(first, 0), (second, 1), (third, 1)])
            .unwrap();
        builder.add_probability("лук", 0, 600_000).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let probs: Vec<f64> = dict.analyze("лук").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [1.0 / 3.0; 3]);
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
    }

    #[test]
    fn rejects_broken_paradigms() {
        let base = || Sections {
            meta: String::new(),
            tags: vec!["NOUN".into()],
            prefixes: vec![String::new()],
            suffixes: vec![String::new()],
            paradigm_offsets: vec![0, 3],
            paradigms: vec![0, 0, 0],
            words: fst::MapBuilder::memory().into_inner().unwrap(),
            records: vec![],
            probabilities: fst::MapBuilder::memory().into_inner().unwrap(),
            probability_records: vec![],
        };
        assert!(Dictionary::from_bytes(&format::write(&base())).is_ok());
        let broken: [fn(&mut Sections); 5] = [
            |s| s.paradigms = vec![0, 1, 0],
            |s| s.paradigms = vec![0, 0, 1],
            |s| s.paradigm_offsets = vec![0, 2],
            |s| s.paradigm_offsets = vec![1, 3],
            |s| s.paradigm_offsets = vec![0, 9, 3],
        ];
        for breaker in broken {
            let mut sections = base();
            breaker(&mut sections);
            let result = Dictionary::from_bytes(&format::write(&sections));
            assert!(matches!(result, Err(Error::Format(_))));
        }
    }

    #[test]
    fn broken_records_give_no_analyses() {
        let mut builder = Builder::new(vec!["NOUN".into()], vec![String::new()], vec!["".into()]);
        let paradigm = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        builder.add_word("кот", &[(paradigm, 0)]).unwrap();
        let mut sections = format::read(&builder.build().unwrap()).unwrap();
        sections.records = vec![5, 0];
        let dict = Dictionary::from_bytes(&format::write(&sections)).unwrap();
        assert!(dict.analyze("кот").is_empty());
    }
    #[test]
    fn rejects_corrupted_automaton() {
        let data = dictionary_bytes();
        let mut sections = format::read(&data).unwrap();
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

    #[test]
    fn tag_probability_shared_within_spelling() {
        let mut builder = Builder::new(
            vec!["NOUN,datv".into(), "NOUN,loct".into()],
            vec![String::new()],
            vec!["".into()],
        );
        let dative = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        let locative = builder.add_paradigm(&[(0, 1, 0)]).unwrap();
        builder.add_word("алеше", &[(dative, 0)]).unwrap();
        builder
            .add_word("алёше", &[(dative, 0), (locative, 0)])
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
        let mut builder = Builder::new(
            vec!["ADVB".into(), "PRCL".into()],
            vec![String::new()],
            vec!["".into()],
        );
        let adverb = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        let particle = builder.add_paradigm(&[(0, 1, 0)]).unwrap();
        builder
            .add_word("ещё", &[(adverb, 0), (particle, 0)])
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
}
