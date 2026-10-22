//! Доли разборов по счетчикам трибанков UD: доли словаря (частоты OpenCorpora, без них - равные)
//! сглаживаются числом разметок словоформы каждой парой «часть речи, лемма» в корпусе;
//! разметки имен собственных - с поправкой на регистр

use std::collections::{BTreeMap, HashMap};

use rumorph::{Analysis, Dictionary, Entry};

use crate::ud;

/// Вес долей словаря против счетчиков корпуса - как столько же разметок: подобран по
/// SynTagRus train (часть речи и лемма первого разбора)
const PRIOR_WEIGHT: f64 = 5.0;

/// Счетчики разметок словоформы: часть речи, лемма, число всего и с заглавной буквы
pub type Counts = BTreeMap<String, Vec<(String, String, u32, u32)>>;

/// Счетчики из `counts.tsv`: словоформа, часть речи, лемма, число всего и с заглавной буквы
pub fn read(text: &str) -> Result<Counts, String> {
    let mut counts = Counts::new();
    for (number, line) in text.lines().enumerate() {
        let invalid = || format!("counts.tsv:{}: неверная строка {line:?}", number + 1);
        let fields: Vec<&str> = line.split('\t').collect();
        let [word, pos, lemma, count, capitalized] = fields.as_slice() else {
            return Err(invalid());
        };
        let count: u32 = count.parse().map_err(|_| invalid())?;
        let capitalized: u32 = capitalized.parse().map_err(|_| invalid())?;
        if capitalized > count {
            return Err(invalid());
        }
        counts.entry((*word).to_owned()).or_default().push((
            (*pos).to_owned(),
            (*lemma).to_owned(),
            count,
            capitalized,
        ));
    }
    Ok(counts)
}

/// Запись без регистра и «ё»: словоформа в счетчиках, лемма при сравнении с точностью до «ё»
fn normal(text: &str) -> String {
    text.to_lowercase().replace('ё', "е")
}

/// Доли форм таблиц в миллионных у словоформ, разборы которых счетчики различают: у словоформы
/// разборы хотя бы двух пар «часть речи, лемма» и разметки хотя бы одной из них. Список - для
/// словоформы через «е» и для каждого ее написания с «ё» в словаре
pub fn blend(dictionary: &Dictionary, counts: &Counts) -> Vec<(String, Vec<(Entry, u32)>)> {
    let rates = capitalized_rates(counts);
    let mut lists = Vec::new();
    for (word, pairs) in counts {
        let mut spellings: Vec<String> = dictionary
            .candidate_analyses(word)
            .into_iter()
            .map(|analysis| analysis.word)
            .filter(|spelling| spelling.contains('ё'))
            .collect();
        spellings.sort();
        spellings.dedup();
        for spelling in std::iter::once(word).chain(&spellings) {
            if let Some(list) = shares(dictionary, spelling, pairs, &rates) {
                lists.push((spelling.clone(), list));
            }
        }
    }
    lists
}

/// Доля разметок слов с заглавной буквы у каждой части речи, кроме имен собственных, - чаще всего
/// это слово в начале предложения - и у всех таких разметок вместе (для остальных частей речи)
fn capitalized_rates(counts: &Counts) -> (HashMap<&str, f64>, f64) {
    let mut sums: HashMap<&str, (f64, f64)> = HashMap::new();
    for (pos, _, count, capitalized) in counts.values().flatten() {
        if pos != "PROPN" && pos != "PUNCT" {
            let sum = sums.entry(pos.as_str()).or_default();
            sum.0 += f64::from(*capitalized);
            sum.1 += f64::from(*count);
        }
    }
    let (capitalized, all) = (sums.values()).fold((0.0, 0.0), |(a, b), (c, d)| (a + c, b + d));
    let rates = (sums.into_iter())
        .filter(|(_, (_, all))| *all > 0.0)
        .map(|(pos, (capitalized, all))| (pos, capitalized / all))
        .collect();
    (rates, if all > 0.0 { capitalized / all } else { 0.0 })
}

/// Доли форм таблиц словоформы `word` по счетчикам `pairs` и долям слов с заглавной `rates`;
/// `None`, если счетчики разборы не различают
fn shares(
    dictionary: &Dictionary,
    word: &str,
    pairs: &[(String, String, u32, u32)],
    rates: &(HashMap<&str, f64>, f64),
) -> Option<Vec<(Entry, u32)>> {
    let analyses = dictionary.candidate_analyses(word);
    if analyses.is_empty() || analyses.iter().any(|analysis| analysis.entry.is_none()) {
        return None;
    }
    // Счетчики голого сокращения не говорят о сокращении с точкой: «т» в корпусе - буква, «т.» - «так»
    let abbreviation = |analysis: &Analysis| {
        dictionary
            .ud_tag(analysis.ud)
            .is_some_and(|tag| tag.feat("Abbr").is_some() || tag.has_label("Abbr"))
    };
    if analyses.iter().any(abbreviation) {
        return None;
    }
    let keys: Vec<(String, String)> = analyses
        .iter()
        .map(|analysis| {
            (
                dictionary
                    .ud_tag(analysis.ud)
                    .map_or(String::new(), |tag| tag.pos().to_owned()),
                analysis.lemma.to_lowercase(),
            )
        })
        .collect();
    let pairs: Vec<(String, String, f64)> = (pairs.iter())
        .map(|(pos, lemma, count, _)| (pos.clone(), lemma.clone(), f64::from(*count)))
        .collect();
    let counts = attributed(&keys, &pairs);
    let mut distinct: Vec<&(String, String)> = keys.iter().collect();
    distinct.sort();
    distinct.dedup();
    if distinct.len() < 2 || counts.values().all(|&count| count == 0.0) {
        return None;
    }
    let priors = priors(dictionary, &analyses);
    let mut mass: HashMap<&(String, String), (f64, usize)> = HashMap::new();
    for (key, prior) in keys.iter().zip(&priors) {
        let entry = mass.entry(key).or_default();
        entry.0 += prior;
        entry.1 += 1;
    }
    let mut rows: Vec<(Entry, f64, f64, &str, bool)> = Vec::new();
    for ((analysis, key), prior) in analyses.iter().zip(&keys).zip(&priors) {
        let (key_mass, key_size) = mass[key];
        // Разметки пары делятся между ее разборами по долям словаря, без них - поровну
        let part = if key_mass > 0.0 {
            prior / key_mass
        } else {
            1.0 / key_size as f64
        };
        let corpus = counts.get(key).copied().unwrap_or_default() * part;
        let Some(entry) = analysis.entry else {
            continue;
        };
        let copy = ud::PROPER_COPIES.iter().any(|&(lemma, _)| lemma == key.1);
        rows.push((entry, corpus, PRIOR_WEIGHT * prior, key.0.as_str(), copy));
    }
    // Имя собственное почти всегда пишется с заглавной, а другое слово - с долей `rates` своей
    // части речи, поэтому у слова с заглавной отношение имени собственного к остальным - как
    // к сумме их долей, умноженных на `rates`. Во время разбора доля имени собственного у слова
    // с заглавной умножается на `TITLECASE_PROPN`, у строчного - почти обнуляется: здесь разметки
    // имени собственного масштабируются под это отношение («Земли» - чаще PROPN, «земли» - NOUN).
    // Доля словаря масштабируется только у копий `ud::PROPER_COPIES`: у других имен собственных
    // («Любим», «Данна») она не говорит о частоте
    let (mut common, mut expected) = (0.0, 0.0);
    for &(_, corpus, prior, pos, _) in &rows {
        if pos != "PROPN" {
            common += corpus + prior;
            expected += (corpus + prior) * rates.0.get(pos).copied().unwrap_or(rates.1);
        }
    }
    let scale = if rows.iter().any(|row| row.3 == "PROPN") && expected > 0.0 {
        common / (Dictionary::TITLECASE_PROPN * expected)
    } else {
        1.0
    };
    let mut shares: Vec<(Entry, f64)> = Vec::new();
    for (entry, corpus, prior, pos, copy) in rows {
        let share = if pos == "PROPN" && copy {
            (corpus + prior) * scale
        } else if pos == "PROPN" {
            corpus * scale + prior
        } else {
            corpus + prior
        };
        match shares.iter_mut().find(|(known, _)| *known == entry) {
            Some((_, known)) => *known += share,
            None => shares.push((entry, share)),
        }
    }
    let total: f64 = shares.iter().map(|(_, share)| share).sum();
    // Ненулевая доля у каждого разбора: нулевой разбор при загрузке не отличить от разбора без частот
    Some(
        shares
            .into_iter()
            .map(|(entry, share)| (entry, (share / total * 1e6).round().max(1.0) as u32))
            .collect(),
    )
}

/// Разметки корпуса по парам «часть речи, лемма» разборов: лемма сравнивается точно, а если
/// такой пары у разборов нет - с точностью до «ё» (у формы «ещё» в корпусе бывает лемма «еще»)
fn attributed<'k>(
    keys: &'k [(String, String)],
    pairs: &[(String, String, f64)],
) -> HashMap<&'k (String, String), f64> {
    let mut counts: HashMap<&(String, String), f64> = HashMap::new();
    for (pos, lemma, count) in pairs {
        let exact: Vec<&(String, String)> = keys
            .iter()
            .filter(|key| (&key.0, &key.1) == (pos, lemma))
            .collect();
        let matched = if exact.is_empty() {
            let mut loose: Vec<&(String, String)> = keys
                .iter()
                .filter(|key| &key.0 == pos && normal(&key.1) == normal(lemma))
                .collect();
            loose.sort();
            loose.dedup();
            loose
        } else {
            exact[..1].to_vec()
        };
        for key in &matched {
            *counts.entry(key).or_default() += count / matched.len() as f64;
        }
    }
    counts
}

/// Доли разборов в словаре: из частот, а без них - как у разбора после слияния: равные доли
/// разборов с разными частью речи, признаками или леммой, поровну между слитыми
fn priors(dictionary: &Dictionary, analyses: &[Analysis]) -> Vec<f64> {
    if analyses.iter().any(|analysis| analysis.prob > 0.0) {
        return analyses.iter().map(|analysis| analysis.prob).collect();
    }
    let group = |analysis: &Analysis| {
        let tag = dictionary.ud_tag(analysis.ud);
        (
            tag.map(|tag| tag.pos().to_owned()),
            tag.map(|tag| tag.feats_string()),
            analysis.lemma.clone(),
            normal(&analysis.word),
        )
    };
    let groups: Vec<_> = analyses.iter().map(group).collect();
    let mut sizes: HashMap<&_, usize> = HashMap::new();
    for group in &groups {
        *sizes.entry(group).or_default() += 1;
    }
    let distinct = sizes.len() as f64;
    groups
        .iter()
        .map(|group| 1.0 / distinct / sizes[group] as f64)
        .collect()
}

#[cfg(test)]
mod tests {
    use rumorph::Builder;

    use super::*;

    /// «стали» - форма «стать» и «сталь», «т» - сокращение; без частот словаря
    fn dictionary() -> Dictionary {
        let tags = ["INFN", "VERB", "NOUN", "NOUN Abbr"]
            .map(String::from)
            .to_vec();
        let suffixes = ["", "ть", "ли", "ь", "и"].map(String::from).to_vec();
        let mut builder = Builder::new(tags, vec![String::new()], suffixes);
        let infinitive = builder.add_ud_tag("VERB", "VerbForm=Inf", &[]).unwrap();
        let past = builder.add_ud_tag("VERB", "Tense=Past", &[]).unwrap();
        let noun = builder.add_ud_tag("NOUN", "Case=Gen", &[]).unwrap();
        let abbreviation = builder.add_ud_tag("NOUN", "Abbr=Yes", &[]).unwrap();
        let verb_paradigm = builder.add_paradigm(&[(1, 0, 0), (2, 1, 0)]).unwrap();
        let verb = builder
            .add_table(verb_paradigm, &[(0, infinitive), (0, past)])
            .unwrap();
        let noun_paradigm = builder.add_paradigm(&[(3, 2, 0), (4, 2, 0)]).unwrap();
        let steel = builder
            .add_table(noun_paradigm, &[(0, noun), (0, noun)])
            .unwrap();
        let abbreviation_paradigm = builder.add_paradigm(&[(0, 3, 0)]).unwrap();
        let abbreviated = builder
            .add_table(abbreviation_paradigm, &[(0, abbreviation)])
            .unwrap();
        let entry = |table, form| Entry { table, form };
        builder.add_word("стать", &[entry(verb, 0)]).unwrap();
        builder.add_word("сталь", &[entry(steel, 0)]).unwrap();
        builder
            .add_word("стали", &[entry(verb, 1), entry(steel, 1)])
            .unwrap();
        builder.add_word("т", &[entry(abbreviated, 0)]).unwrap();
        Dictionary::from_bytes(&builder.build().unwrap()).unwrap()
    }

    #[test]
    fn blends_counts_with_dictionary_shares() {
        let dictionary = dictionary();
        let counts = read(
            "стали\tVERB\tстать\t9\t1\nстали\tNOUN\tсталь\t1\t0\nсталь\tNOUN\tсталь\t3\t0\nт\tNOUN\tт\t5\t0\n",
        )
        .unwrap();
        let lists = blend(&dictionary, &counts);
        // Только «стали»: у «сталь» один разбор, «т» - сокращение
        assert_eq!(lists.len(), 1);
        let (word, list) = &lists[0];
        assert_eq!(word, "стали");
        // Равные доли словаря (1/2) и счетчики 9 и 1 с весом словаря 5: (9 + 2,5) / 15 и (1 + 2,5) / 15
        let mut shares: Vec<u32> = list.iter().map(|&(_, millionths)| millionths).collect();
        shares.sort_unstable();
        assert_eq!(shares, [233_333, 766_667]);
        assert!(read("стали\tVERB\tстать\t9").is_err());
        assert!(read("стали\tVERB\tстать\t1\t2").is_err());
    }

    /// «земли» и «воли» - нарицательные и имена собственные, без частот словаря; «земля» - из
    /// `ud::PROPER_COPIES`
    fn proper_dictionary() -> Dictionary {
        let tags = vec!["NOUN".to_owned()];
        let suffixes = ["я", "и", "ей"].map(String::from).to_vec();
        let mut builder = Builder::new(tags, vec![String::new()], suffixes);
        let noun = builder.add_ud_tag("NOUN", "Case=Gen", &[]).unwrap();
        let proper = builder.add_ud_tag("PROPN", "Case=Gen", &[]).unwrap();
        let paradigm = builder
            .add_paradigm(&[(0, 0, 0), (1, 0, 0), (2, 0, 0)])
            .unwrap();
        let common = builder
            .add_table(paradigm, &[(0, noun), (0, noun), (0, noun)])
            .unwrap();
        let named = builder
            .add_table(paradigm, &[(0, proper), (0, proper), (0, proper)])
            .unwrap();
        let entry = |table, form| Entry { table, form };
        for stem in ["земл", "вол"] {
            for (suffix, form) in [("я", 0), ("и", 1), ("ей", 2)] {
                let entries = [entry(common, form), entry(named, form)];
                builder
                    .add_word(&format!("{stem}{suffix}"), &entries)
                    .unwrap();
            }
        }
        Dictionary::from_bytes(&builder.build().unwrap()).unwrap()
    }

    #[test]
    fn proper_noun_shares_by_capitalized_words() {
        let dictionary = proper_dictionary();
        // NOUN с заглавной - 20 из 452 разметок; «землей» и «волей» - только строчные NOUN
        let mut text = String::new();
        for lemma in ["земля", "воля"] {
            let stem = lemma.trim_end_matches('я');
            text += &format!("{stem}и\tNOUN\t{lemma}\t221\t10\n{stem}и\tPROPN\t{lemma}\t36\t36\n");
            text += &format!("{stem}ей\tNOUN\t{lemma}\t5\t0\n");
        }
        let counts = read(&text).unwrap();
        let lists: BTreeMap<String, Vec<(Entry, u32)>> =
            blend(&dictionary, &counts).into_iter().collect();
        let share = |word: &str, table: u16| {
            let list = &lists[word];
            let found = list.iter().find(|(entry, _)| entry.table == table).unwrap();
            f64::from(found.1) / 1e6
        };
        let (title, rate) = (Dictionary::TITLECASE_PROPN, 20.0 / 452.0);
        let ratio = |word: &str| share(word, 1) * title / share(word, 0);
        let close = |ratio: f64, expected: f64| (ratio / expected - 1.0).abs() < 1e-4;
        // С заглавной PROPN к NOUN - как доли без поправки (разметки и доли словаря 1/2 с весом 5),
        // где доля NOUN умножена на долю NOUN с заглавной: 38,5 / (223,5 × 20 / 452)
        assert!(close(ratio("земли"), 38.5 / (223.5 * rate)));
        assert!(close(ratio("землей"), 2.5 / (7.5 * rate)));
        // Не копия: масштабируются только разметки, доля словаря - с множителем во время разбора
        assert!(close(ratio("воли"), (36.0 / rate + 2.5 * title) / 223.5));
        assert!(close(ratio("волей"), 2.5 * title / 7.5));
    }

    #[test]
    fn matches_lemmas_up_to_yo_only_without_exact_pair() {
        let key = |pos: &str, lemma: &str| (pos.to_owned(), lemma.to_owned());
        let pair = |pos: &str, lemma: &str, count| (pos.to_owned(), lemma.to_owned(), count);
        // «все» и «всё» - разные леммы, а лемма «еще» корпуса - «ещё» словаря
        let keys = [key("PRON", "все"), key("PRON", "всё"), key("ADV", "ещё")];
        let counts = attributed(
            &keys,
            &[
                pair("PRON", "все", 445.0),
                pair("PRON", "всё", 1996.0),
                pair("ADV", "еще", 10.0),
            ],
        );
        let get = |key: &(String, String)| counts.get(key).copied();
        assert_eq!(
            (get(&keys[0]), get(&keys[1]), get(&keys[2])),
            (Some(445.0), Some(1996.0), Some(10.0))
        );
    }
}
