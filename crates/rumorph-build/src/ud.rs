//! Перевод тегов OpenCorpora в Universal Dependencies по соглашениям UD_Russian-SynTagRus

use std::collections::BTreeMap;

/// Часть речи, признаки и пометы UD одного разбора
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ud {
    pub pos: &'static str,
    pub feats: BTreeMap<&'static str, &'static str>,
    pub labels: Vec<String>,
}

/// Подчинительные союзы; остальные союзы - сочинительные
const SCONJ: &[&str] = &[
    "благо",
    "будто",
    "дабы",
    "едва",
    "ежели",
    "если",
    "ибо",
    "как",
    "когда",
    "коли",
    "коль",
    "нежели",
    "пока",
    "покамест",
    "покуда",
    "поскольку",
    "пусть",
    "раз",
    "сколько",
    "словно",
    "столько",
    "тем",
    "то",
    "точно",
    "хотя",
    "чем",
    "что",
    "чтоб",
    "чтобы",
];

/// Служебные слова, часть речи которых в UD не совпадает с частью речи OpenCorpora
const POS_EXCEPTIONS: &[(&str, &str, &str)] = &[
    ("ADJF", "данный", "ADJ"),
    ("ADJF", "один", "NUM"),
    ("ADVB", "вдоль", "ADP"),
    ("ADVB", "вне", "ADP"),
    ("ADVB", "возле", "ADP"),
    ("ADVB", "еле", "PART"),
    ("ADVB", "несколько", "NUM"),
    ("ADVB", "около", "ADP"),
    ("ADVB", "подобно", "ADP"),
    ("ADVB", "попросту", "PART"),
    ("ADVB", "посреди", "ADP"),
    ("ADVB", "после", "ADP"),
    ("ADVB", "против", "ADP"),
    ("ADVB", "просто", "PART"),
    ("ADVB", "путём", "ADP"),
    ("ADVB", "согласно", "ADP"),
    ("ADVB", "только", "PART"),
    ("ADVB", "уж", "PART"),
    ("ADVB", "что-то", "PRON"),
    ("CONJ", "безусловно", "ADV"),
    ("CONJ", "в-третьих", "ADV"),
    ("CONJ", "вероятно", "ADV"),
    ("CONJ", "видимо", "ADV"),
    ("CONJ", "видно", "ADV"),
    ("CONJ", "во-вторых", "ADV"),
    ("CONJ", "во-первых", "ADV"),
    ("CONJ", "возможно", "ADV"),
    ("CONJ", "вообще", "ADV"),
    ("CONJ", "впрочем", "ADV"),
    ("CONJ", "где", "ADV"),
    ("CONJ", "дескать", "PART"),
    ("CONJ", "действительно", "ADV"),
    ("CONJ", "естественно", "ADV"),
    ("CONJ", "же", "PART"),
    ("CONJ", "значит", "ADV"),
    ("CONJ", "итак", "ADV"),
    ("CONJ", "конечно", "ADV"),
    ("CONJ", "куда", "ADV"),
    ("CONJ", "ли", "PART"),
    ("CONJ", "лишь", "PART"),
    ("CONJ", "наверное", "ADV"),
    ("CONJ", "наверняка", "ADV"),
    ("CONJ", "наконец", "ADV"),
    ("CONJ", "наоборот", "ADV"),
    ("CONJ", "например", "ADV"),
    ("CONJ", "несомненно", "ADV"),
    ("CONJ", "однако", "ADV"),
    ("CONJ", "отчего", "ADV"),
    ("CONJ", "очевидно", "ADV"),
    ("CONJ", "по-видимому", "ADV"),
    ("CONJ", "по-моему", "ADV"),
    ("CONJ", "пожалуй", "ADV"),
    ("CONJ", "похоже", "ADV"),
    ("CONJ", "правда", "ADV"),
    ("CONJ", "разве", "PART"),
    ("CONJ", "разумеется", "ADV"),
    ("CONJ", "ровно", "ADV"),
    ("CONJ", "скорее", "ADV"),
    ("CONJ", "следовательно", "ADV"),
    ("CONJ", "случайно", "ADV"),
    ("CONJ", "собственно", "ADV"),
    ("CONJ", "соответственно", "ADV"),
    ("CONJ", "так", "ADV"),
    ("CONJ", "только", "PART"),
    ("CONJ", "фактически", "ADV"),
    ("CONJ", "якобы", "ADV"),
    ("INTJ", "есть", "VERB"),
    ("INTJ", "неужели", "PART"),
    ("INTJ", "ну", "PART"),
    ("INTJ", "однако", "ADV"),
    ("INTJ", "пожалуйста", "ADV"),
    ("INTJ", "разве", "PART"),
    ("INTJ", "спасибо", "NOUN"),
    ("INTJ", "увы", "ADV"),
    ("NPRO", "всякий", "DET"),
    ("NPRO", "любой", "DET"),
    ("NPRO", "некого", "VERB"),
    ("NPRO", "нечего", "VERB"),
    ("NUMR", "достаточно", "ADV"),
    ("NUMR", "мало", "ADV"),
    ("NUMR", "много", "ADV"),
    ("NUMR", "недостаточно", "ADV"),
    ("PRCL", "бы", "AUX"),
    ("PRCL", "всё", "PRON"),
    ("PRCL", "исключительно", "ADV"),
    ("PRCL", "конечно", "ADV"),
    ("PRCL", "никак", "ADV"),
    ("PRCL", "особенно", "ADV"),
    ("PRCL", "пожалуй", "ADV"),
    ("PRCL", "пожалуйста", "ADV"),
    ("PRCL", "правда", "ADV"),
    ("PRCL", "примерно", "ADV"),
    ("PRCL", "приблизительно", "ADV"),
    ("PRCL", "прямо", "ADV"),
    ("PRCL", "собственно", "ADV"),
    ("PRCL", "там", "ADV"),
    ("PRCL", "угодно", "ADV"),
    ("PRCL", "хоть", "ADV"),
    ("PRCL", "это", "PRON"),
    ("PRCL", "якобы", "ADV"),
    ("PRED", "беда", "NOUN"),
    ("PRED", "видно", "ADV"),
    ("PRED", "грех", "NOUN"),
    ("PRED", "плохо", "ADV"),
    ("PRED", "пора", "NOUN"),
    ("PRED", "след", "NOUN"),
    ("PRED", "стыдно", "ADV"),
    ("PRED", "угодно", "ADV"),
    ("PRED", "хорошо", "ADV"),
    ("PREP", "включая", "ADV"),
    ("PREP", "вперёд", "ADV"),
    ("PREP", "внизу", "ADV"),
    ("PREP", "вслед", "ADV"),
    ("PREP", "изнутри", "ADV"),
    ("PREP", "несмотря", "ADV"),
    ("PREP", "сверху", "ADV"),
    ("PREP", "снизу", "ADV"),
    ("PREP", "соответственно", "ADV"),
];

/// Глаголы с Voice=Mid без -ся
const MIDDLE_VERBS: &[&str] = &[
    "лечь",
    "пересесть",
    "подсесть",
    "присесть",
    "сесть",
    "стать",
];

/// Притяжательные местоимения (Poss=Yes)
const POSSESSIVE: &[&str] = &["ваш", "его", "её", "их", "мой", "наш", "свой", "твой"];

/// Отрицательные слова (Polarity=Neg)
const NEGATIVE: &[&str] = &["не", "нет", "ни"];

/// Наречия в сравнительной и превосходной степени, которые OpenCorpora не отмечает
const ADVERB_DEGREES: &[(&str, &str)] = &[
    ("более", "Cmp"),
    ("выше", "Cmp"),
    ("далее", "Cmp"),
    ("менее", "Cmp"),
    ("наиболее", "Sup"),
    ("наименее", "Sup"),
    ("ниже", "Cmp"),
    ("позднее", "Cmp"),
    ("ранее", "Cmp"),
    ("скорее", "Cmp"),
];

/// Супплетивные превосходные прилагательные: в SynTagRus Degree=Pos, а не Sup
const SUPPLETIVE_SUPERLATIVES: &[&str] = &[
    "высший",
    "худший",
    "лучший",
    "наибольший",
    "наименьший",
    "низший",
];

/// Частицы, которые в UD - местоимения среднего рода: «это», «всё»
const PARTICLE_PRONOUNS: &[&str] = &["всё", "это"];

/// Неодушевленные местоимения без «что» в лемме
const INANIMATE_PRONOUNS: &[&str] = &["всё", "сие", "то", "это"];

fn has(grammemes: &[&str], grammeme: &str) -> bool {
    grammemes.contains(&grammeme)
}

/// Часть речи UD по части речи OpenCorpora, граммемам и лемме
fn pos(oc: &str, grammemes: &[&str], lemma: &str) -> &'static str {
    if let Some(&(_, _, pos)) = POS_EXCEPTIONS
        .iter()
        .find(|&&(oc_pos, word, _)| oc_pos == oc && word == lemma)
    {
        return pos;
    }
    match oc {
        "NOUN"
            if ["Name", "Surn", "Patr", "Geox", "Orgn", "Trad"]
                .iter()
                .any(|g| has(grammemes, g)) =>
        {
            "PROPN"
        }
        "NOUN" => "NOUN",
        "ADJF" if has(grammemes, "Apro") => "DET",
        "ADJF" | "ADJS" | "COMP" => "ADJ",
        "VERB" | "INFN" | "PRTF" | "PRTS" | "GRND" if lemma == "быть" => "AUX",
        "VERB" | "INFN" | "PRTF" | "PRTS" | "GRND" => "VERB",
        "NUMR" => "NUM",
        "ADVB" => "ADV",
        "NPRO" => "PRON",
        "PRED" => "VERB",
        "PREP" => "ADP",
        "CONJ" if SCONJ.contains(&lemma) => "SCONJ",
        "CONJ" => "CCONJ",
        "PRCL" => "PART",
        "INTJ" => "INTJ",
        _ => "X",
    }
}

/// Граммемы, которые переходят в признаки UD (остальные лексические граммемы становятся пометами)
const CONSUMED: &[&str] = &[
    "anim", "inan", "masc", "femn", "neut", "ms-f", "GNdr", "sing", "plur", "nomn", "gent", "datv",
    "accs", "ablt", "loct", "voct", "gen1", "gen2", "acc2", "loc1", "loc2", "perf", "impf", "1per",
    "2per", "3per", "pres", "past", "futr", "indc", "impr", "actv", "pssv", "incl", "excl", "Name",
    "Surn", "Patr", "Geox", "Orgn", "Trad", "Abbr", "Poss", "Supr", "Anum", "Apro",
];

/// Перевод тега OpenCorpora и леммы разбора в часть речи, признаки и пометы UD
pub fn convert(tag: &str, lemma: &str) -> Ud {
    let grammemes: Vec<&str> = tag.split([',', ' ']).filter(|g| !g.is_empty()).collect();
    let oc = grammemes.first().copied().unwrap_or("");
    let pos = pos(oc, &grammemes, lemma);
    let g = |name: &str| has(&grammemes, name);
    let mut feats = BTreeMap::new();
    let verbal = matches!(oc, "VERB" | "INFN" | "PRTF" | "PRTS" | "GRND");

    for (grammeme, value) in [
        ("nomn", "Nom"),
        ("gent", "Gen"),
        ("gen1", "Gen"),
        ("gen2", "Par"),
        ("datv", "Dat"),
        ("accs", "Acc"),
        ("acc2", "Acc"),
        ("ablt", "Ins"),
        ("loct", "Loc"),
        ("loc1", "Loc"),
        ("loc2", "Loc"),
        ("voct", "Voc"),
    ] {
        if g(grammeme) {
            feats.insert("Case", value);
        }
    }
    for (grammeme, name, value) in [
        ("sing", "Number", "Sing"),
        ("plur", "Number", "Plur"),
        ("masc", "Gender", "Masc"),
        ("ms-f", "Gender", "Masc"),
        ("femn", "Gender", "Fem"),
        ("neut", "Gender", "Neut"),
        ("anim", "Animacy", "Anim"),
        ("inan", "Animacy", "Inan"),
        ("1per", "Person", "1"),
        ("2per", "Person", "2"),
        ("3per", "Person", "3"),
        ("pres", "Tense", "Pres"),
        ("past", "Tense", "Past"),
        ("futr", "Tense", "Fut"),
        ("perf", "Aspect", "Perf"),
        ("impf", "Aspect", "Imp"),
        ("indc", "Mood", "Ind"),
        ("impr", "Mood", "Imp"),
        ("Name", "NameType", "Giv"),
        ("Surn", "NameType", "Sur"),
        ("Patr", "NameType", "Pat"),
        ("Geox", "NameType", "Geo"),
        ("Orgn", "NameType", "Com"),
        ("Trad", "NameType", "Pro"),
        ("Abbr", "Abbr", "Yes"),
        ("Poss", "Poss", "Yes"),
    ] {
        if g(grammeme) {
            feats.insert(name, value);
        }
    }
    if g("impr") && !feats.contains_key("Person") {
        feats.insert("Person", if g("incl") { "1" } else { "2" });
    }
    if verbal {
        let form = match oc {
            "VERB" => "Fin",
            "INFN" => "Inf",
            "GRND" => "Conv",
            _ => "Part",
        };
        feats.insert("VerbForm", form);
        let voice = if g("pssv") {
            "Pass"
        } else if lemma.ends_with("ся") || lemma.ends_with("сь") || MIDDLE_VERBS.contains(&lemma)
        {
            "Mid"
        } else {
            "Act"
        };
        feats.insert("Voice", voice);
        if lemma == "быть" {
            feats.remove("Aspect");
        }
    }
    if matches!(oc, "ADJS" | "PRTS") {
        feats.insert("Variant", "Short");
    }
    match (oc, pos) {
        ("COMP", _) => {
            feats.insert("Degree", "Cmp");
        }
        (_, "ADJ") if g("Supr") => {
            feats.insert("Degree", "Sup");
        }
        (_, "ADJ" | "ADV") => {
            feats.insert("Degree", "Pos");
        }
        _ => {}
    }
    if pos == "NUM" {
        feats.insert("NumType", "Card");
    }
    if POSSESSIVE.contains(&lemma) && pos == "DET" {
        feats.insert("Poss", "Yes");
    }
    if NEGATIVE.contains(&lemma) {
        feats.insert("Polarity", "Neg");
    }
    if pos == "AUX" && lemma == "бы" {
        feats.insert("Mood", "Cnd");
    }
    if pos == "SCONJ" && (lemma == "чтобы" || lemma == "чтоб") {
        feats.insert("Mood", "Cnd");
    }
    if lemma == "быть" && feats.get("Tense") == Some(&"Fut") {
        // SynTagRus: «будет», «будут» - Tense=Pres
        feats.insert("Tense", "Pres");
    }
    if oc == "PRED" && pos == "VERB" {
        // можно, надо, нельзя - VERB без признаков
        feats.clear();
    }
    if oc == "PRCL" && PARTICLE_PRONOUNS.contains(&lemma) {
        feats.extend([
            ("Animacy", "Inan"),
            ("Case", "Nom"),
            ("Gender", "Neut"),
            ("Number", "Sing"),
        ]);
    }
    if oc == "NPRO" {
        if lemma.contains("что") || INANIMATE_PRONOUNS.contains(&lemma) {
            feats.insert("Animacy", "Inan");
            if lemma.contains('-') || lemma == "нечто" {
                feats.extend([("Gender", "Neut"), ("Number", "Sing")]);
            }
        } else if lemma.contains("кто") {
            feats.insert("Animacy", "Anim");
        }
        if lemma == "себя" {
            feats.remove("Number");
            feats.remove("Gender");
        }
    }
    if pos == "ADV"
        && let Some(&(_, degree)) = ADVERB_DEGREES.iter().find(|&&(word, _)| word == lemma)
    {
        feats.insert("Degree", degree);
    }
    if feats.get("Degree") == Some(&"Sup") && SUPPLETIVE_SUPERLATIVES.contains(&lemma) {
        feats.insert("Degree", "Pos");
    }
    if g("Abbr") && oc == "NOUN" {
        // Аббревиатуры (РФ, СССР) в SynTagRus без падежа, числа, рода и одушевленности
        for name in ["Case", "Number", "Gender", "Animacy"] {
            feats.remove(name);
        }
    }
    if g("Abbr") && g("Anum") {
        // Порядковые из цифр (90-х) - NumType=Ord, а не сокращения
        feats.remove("Abbr");
        feats.insert("NumType", "Ord");
    }

    let labels = grammemes
        .iter()
        .skip(1)
        .filter(|g| !CONSUMED.contains(g))
        .map(|g| g.to_string())
        .collect();
    Ud { pos, feats, labels }
}

/// Форма парадигмы, которая служит леммой разбора вместо формы 0
#[derive(Debug, PartialEq, Eq)]
pub enum LemmaForm {
    /// Первая форма парадигмы со всеми этими граммемами
    With(&'static [&'static str]),
    /// Сама словоформа: «об» - лемма «об», а не «о»
    Own,
}

/// Форма леммы разбора по соглашениям UD; `None` - форма 0 парадигмы
pub fn lemma_form(tag: &str, lemma: &str, word: &str) -> Option<LemmaForm> {
    let grammemes: Vec<&str> = tag.split([',', ' ']).collect();
    let g = |name: &str| grammemes.contains(&name);
    match grammemes.first().copied() {
        Some("PREP") if word == "об" || word == "обо" => Some(LemmaForm::Own),
        // Деньги - pluralia tantum в SynTagRus, в OpenCorpora - лексема «деньга»
        Some("NOUN") if lemma == "деньга" => Some(LemmaForm::With(&["NOUN", "plur", "nomn"])),
        // Женские фамилии и отчества - в женском роде: Петрова, Васильевна
        Some("NOUN") if g("femn") && (g("Surn") || g("Patr")) => {
            Some(LemmaForm::With(&["NOUN", "femn", "sing", "nomn"]))
        }
        // Превосходная степень остается превосходной: наибольший
        Some("ADJF") if g("Supr") => {
            Some(LemmaForm::With(&["ADJF", "Supr", "masc", "sing", "nomn"]))
        }
        Some("ADJS") if lemma == "должный" => {
            Some(LemmaForm::With(&["ADJS", "masc", "sing"]))
        }
        _ => None,
    }
}

/// Часть леммы, от которой зависит перевод: сама лемма для слов из списков, «ся» для возвратных
/// глаголов, иначе пустая строка; разборы с одним тегом и классом леммы переводятся одинаково
pub fn lemma_class(lemma: &str) -> &str {
    let listed = SCONJ.contains(&lemma)
        || POS_EXCEPTIONS.iter().any(|&(_, word, _)| word == lemma)
        || ADVERB_DEGREES.iter().any(|&(word, _)| word == lemma)
        || [
            MIDDLE_VERBS,
            POSSESSIVE,
            NEGATIVE,
            SUPPLETIVE_SUPERLATIVES,
            PARTICLE_PRONOUNS,
            INANIMATE_PRONOUNS,
            &["быть", "бы", "должный", "себя"],
        ]
        .iter()
        .any(|list| list.contains(&lemma))
        || lemma.contains("что")
        || lemma.contains("кто");
    if listed {
        lemma
    } else if lemma.ends_with("ся") || lemma.ends_with("сь") {
        "ся"
    } else {
        ""
    }
}

/// Признаки UD в записи CoNLL-U: «Case=Gen|Number=Sing»
pub fn feats_string(feats: &BTreeMap<&str, &str>) -> String {
    feats
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(tag: &str, lemma: &str) -> (String, String, Vec<String>) {
        let ud = convert(tag, lemma);
        (ud.pos.to_string(), feats_string(&ud.feats), ud.labels)
    }

    #[test]
    fn nouns() {
        assert_eq!(
            summary("NOUN,inan,femn sing,gent", "сталь"),
            (
                "NOUN".into(),
                "Animacy=Inan|Case=Gen|Gender=Fem|Number=Sing".into(),
                vec![]
            )
        );
        assert_eq!(
            summary("NOUN,anim,femn,Sgtm,Surn sing,nomn", "петрова").0,
            "PROPN"
        );
        let (_, feats, labels) = summary("NOUN,inan,masc,Geox sing,loc2", "сад");
        assert_eq!(
            feats,
            "Animacy=Inan|Case=Loc|Gender=Masc|NameType=Geo|Number=Sing"
        );
        assert!(labels.is_empty());
        assert_eq!(
            summary("NOUN,anim,ms-f,Infr sing,gen2", "сирота").1,
            "Animacy=Anim|Case=Par|Gender=Masc|Number=Sing"
        );
    }

    #[test]
    fn verbs() {
        assert_eq!(
            summary("VERB,perf,intr plur,past,indc", "стать"),
            (
                "VERB".into(),
                "Aspect=Perf|Mood=Ind|Number=Plur|Tense=Past|VerbForm=Fin|Voice=Mid".into(),
                vec!["intr".into()]
            )
        );
        assert_eq!(
            summary("VERB,impf,intr sing,3per,futr,indc", "быть"),
            (
                "AUX".into(),
                "Mood=Ind|Number=Sing|Person=3|Tense=Pres|VerbForm=Fin|Voice=Act".into(),
                vec!["intr".into()]
            )
        );
        assert_eq!(
            summary("VERB,impf,tran sing,impr,excl", "мыть").1,
            "Aspect=Imp|Mood=Imp|Number=Sing|Person=2|VerbForm=Fin|Voice=Act"
        );
        assert_eq!(
            summary("PRTS,perf,past,pssv femn,sing", "сделать").1,
            "Aspect=Perf|Gender=Fem|Number=Sing|Tense=Past|Variant=Short|VerbForm=Part|Voice=Pass"
        );
        assert_eq!(
            summary("GRND,impf,intr pres", "смеяться").1,
            "Aspect=Imp|Tense=Pres|VerbForm=Conv|Voice=Mid"
        );
    }

    #[test]
    fn adjectives_and_determiners() {
        assert_eq!(summary("ADJF,Apro,Subx masc,sing,nomn", "который").0, "DET");
        assert_eq!(
            summary("ADJF,Apro,Anum masc,sing,nomn", "один").1,
            "Case=Nom|Gender=Masc|NumType=Card|Number=Sing"
        );
        assert_eq!(
            summary("ADJF,Apro,Fixd,Anph masc,sing,gent", "его"),
            (
                "DET".into(),
                "Case=Gen|Gender=Masc|Number=Sing|Poss=Yes".into(),
                vec!["Fixd".into(), "Anph".into()]
            )
        );
        assert_eq!(
            summary("ADJF,Supr,Qual masc,sing,nomn", "важный").1,
            "Case=Nom|Degree=Sup|Gender=Masc|Number=Sing"
        );
        assert_eq!(
            summary("ADJF,Supr,Qual masc,sing,nomn", "лучший").1,
            "Case=Nom|Degree=Pos|Gender=Masc|Number=Sing"
        );
        assert_eq!(summary("COMP,Qual", "большой").1, "Degree=Cmp");
        assert_eq!(
            summary("ADJS,Qual femn,sing", "нужный").1,
            "Degree=Pos|Gender=Fem|Number=Sing|Variant=Short"
        );
        assert_eq!(
            summary("ADJF,Anum masc,sing,nomn", "первый").1,
            "Case=Nom|Degree=Pos|Gender=Masc|Number=Sing"
        );
    }

    #[test]
    fn function_words() {
        assert_eq!(summary("CONJ", "что").0, "SCONJ");
        assert_eq!(summary("CONJ", "и").0, "CCONJ");
        assert_eq!(summary("CONJ,Prnt", "однако").0, "ADV");
        assert_eq!(summary("CONJ", "однако").1, "Degree=Pos");
        assert_eq!(
            summary("PRCL", "бы"),
            ("AUX".into(), "Mood=Cnd".into(), vec![])
        );
        assert_eq!(
            summary("PRCL", "не"),
            ("PART".into(), "Polarity=Neg".into(), vec![])
        );
        assert_eq!(
            summary("PRED,pres", "можно"),
            ("VERB".into(), "".into(), vec![])
        );
        assert_eq!(summary("CONJ", "чтобы").1, "Mood=Cnd");
        assert_eq!(
            summary("PRCL", "это"),
            (
                "PRON".into(),
                "Animacy=Inan|Case=Nom|Gender=Neut|Number=Sing".into(),
                vec![]
            )
        );
        assert_eq!(summary("ADVB", "более").1, "Degree=Cmp");
        assert_eq!(summary("ADVB", "наиболее").1, "Degree=Sup");
        assert_eq!(summary("PREP", "в").0, "ADP");
        assert_eq!(summary("NUMR,nomn", "много").0, "ADV");
        assert_eq!(summary("NUMR,nomn", "пять").1, "Case=Nom|NumType=Card");
        assert_eq!(
            summary("NPRO,1per sing,nomn", "я").1,
            "Case=Nom|Number=Sing|Person=1"
        );
        assert_eq!(
            summary("NPRO,neut sing,gent", "что").1,
            "Animacy=Inan|Case=Gen|Gender=Neut|Number=Sing"
        );
        assert_eq!(
            summary("NPRO,masc sing,nomn", "кто").1,
            "Animacy=Anim|Case=Nom|Gender=Masc|Number=Sing"
        );
        assert_eq!(summary("NPRO sing,datv", "себя").1, "Case=Dat");
        assert_eq!(
            summary("NPRO,neut sing,nomn", "это").1,
            "Animacy=Inan|Case=Nom|Gender=Neut|Number=Sing"
        );
        assert_eq!(
            summary("ADJF,Abbr,Anum plur,gent", "90-й").1,
            "Case=Gen|Degree=Pos|NumType=Ord|Number=Plur"
        );
        assert_eq!(
            summary("NOUN,inan,femn,Abbr,Fixd,Geox sing,gent", "рф").1,
            "Abbr=Yes|NameType=Geo"
        );
        assert_eq!(summary("ADVB", "быстро").1, "Degree=Pos");
    }

    #[test]
    fn labels_keep_lexical_grammemes() {
        let (_, _, labels) = summary("NOUN,inan,masc,Slng sing,nomn", "комп");
        assert_eq!(labels, ["Slng"]);
        let (_, _, labels) = summary("ADVB,Prdx,Infr", "чуток");
        assert_eq!(labels, ["Prdx", "Infr"]);
    }

    #[test]
    fn lemma_classes() {
        assert_eq!(lemma_class("быть"), "быть");
        assert_eq!(lemma_class("однако"), "однако");
        assert_eq!(lemma_class("смеяться"), "ся");
        assert_eq!(lemma_class("стол"), "");
        for (tag, lemma) in [
            ("VERB,impf,intr sing,3per,pres,indc", "смеяться"),
            ("CONJ", "однако"),
        ] {
            assert_eq!(convert(tag, lemma), convert(tag, lemma_class(lemma)));
        }
    }

    #[test]
    fn lemma_forms() {
        assert_eq!(
            lemma_form("NOUN,anim,femn,Sgtm,Surn sing,gent", "петров", "петровой"),
            Some(LemmaForm::With(&["NOUN", "femn", "sing", "nomn"]))
        );
        assert!(lemma_form("NOUN,anim,masc,Sgtm,Surn sing,gent", "петров", "петрова").is_none());
        assert_eq!(
            lemma_form("ADJF,Supr,Qual femn,sing,gent", "большой", "наибольшей"),
            Some(LemmaForm::With(&["ADJF", "Supr", "masc", "sing", "nomn"]))
        );
        assert!(lemma_form("ADJS,Qual masc,sing", "нужный", "нужен").is_none());
        assert!(lemma_form("ADJS femn,sing", "должный", "должна").is_some());
        assert_eq!(lemma_form("PREP", "о", "об"), Some(LemmaForm::Own));
        assert!(lemma_form("PREP", "в", "во").is_none());
        assert!(lemma_form("NOUN,inan,femn plur,gent", "деньга", "денег").is_some());
    }
}
