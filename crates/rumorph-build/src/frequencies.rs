//! Доли разборов по счетчикам трибанков UD: доли словаря (частоты OpenCorpora, без них - равные)
//! сглаживаются числом разметок словоформы каждой парой «часть речи, лемма» в корпусе

use std::collections::{BTreeMap, HashMap};

use rumorph::{Analysis, Dictionary, Entry};

/// Вес долей словаря против счетчиков корпуса - как столько же разметок: подобран по
/// SynTagRus train (часть речи и лемма первого разбора)
const PRIOR_WEIGHT: f64 = 5.0;

/// Счетчики разметок словоформы: часть речи, лемма и число
pub type Counts = BTreeMap<String, Vec<(String, String, u32)>>;

/// Счетчики из `counts.tsv`: словоформа, часть речи, лемма, число
pub fn read(text: &str) -> Result<Counts, String> {
    let mut counts = Counts::new();
    for (number, line) in text.lines().enumerate() {
        let invalid = || format!("counts.tsv:{}: неверная строка {line:?}", number + 1);
        let fields: Vec<&str> = line.split('\t').collect();
        let [word, pos, lemma, count] = fields.as_slice() else {
            return Err(invalid());
        };
        let count = count.parse().map_err(|_| invalid())?;
        counts.entry((*word).to_owned()).or_default().push((
            (*pos).to_owned(),
            (*lemma).to_owned(),
            count,
        ));
    }
    Ok(counts)
}

/// Словоформа или лемма в записи счетчиков: нижний регистр, «ё» как «е»
fn normal(text: &str) -> String {
    text.to_lowercase().replace('ё', "е")
}

/// Доли форм таблиц в миллионных у словоформ, разборы которых счетчики различают: у словоформы
/// разборы хотя бы двух пар «часть речи, лемма» и разметки хотя бы одной из них. Список - для
/// словоформы через «е» и для каждого ее написания с «ё» в словаре
pub fn blend(dictionary: &Dictionary, counts: &Counts) -> Vec<(String, Vec<(Entry, u32)>)> {
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
            if let Some(list) = shares(dictionary, spelling, pairs) {
                lists.push((spelling.clone(), list));
            }
        }
    }
    lists
}

/// Доли форм таблиц словоформы `word` по счетчикам `pairs`; `None`, если счетчики разборы не
/// различают
fn shares(
    dictionary: &Dictionary,
    word: &str,
    pairs: &[(String, String, u32)],
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
                normal(&analysis.lemma),
            )
        })
        .collect();
    let count = |key: &(String, String)| {
        pairs
            .iter()
            .filter(|(pos, lemma, _)| (pos, lemma) == (&key.0, &key.1))
            .map(|&(_, _, count)| f64::from(count))
            .sum::<f64>()
    };
    let mut distinct: Vec<&(String, String)> = keys.iter().collect();
    distinct.sort();
    distinct.dedup();
    if distinct.len() < 2 || distinct.iter().all(|key| count(key) == 0.0) {
        return None;
    }
    let priors = priors(dictionary, &analyses);
    // Разметки, которые не совпали ни с одним разбором, тоже входят в итог: они ослабляют корпус
    let total: f64 = pairs.iter().map(|&(_, _, count)| f64::from(count)).sum();
    let mut mass: HashMap<&(String, String), (f64, usize)> = HashMap::new();
    for (key, prior) in keys.iter().zip(&priors) {
        let entry = mass.entry(key).or_default();
        entry.0 += prior;
        entry.1 += 1;
    }
    let mut shares: Vec<(Entry, f64)> = Vec::new();
    for ((analysis, key), prior) in analyses.iter().zip(&keys).zip(&priors) {
        let (key_mass, key_size) = mass[key];
        // Разметки пары делятся между ее разборами по долям словаря, без них - поровну
        let part = if key_mass > 0.0 {
            prior / key_mass
        } else {
            1.0 / key_size as f64
        };
        let share = (count(key) * part + PRIOR_WEIGHT * prior) / (total + PRIOR_WEIGHT);
        let Some(entry) = analysis.entry else {
            continue;
        };
        match shares.iter_mut().find(|(known, _)| *known == entry) {
            Some((_, known)) => *known += share,
            None => shares.push((entry, share)),
        }
    }
    Some(
        shares
            .into_iter()
            .map(|(entry, share)| (entry, (share * 1e6).round() as u32))
            .collect(),
    )
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
            "стали\tVERB\tстать\t9\nстали\tNOUN\tсталь\t1\nсталь\tNOUN\tсталь\t3\nт\tNOUN\tт\t5\n",
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
        assert!(read("стали\tVERB\tстать").is_err());
    }
}
