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
    ("CONJ", "притом", "ADV"),
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
    // «выше», «повыше» в лексеме наречия «высоко» - наречия, а не прилагательные
    ("COMP", "высоко", "ADV"),
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

/// Признак UD и его значение
type Feature = (&'static str, &'static str);

/// Признаки слова из `POS_EXCEPTIONS`, с которыми его разбор совпадает с разбором другой части речи
/// OpenCorpora и сливается с ним: «несколько» (ADVB) - как числительное в винительном падеже
const EXCEPTION_FEATS: &[(&str, &str, &[Feature])] = &[
    ("ADVB", "несколько", &[("Animacy", "Inan"), ("Case", "Acc")]),
    (
        "ADVB",
        "что-то",
        &[
            ("Animacy", "Inan"),
            ("Case", "Nom"),
            ("Gender", "Neut"),
            ("Number", "Sing"),
        ],
    ),
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

/// Тип местоимения и местоименного наречия (PronType) по части речи UD и лемме
const PRONOUN_TYPES: &[(&str, &str, &str)] = &[
    ("ADV", "более", "Tot"),
    ("ADV", "больше", "Tot"),
    ("ADV", "везде", "Tot"),
    ("ADV", "вон", "Dem"),
    ("ADV", "всегда", "Tot"),
    ("ADV", "всего", "Tot"),
    ("ADV", "всюду", "Tot"),
    ("ADV", "всячески", "Tot"),
    ("ADV", "где", "Rel"),
    ("ADV", "где-нибудь", "Ind"),
    ("ADV", "где-то", "Ind"),
    ("ADV", "доселе", "Dem"),
    ("ADV", "затем", "Dem"),
    ("ADV", "зачем", "Int"),
    ("ADV", "зачем-то", "Ind"),
    ("ADV", "здесь", "Dem"),
    ("ADV", "иначе", "Tot"),
    ("ADV", "иногда", "Tot"),
    ("ADV", "как", "Rel"),
    ("ADV", "как-нибудь", "Ind"),
    ("ADV", "как-то", "Ind"),
    ("ADV", "когда", "Rel"),
    ("ADV", "когда-нибудь", "Ind"),
    ("ADV", "когда-то", "Ind"),
    ("ADV", "кое-где", "Ind"),
    ("ADV", "кое-как", "Ind"),
    ("ADV", "куда", "Rel"),
    ("ADV", "куда-нибудь", "Ind"),
    ("ADV", "куда-то", "Ind"),
    ("ADV", "мало", "Tot"),
    ("ADV", "менее", "Tot"),
    ("ADV", "много", "Tot"),
    ("ADV", "навсегда", "Tot"),
    ("ADV", "наиболее", "Tot"),
    ("ADV", "насколько", "Rel"),
    ("ADV", "настолько", "Dem"),
    ("ADV", "некогда", "Ind"),
    ("ADV", "немало", "Tot"),
    ("ADV", "немного", "Tot"),
    ("ADV", "немножко", "Tot"),
    ("ADV", "нигде", "Neg"),
    ("ADV", "никак", "Neg"),
    ("ADV", "никогда", "Neg"),
    ("ADV", "никуда", "Neg"),
    ("ADV", "нисколько", "Neg"),
    ("ADV", "ничего", "Neg"),
    ("ADV", "ныне", "Dem"),
    ("ADV", "откуда", "Rel"),
    ("ADV", "откуда-то", "Ind"),
    ("ADV", "отныне", "Dem"),
    ("ADV", "отовсюду", "Tot"),
    ("ADV", "отсюда", "Dem"),
    ("ADV", "оттого", "Dem"),
    ("ADV", "оттуда", "Dem"),
    ("ADV", "отчего", "Rel"),
    ("ADV", "по-другому", "Tot"),
    ("ADV", "повсюду", "Tot"),
    ("ADV", "потом", "Dem"),
    ("ADV", "потому", "Dem"),
    ("ADV", "почему", "Int"),
    ("ADV", "почему-то", "Ind"),
    ("ADV", "почто", "Int"),
    ("ADV", "поэтому", "Dem"),
    ("ADV", "притом", "Dem"),
    ("ADV", "сейчас", "Dem"),
    ("ADV", "сколь", "Rel"),
    ("ADV", "сколько", "Rel"),
    ("ADV", "сколько-нибудь", "Ind"),
    ("ADV", "столь", "Dem"),
    ("ADV", "столько", "Dem"),
    ("ADV", "сюда", "Dem"),
    ("ADV", "так", "Dem"),
    ("ADV", "там", "Dem"),
    ("ADV", "теперь", "Dem"),
    ("ADV", "тогда", "Dem"),
    ("ADV", "туда", "Dem"),
    ("ADV", "тут", "Dem"),
    ("ADV", "что", "Int"),
    ("DET", "ваш", "Prs"),
    ("DET", "весь", "Tot"),
    ("DET", "всякий", "Tot"),
    ("DET", "всяческий", "Tot"),
    ("DET", "другой", "Tot"),
    ("DET", "его", "Prs"),
    ("DET", "её", "Prs"),
    ("DET", "иной", "Tot"),
    ("DET", "их", "Prs"),
    ("DET", "каждый", "Tot"),
    ("DET", "каков", "Int"),
    ("DET", "какой", "Int,Rel"),
    ("DET", "какой-либо", "Ind"),
    ("DET", "какой-нибудь", "Ind"),
    ("DET", "какой-то", "Ind"),
    ("DET", "кое-какой", "Ind"),
    ("DET", "кой", "Neg"),
    ("DET", "который", "Rel"),
    ("DET", "любой", "Ind"),
    ("DET", "многий", "Tot"),
    ("DET", "мой", "Prs"),
    ("DET", "наш", "Prs"),
    ("DET", "некий", "Ind"),
    ("DET", "некоторый", "Ind"),
    ("DET", "никакой", "Neg"),
    ("DET", "никой", "Neg"),
    ("DET", "ничей", "Neg"),
    ("DET", "остальной", "Tot"),
    ("DET", "прочий", "Tot"),
    ("DET", "сам", "Emp"),
    ("DET", "самый", "Emp"),
    ("DET", "свой", "Prs"),
    ("DET", "сей", "Dem"),
    ("DET", "таков", "Dem"),
    ("DET", "таковой", "Dem"),
    ("DET", "такой", "Dem"),
    ("DET", "такой-то", "Dem"),
    ("DET", "твой", "Prs"),
    ("DET", "тот", "Dem"),
    ("DET", "чей", "Int,Rel"),
    ("DET", "чей-то", "Ind"),
    ("DET", "этакий", "Dem"),
    ("DET", "этот", "Dem"),
    ("PRON", "всё", "Tot"),
    ("PRON", "вы", "Prs"),
    ("PRON", "кое-кто", "Ind"),
    ("PRON", "кое-что", "Ind"),
    ("PRON", "кто", "Int,Rel"),
    ("PRON", "кто-либо", "Ind"),
    ("PRON", "кто-нибудь", "Ind"),
    ("PRON", "кто-то", "Ind"),
    ("PRON", "многие", "Tot"),
    ("PRON", "многое", "Tot"),
    ("PRON", "мы", "Prs"),
    ("PRON", "некто", "Ind"),
    ("PRON", "немногие", "Tot"),
    ("PRON", "немногое", "Tot"),
    ("PRON", "нечто", "Ind"),
    ("PRON", "никто", "Neg"),
    ("PRON", "ничто", "Neg"),
    ("PRON", "он", "Prs"),
    ("PRON", "она", "Prs"),
    ("PRON", "они", "Prs"),
    ("PRON", "оно", "Prs"),
    ("PRON", "прочее", "Ind"),
    ("PRON", "себя", "Prs"),
    ("PRON", "ты", "Prs"),
    ("PRON", "что", "Int,Rel"),
    ("PRON", "что-либо", "Ind"),
    ("PRON", "что-нибудь", "Ind"),
    ("PRON", "что-то", "Ind"),
    ("PRON", "это", "Dem"),
    ("PRON", "я", "Prs"),
];

/// Тип редких, устаревших и сокращенных местоимений, которых нет в SynTagRus, для PRON и DET
const RARE_PRONOUN_TYPES: &[(&str, &str)] = &[
    ("всяк", "Tot"),
    ("всякая", "Tot"),
    ("всякое", "Tot"),
    ("др", "Tot"),
    ("иже", "Rel"),
    ("ихний", "Prs"),
    ("к-рый", "Rel"),
    ("каковой", "Rel"),
    ("каковский", "Int"),
    ("любая", "Ind"),
    ("нек-рый", "Ind"),
    ("некоторые", "Ind"),
    ("немногий", "Tot"),
    ("никоторый", "Neg"),
    ("ничего", "Neg"),
    ("оное", "Dem"),
    ("оный", "Dem"),
    ("пр", "Tot"),
    ("проч", "Tot"),
    ("сие", "Dem"),
    ("сякой", "Dem"),
    ("таковский", "Dem"),
    ("тот-то", "Dem"),
    ("эдакий", "Dem"),
    ("экий", "Dem"),
    ("энный", "Ind"),
];

/// Тип местоимения по серии от вопросительной основы: «когда-либо», «кое-где» - Ind, «ниоткуда» - Neg
pub fn pronoun_series(lemma: &str) -> Option<&'static str> {
    const INTERROGATIVE: &[&str] = &[
        "где",
        "зачем",
        "как",
        "каков",
        "какой",
        "когда",
        "который",
        "кто",
        "куда",
        "насколько",
        "отчего",
        "откуда",
        "почему",
        "сколько",
        "чего",
        "что",
        "чей",
    ];
    let interrogative = |stem: &str| INTERROGATIVE.contains(&stem);
    let indefinite = ["-то", "-нибудь", "-либо"]
        .iter()
        .find_map(|suffix| lemma.strip_suffix(suffix))
        .or_else(|| lemma.strip_prefix("кое-"))
        .or_else(|| lemma.strip_prefix("кой-"));
    if indefinite.is_some_and(interrogative) {
        return Some("Ind");
    }
    lemma
        .strip_prefix("ни")
        .filter(|stem| interrogative(stem))
        .map(|_| "Neg")
}

/// Возвратные местоимения (Reflex=Yes)
const REFLEXIVE: &[&str] = &["свой", "себя"];

/// Притяжательные местоимения (Poss=Yes)
const POSSESSIVE: &[&str] = &[
    "ваш",
    "его",
    "её",
    "их",
    "ихний",
    "мой",
    "наш",
    "свой",
    "твой",
];

/// Отрицательные слова (Polarity=Neg)
const NEGATIVE: &[&str] = &["не", "нет", "ни"];

/// Отрицательные предикативы: Polarity=Neg у разбора VERB («нельзя», «нечего»), но не у наречия
/// («некогда» - «когда-то»)
const NEGATIVE_PREDICATIVES: &[&str] =
    &["негде", "некогда", "некого", "некуда", "нельзя", "нечего"];

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
    ("больше", "Cmp"),
];

/// Существительные, род которых в SynTagRus другой, чем в OpenCorpora: «евро» - мужского рода
const GENDER_EXCEPTIONS: &[(&str, &str)] =
    &[("евро", "Masc"), ("масс-медиа", "Neut"), ("медиа", "Neut")];

/// Нарицательные существительные OpenCorpora, которые с заглавной буквы - имена собственные:
/// небесные тела, единственные в своем роде место, учреждение и лицо, с типом имени, как
/// в SynTagRus. Их формы единственного числа получают и разбор PROPN, долю которого задают
/// частоты корпуса
pub const PROPER_COPIES: &[(&str, &str)] = &[
    ("вселенная", "Geo"),
    ("дума", "Com"),
    ("земля", "Geo"),
    ("кремль", "Geo"),
    ("луна", "Geo"),
    ("солнце", "Geo"),
    ("христос", "Prs"),
];

/// Супплетивные превосходные прилагательные: в SynTagRus Degree=Pos, а не Sup
const SUPPLETIVE_SUPERLATIVES: &[&str] = &[
    "высший",
    "худший",
    "лучший",
    "меньший",
    "наибольший",
    "наилучший",
    "наименьший",
    "низший",
];

/// Прилагательные без пометы Supr, которые в SynTagRus в превосходной степени
const SUPERLATIVES: &[&str] = &["дальнейший"];

/// Частицы, которые в UD - местоимения среднего рода: «всё»
const PARTICLE_PRONOUNS: &[&str] = &["всё"];

/// Неодушевленные местоимения без «что» в лемме
const INANIMATE_PRONOUNS: &[&str] = &["всё", "многое", "прочее", "сие", "то", "это"];

/// Одушевленные местоимения без «кто» в лемме
const ANIMATE_PRONOUNS: &[&str] = &["все", "многие"];

/// Имена собственные и притяжательность OpenCorpora: у прилагательных в SynTagRus таких
/// признаков нет, граммемы остаются пометами
const ADJECTIVE_LABELS: &[(&str, &str)] = &[
    ("Geox", "NameType"),
    ("Name", "NameType"),
    ("Orgn", "NameType"),
    ("Patr", "NameType"),
    ("Poss", "Poss"),
    ("Surn", "NameType"),
    ("Trad", "NameType"),
];

/// Тип местоимения, местоименного прилагательного или наречия с частью речи UD `pos`
fn pronoun_type(pos: &str, lemma: &str) -> Option<&'static str> {
    if !matches!(pos, "PRON" | "DET" | "ADV") {
        return None;
    }
    PRONOUN_TYPES
        .iter()
        .find(|&&(table_pos, word, _)| table_pos == pos && word == lemma)
        .map(|&(_, _, kind)| kind)
        .or_else(|| pronoun_series(lemma))
        .or_else(|| {
            RARE_PRONOUN_TYPES
                .iter()
                .find(|&&(word, _)| word == lemma && pos != "ADV")
                .map(|&(_, kind)| kind)
        })
}

fn has(grammemes: &[&str], grammeme: &str) -> bool {
    grammemes.contains(&grammeme)
}

/// Есть ли у слова с частью речи OpenCorpora `oc` своя часть речи UD в `POS_EXCEPTIONS`
pub fn has_pos_exception(oc: &str, lemma: &str) -> bool {
    POS_EXCEPTIONS
        .iter()
        .any(|&(oc_pos, word, _)| oc_pos == oc && word == lemma)
}

/// Часть речи UD по части речи OpenCorpora, граммемам и лемме; `true` - часть речи из `POS_EXCEPTIONS`
fn pos(oc: &str, grammemes: &[&str], lemma: &str) -> (&'static str, bool) {
    if let Some(&(_, _, pos)) = POS_EXCEPTIONS
        .iter()
        .find(|&&(oc_pos, word, _)| oc_pos == oc && word == lemma)
    {
        return (pos, true);
    }
    let pos = match oc {
        "NOUN"
            if ["Name", "Surn", "Patr", "Geox", "Orgn", "Trad"]
                .iter()
                .any(|g| has(grammemes, g)) =>
        {
            "PROPN"
        }
        "NOUN" => "NOUN",
        "ADJF" | "ADJS" if has(grammemes, "Apro") => "DET",
        "ADJF" | "ADJS" | "COMP" => "ADJ",
        "VERB" | "INFN" | "PRTF" | "PRTS" | "GRND" if lemma == "быть" => "AUX",
        "VERB" | "INFN" | "PRTF" | "PRTS" | "GRND" => "VERB",
        "NUMR" => "NUM",
        "ADVB" => "ADV",
        "NPRO" => "PRON",
        "PRED" => "VERB",
        "PREP" => "ADP",
        // Вводные слова, у которых нет другого разбора («в-четвёртых»), - наречия, как в SynTagRus
        "CONJ" if has(grammemes, "Prnt") => "ADV",
        "CONJ" if SCONJ.contains(&lemma) => "SCONJ",
        "CONJ" => "CCONJ",
        "PRCL" => "PART",
        "INTJ" => "INTJ",
        _ => "X",
    };
    (pos, false)
}

/// Части речи OpenCorpora: в пометы не попадают
const OPENCORPORA_POS: &[&str] = &[
    "NOUN", "ADJF", "ADJS", "COMP", "VERB", "INFN", "PRTF", "PRTS", "GRND", "NUMR", "ADVB", "NPRO",
    "PRED", "PREP", "CONJ", "PRCL", "INTJ",
];

/// Граммемы, которые переходят в признаки UD (остальные граммемы становятся пометами);
/// второй предложный («в лесу») - `Case=Loc` с пометой `loc2`, чтобы отличать его от «о лесе»
const CONSUMED: &[&str] = &[
    "anim", "inan", "masc", "femn", "neut", "ms-f", "GNdr", "sing", "plur", "nomn", "gent", "datv",
    "accs", "ablt", "loct", "voct", "gen1", "gen2", "acc2", "loc1", "perf", "impf", "1per", "2per",
    "3per", "pres", "past", "futr", "indc", "impr", "actv", "pssv", "Name", "Surn", "Patr", "Geox",
    "Orgn", "Trad", "Abbr", "Poss", "Supr", "Anum", "Apro",
];

/// Граммемы OpenCorpora, которые становятся пометами
const LABEL_GRAMMEMES: &[&str] = &[
    "Adjx", "Af-p", "Anph", "Arch", "Cmp2", "Coll", "Coun", "Dist", "Dmns", "Erro", "Fimp", "Fixd",
    "Hypo", "Impe", "Impx", "Infr", "Inmx", "Litr", "Ms-f", "Pltm", "Prdx", "Prnt", "Qual", "Ques",
    "Sgtm", "Slng", "Subx", "V-be", "V-bi", "V-ej", "V-en", "V-ey", "V-ie", "V-oy", "V-sh", "Vpre",
    "excl", "incl", "intr", "loc2", "tran",
];

/// Граммемы тега OpenCorpora, которых перевод не знает: неизвестная часть речи становится `X`,
/// неизвестная граммема - пометой
pub fn unknown_grammemes(tag: &str) -> Vec<&str> {
    tag.split([',', ' '])
        .filter(|g| !g.is_empty())
        .enumerate()
        .filter(|&(at, g)| {
            !OPENCORPORA_POS.contains(&g)
                && (at == 0 || !(CONSUMED.contains(&g) || LABEL_GRAMMEMES.contains(&g)))
        })
        .map(|(_, g)| g)
        .collect()
}

/// Перевод тега OpenCorpora и леммы разбора в часть речи, признаки и пометы UD
pub fn convert(tag: &str, lemma: &str) -> Ud {
    let grammemes: Vec<&str> = tag.split([',', ' ']).filter(|g| !g.is_empty()).collect();
    let oc = grammemes.first().copied().unwrap_or("");
    let (pos, exception) = pos(oc, &grammemes, lemma);
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
    if let Some(&(_, gender)) = GENDER_EXCEPTIONS.iter().find(|&&(word, _)| word == lemma) {
        feats.insert("Gender", gender);
    }
    let verbal_pos = matches!(oc, "VERB" | "INFN" | "PRTF" | "PRTS" | "GRND");
    let uninflected = matches!(pos, "ADV" | "PART" | "ADP" | "CCONJ" | "SCONJ" | "INTJ")
        || (pos == "VERB" && !verbal_pos);
    if oc == "PRED" || (exception && uninflected) {
        // Признаки OpenCorpora другой части речи не переходят: «плохо» - ADV без времени, «мало» - без падежа;
        // у «нечего», «нечем» (NPRO → VERB) падеж в SynTagRus остается, винительного у них нет:
        // «некого» - родительный
        let case = feats
            .get("Case")
            .copied()
            .filter(|_| oc == "NPRO")
            .map(|case| if case == "Acc" { "Gen" } else { case });
        feats.clear();
        feats.extend(case.map(|case| ("Case", case)));
    }
    if exception
        && let Some(&(_, _, extra)) = EXCEPTION_FEATS
            .iter()
            .find(|&&(oc_pos, word, _)| oc_pos == oc && word == lemma)
    {
        feats.extend(extra.iter().copied());
    }
    if g("impr") && !feats.contains_key("Person") {
        feats.insert("Person", if g("incl") { "1" } else { "2" });
    }
    // Безличный глагол («хочется») в OpenCorpora без лица, в UD - третье лицо
    if oc == "VERB" && g("Impe") && !g("past") {
        feats.insert("Person", "3");
    }
    // Род «двух», «двум» в OpenCorpora не указан, в UD он - по существительному: здесь мужской,
    // женский и средний - копии форм (`numeral_genders`)
    if lemma == "два" && !feats.contains_key("Gender") && feats.get("Case") != Some(&"Nom") {
        feats.insert("Gender", "Masc");
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
        (_, "ADJ") if g("Supr") || SUPERLATIVES.contains(&lemma) => {
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
    if NEGATIVE.contains(&lemma) || (pos == "VERB" && NEGATIVE_PREDICATIVES.contains(&lemma)) {
        feats.insert("Polarity", "Neg");
    }
    if pos == "AUX" && lemma == "бы" {
        feats.insert("Mood", "Cnd");
    }
    if pos == "SCONJ" && (lemma == "чтобы" || lemma == "чтоб") {
        feats.insert("Mood", "Cnd");
    }
    let mut extra_labels = Vec::new();
    if lemma == "быть" && feats.get("Tense") == Some(&"Fut") {
        // SynTagRus: «будет», «будут» - Tense=Pres; помета futr отличает их от «есть», «суть»
        feats.insert("Tense", "Pres");
        extra_labels.push("futr".to_string());
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
        } else if lemma.contains("кто") || ANIMATE_PRONOUNS.contains(&lemma) {
            feats.insert("Animacy", "Anim");
            if lemma.contains("кто") && pos == "PRON" {
                feats.entry("Gender").or_insert("Masc");
            }
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
    if g("Abbr") && g("Fixd") && oc == "NOUN" {
        // Несклоняемые аббревиатуры (РФ, СССР) в SynTagRus без падежа, числа, рода и одушевленности;
        // у склоняемых («вуз», «МИД») признаки полные
        for name in UNINFLECTED_ABBREVIATION {
            feats.remove(name);
        }
    }
    if g("Abbr") && g("Anum") {
        // Порядковые из цифр (90-х) - NumType=Ord, а не сокращения, и без степени
        feats.remove("Abbr");
        feats.remove("Degree");
        feats.insert("NumType", "Ord");
        feats.insert("NumForm", "Combi");
    }
    if pos == "NUM" {
        feats.insert("NumForm", "Word");
    }
    if let Some(kind) = pronoun_type(pos, lemma) {
        feats.insert("PronType", kind);
    }
    if pos == "DET" && g("Fixd") && POSSESSIVE.contains(&lemma) {
        // «его», «её», «их» в SynTagRus - Poss=Yes|PronType=Prs без падежа, рода и числа
        for name in ["Animacy", "Case", "Gender", "Number"] {
            feats.remove(name);
        }
    }
    if pos == "ADJ" {
        for &(grammeme, name) in ADJECTIVE_LABELS {
            if g(grammeme) && feats.remove(name).is_some() {
                extra_labels.push(grammeme.to_string());
            }
        }
    }
    if REFLEXIVE.contains(&lemma) && matches!(pos, "PRON" | "DET") {
        feats.insert("Reflex", "Yes");
    }
    if g("Fixd") && !g("Abbr") && matches!(pos, "NOUN" | "PROPN") {
        feats.insert("InflClass", "Ind");
    }

    let mut labels: Vec<String> = grammemes
        .iter()
        .skip(1)
        .filter(|g| !CONSUMED.contains(g) && !OPENCORPORA_POS.contains(g))
        .filter(|g| **g != "Fixd" || !feats.contains_key("InflClass"))
        .filter(|g| !matches!(**g, "Dmns" | "Ques") || !feats.contains_key("PronType"))
        .map(|g| g.to_string())
        .chain(extra_labels)
        .collect();
    labels.sort();
    labels.dedup();
    Ud { pos, feats, labels }
}

/// Местоимения-существительные, которых нет в OpenCorpora: «то», «всё» - формы среднего рода
/// единственного числа местоименных прилагательных «тот», «весь», «все» - формы множественного
/// числа «весь», кроме неодушевленного винительного. Вывод - лемма и разбор PRON формы с тегом
/// `tag` лексемы `base`, как в SynTagRus
pub fn substantive_pronoun(base: &str, tag: &str) -> Option<(&'static str, Ud)> {
    let grammemes: Vec<&str> = tag.split([',', ' ']).collect();
    let g = |name: &str| grammemes.contains(&name);
    if grammemes.first() != Some(&"ADJF") || !g("Apro") {
        return None;
    }
    let (lemma, kind) = match base {
        "тот" if g("neut") && g("sing") => ("то", "Dem"),
        "весь" if g("neut") && g("sing") => ("всё", "Tot"),
        "весь" if g("plur") && !g("inan") => ("все", "Tot"),
        _ => return None,
    };
    let mut ud = convert(tag, base);
    ud.pos = "PRON";
    ud.feats.insert("PronType", kind);
    if lemma == "все" {
        ud.feats.remove("Gender");
        ud.feats.extend([("Animacy", "Anim"), ("Number", "Plur")]);
    } else {
        ud.feats
            .extend([("Animacy", "Inan"), ("Gender", "Neut"), ("Number", "Sing")]);
    }
    Some((lemma, ud))
}

/// Разбор, который различает только контекст, - копия формы с тегом `tag` лексемы `base` с другой
/// частью речи: у «быть» - и VERB (бытийное «есть»), у «бы» - и PART, у «один» - и DET
/// («одна женщина»), у единственного числа «друг» - и взаимное местоимение («друг друга»)
pub fn context_copy(base: &str, tag: &str) -> Option<Ud> {
    if !matches!(base, "быть" | "бы" | "один" | "друг") {
        return None;
    }
    let mut ud = convert(tag, lemma_class(base));
    let case = ud.feats.get("Case").copied();
    match (base, ud.pos) {
        ("быть", "AUX") => ud.pos = "VERB",
        ("бы", "AUX") => {
            ud.pos = "PART";
            ud.feats.clear();
        }
        ("один", "NUM") => {
            ud.pos = "DET";
            ud.feats.remove("NumForm");
            ud.feats.remove("NumType");
            ud.feats.insert("PronType", "Ind");
        }
        ("друг", "NOUN")
            if ud.feats.get("Number") == Some(&"Sing")
                && matches!(case, Some("Nom" | "Gen" | "Dat" | "Acc" | "Ins" | "Loc")) =>
        {
            ud.pos = "PRON";
            ud.feats.retain(|&name, _| name == "Case");
            ud.feats.insert("PronType", "Rcp");
        }
        _ => return None,
    }
    Some(ud)
}

/// Копии формы «два» без рода в OpenCorpora («двух», «двум»): женский и средний род
pub fn numeral_genders(base: &str, tag: &str) -> Vec<Ud> {
    let genderless = !(tag.split([',', ' '])).any(|g| matches!(g, "masc" | "femn" | "neut"));
    if base != "два" || !genderless {
        return Vec::new();
    }
    let ud = convert(tag, lemma_class(base));
    if ud.feats.get("Gender") != Some(&"Masc") {
        return Vec::new();
    }
    ["Fem", "Neut"]
        .into_iter()
        .map(|gender| {
            let mut copy = ud.clone();
            copy.feats.insert("Gender", gender);
            copy
        })
        .collect()
}

/// Наречия, которые могут быть положительной степенью сравнительной формы прилагательного
/// `lemma` с краткими формами среднего рода `short`: сама краткая форма («хорошо» - «лучше»),
/// основа на «-о» («поздно» - «позже») и она же с одной «н» («рано» - «раньше»); у «меньше»
/// положительная степень другого корня - «мало», а «малее», «меньшее» «малого» - не наречия
pub fn positive_adverbs(lemma: &str, short: &[String]) -> Vec<String> {
    match lemma {
        "маленький" => return vec!["мало".into()],
        "малый" => return Vec::new(),
        _ => {}
    }
    let mut adverbs = short.to_vec();
    if let Some(stem) = ["ий", "ый", "ой"]
        .iter()
        .find_map(|ending| lemma.strip_suffix(ending))
    {
        adverbs.push(format!("{stem}о"));
        if let Some(single) = stem.strip_suffix("нн") {
            adverbs.push(format!("{single}но"));
        }
    }
    adverbs
}

/// Сравнительная степень наречия `adverb` - форма COMP прилагательного с тегом `tag`
/// и типом местоименного наречия («меньше» - `PronType=Tot`, как «мало»)
pub fn comparative_adverb(tag: &str, adverb: &str) -> Ud {
    let mut ud = convert(tag, lemma_class(adverb));
    ud.pos = "ADV";
    if let Some(kind) = pronoun_type(ud.pos, adverb) {
        ud.feats.insert("PronType", kind);
    }
    ud
}

/// Признаки, которых нет у несклоняемых аббревиатур и сокращений слов
const UNINFLECTED_ABBREVIATION: [&str; 4] = ["Animacy", "Case", "Gender", "Number"];

/// Сокращение слова - форма с пометой Abbr в лексеме несокращенного слова («им» в лексеме «имя»):
/// в SynTagRus, как несклоняемые аббревиатуры, без падежа, числа, рода и одушевленности
pub fn strip_abbreviation(ud: &mut Ud) {
    if ud.feats.contains_key("Abbr") {
        for name in UNINFLECTED_ABBREVIATION {
            ud.feats.remove(name);
        }
    }
}

/// Склоняемая аббревиатура («вуз», «МИД»): в SynTagRus у нее нет Abbr=Yes, он остается пометой
pub fn abbreviation_label(ud: &mut Ud) {
    if ud.feats.remove("Abbr").is_some() {
        ud.labels.push("Abbr".into());
        ud.labels.sort();
    }
}

/// Форма парадигмы, которая служит леммой разбора вместо формы 0
#[derive(Debug, PartialEq, Eq)]
pub enum LemmaForm {
    /// Форма парадигмы со всеми этими граммемами; из нескольких - ближайшая к словоформе по написанию
    /// («лучшей» - «лучший», а не «наихороший» той же парадигмы)
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
        // У мужских отчеств форма 0 бывает вариантом написания: «василиевич» вместо «васильевич»
        Some("NOUN") if g("masc") && g("Patr") => {
            Some(LemmaForm::With(&["NOUN", "masc", "sing", "nomn"]))
        }
        // Превосходная степень остается превосходной: наибольший
        Some("ADJF") if g("Supr") => {
            Some(LemmaForm::With(&["ADJF", "Supr", "masc", "sing", "nomn"]))
        }
        // Краткие местоименные прилагательные - леммы «таков», «каков», а не «таковой»
        Some("ADJS") if lemma == "должный" || g("Apro") => {
            Some(LemmaForm::With(&["ADJS", "masc", "sing"]))
        }
        _ => None,
    }
}

/// Счетные формы родительного падежа множественного числа без пометы `Coun` в OpenCorpora
const COUNTING_FORMS: &[(&str, &str)] = &[("год", "лет"), ("человек", "человек")];

/// Нужна ли форме помета `Coun`, которой нет в OpenCorpora: «5 лет», «5 человек»
pub fn counting_form(tag: &str, lemma: &str, word: &str) -> bool {
    let grammemes: Vec<&str> = tag.split([',', ' ']).collect();
    COUNTING_FORMS.contains(&(lemma, word))
        && grammemes.contains(&"plur")
        && grammemes.contains(&"gent")
        && !grammemes.contains(&"Coun")
}

/// Часть леммы, от которой зависит перевод: сама лемма для слов из списков, «ся» для возвратных
/// глаголов, иначе пустая строка; разборы с одним тегом и классом леммы переводятся одинаково
pub fn lemma_class(lemma: &str) -> &str {
    let listed = SCONJ.contains(&lemma)
        || GENDER_EXCEPTIONS.iter().any(|&(word, _)| word == lemma)
        || POS_EXCEPTIONS.iter().any(|&(_, word, _)| word == lemma)
        || ADVERB_DEGREES.iter().any(|&(word, _)| word == lemma)
        || PRONOUN_TYPES.iter().any(|&(_, word, _)| word == lemma)
        || pronoun_series(lemma).is_some()
        || RARE_PRONOUN_TYPES.iter().any(|&(word, _)| word == lemma)
        || [
            MIDDLE_VERBS,
            POSSESSIVE,
            NEGATIVE,
            NEGATIVE_PREDICATIVES,
            SUPPLETIVE_SUPERLATIVES,
            SUPERLATIVES,
            ANIMATE_PRONOUNS,
            PARTICLE_PRONOUNS,
            INANIMATE_PRONOUNS,
            REFLEXIVE,
            &["быть", "бы", "два", "должный", "себя"],
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

/// Род существительного, у которого есть только множественное число (Pltm) и нет рода в теге,
/// по склонению, как в SynTagRus: именительный на -а, -я - средний («ворота», «уста»),
/// родительный на -ов, -ев, -ей - мужской («выборов», «саней»; из двух форм родительного хватает
/// одной: «шорт» и «шортов»), иначе - женский («суток», «денег»). У несклоняемых («СМИ»)
/// и склоняемых как прилагательные («данные») рода нет. Формы - тег и написание
pub fn plural_only_gender(forms: &[(&str, String)]) -> Option<&'static str> {
    let find = |case: &'static str| {
        forms.iter().filter_map(move |(tag, word)| {
            let grammemes: Vec<&str> = tag.split([',', ' ']).collect();
            let genderless = grammemes.contains(&"Pltm") && grammemes.contains(&"GNdr");
            (genderless && grammemes.contains(&case)).then_some(word.as_str())
        })
    };
    let nominative = find("nomn").next()?;
    let genitives: Vec<&str> = find("gent").collect();
    let adjectival = |genitive: &&str| genitive.ends_with("ых") || genitive.ends_with("их");
    if genitives.is_empty() || genitives.contains(&nominative) || genitives.iter().any(adjectival) {
        return None;
    }
    Some(if nominative.ends_with(['а', 'я']) {
        "Neut"
    } else if genitives.iter().any(|genitive| {
        ["ов", "ев", "ёв", "ей"]
            .iter()
            .any(|ending| genitive.ends_with(ending))
    }) {
        "Masc"
    } else {
        "Fem"
    })
}

/// Признаки UD в записи CoNLL-U: «Case=Gen|Number=Sing», по алфавиту без учета регистра
pub fn feats_string(feats: &BTreeMap<&str, &str>) -> String {
    let mut feats: Vec<_> = feats.iter().collect();
    feats.sort_by_key(|(name, _)| name.to_ascii_lowercase());
    feats
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plural_only_gender_by_declension() {
        let forms = |nominative: &str, genitive: &str| {
            vec![
                ("NOUN,inan,GNdr,Pltm plur,nomn", nominative.to_owned()),
                ("NOUN,inan,GNdr,Pltm plur,gent", genitive.to_owned()),
            ]
        };
        for (nominative, genitive, gender) in [
            ("выборы", "выборов", "Masc"),
            ("сани", "саней", "Masc"),
            ("ворота", "ворот", "Neut"),
            ("сутки", "суток", "Fem"),
            ("деньги", "денег", "Fem"),
        ] {
            let found = plural_only_gender(&forms(nominative, genitive));
            assert_eq!(found, Some(gender), "{nominative}");
        }
        for (nominative, genitive) in [("сми", "сми"), ("данные", "данных")] {
            assert_eq!(plural_only_gender(&forms(nominative, genitive)), None);
        }
        // Из двух форм родительного мужской род дает любая на -ов, -ев, -ей
        for genitives in [["шорт", "шортов"], ["шортов", "шорт"]] {
            let mut two = forms("шорты", genitives[0]);
            two.push(("NOUN,inan,GNdr,Pltm plur,gent", genitives[1].to_owned()));
            assert_eq!(plural_only_gender(&two), Some("Masc"), "{genitives:?}");
        }
        let gendered = [
            ("NOUN,inan,femn,Pltm plur,nomn", "ворота".to_owned()),
            ("NOUN,inan,femn,Pltm plur,gent", "ворот".to_owned()),
        ];
        assert_eq!(plural_only_gender(&gendered), None);
        assert_eq!(
            plural_only_gender(&[("NOUN,inan,masc plur,nomn", "столы".to_owned())]),
            None
        );
    }

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
        assert_eq!(labels, ["loc2"]);
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
        assert_eq!(
            summary("VERB,impf,intr,Impe sing,pres,indc", "хотеться").1,
            "Aspect=Imp|Mood=Ind|Number=Sing|Person=3|Tense=Pres|VerbForm=Fin|Voice=Mid"
        );
        assert!(
            !summary("VERB,impf,intr,Impe neut,sing,past,indc", "хотеться")
                .1
                .contains("Person")
        );
    }

    #[test]
    fn adjectives_and_determiners() {
        assert_eq!(summary("ADJF,Apro,Subx masc,sing,nomn", "который").0, "DET");
        assert_eq!(
            summary("ADJF,Apro,Anum masc,sing,nomn", "один").1,
            "Case=Nom|Gender=Masc|Number=Sing|NumForm=Word|NumType=Card"
        );
        // «его» в SynTagRus без падежа, рода и числа: 24 разбора OpenCorpora сливаются в один
        assert_eq!(
            summary("ADJF,Apro,Fixd,Anph masc,sing,gent", "его"),
            (
                "DET".into(),
                "Poss=Yes|PronType=Prs".into(),
                vec!["Anph".into(), "Fixd".into()]
            )
        );
        assert_eq!(
            summary("ADJF,Apro,Fixd,Anph plur,datv", "их"),
            summary("ADJF,Apro,Fixd,Anph femn,sing,gent", "их")
        );
        assert_eq!(
            summary("ADJF,Geox,Qual masc,sing,gent", "украинский"),
            (
                "ADJ".into(),
                "Case=Gen|Degree=Pos|Gender=Masc|Number=Sing".into(),
                vec!["Geox".into(), "Qual".into()]
            )
        );
        assert_eq!(summary("ADJF,Poss plur,nomn", "казачий").2, ["Poss"]);
        assert_eq!(
            summary("ADJF masc,sing,nomn", "дальнейший").1,
            "Case=Nom|Degree=Sup|Gender=Masc|Number=Sing"
        );
        assert_eq!(
            summary("ADJF,Supr masc,sing,nomn", "наилучший").1,
            "Case=Nom|Degree=Pos|Gender=Masc|Number=Sing"
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
        assert_eq!(
            summary("CONJ,Prnt", "в-четвёртых"),
            ("ADV".into(), "Degree=Pos".into(), vec!["Prnt".into()])
        );
        assert_eq!(
            summary("COMP,Qual", "высоко"),
            ("ADV".into(), "Degree=Cmp".into(), vec!["Qual".into()])
        );
        assert_eq!(summary("ADVB", "больше").1, "Degree=Cmp|PronType=Tot");
        assert!(has_pos_exception("CONJ", "значит"));
        assert!(!has_pos_exception("CONJ", "кстати"));
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
        assert_eq!(
            summary("PRED,pres", "плохо"),
            ("ADV".into(), "Degree=Pos".into(), vec![])
        );
        assert_eq!(
            summary("PRED,pres", "пора"),
            ("NOUN".into(), "".into(), vec![])
        );
        assert_eq!(
            summary("PRED,pres", "нет"),
            ("VERB".into(), "Polarity=Neg".into(), vec![])
        );
        assert_eq!(
            summary("NUMR,gent", "мало"),
            ("ADV".into(), "Degree=Pos|PronType=Tot".into(), vec![])
        );
        assert_eq!(summary("NUMR,nomn", "много").0, "NUM");
        assert_eq!(summary("CONJ", "чтобы").1, "Mood=Cnd");
        // Частица «это» («это он сделал») остается частицей рядом с местоимением NPRO
        assert_eq!(summary("PRCL", "это"), ("PART".into(), "".into(), vec![]));
        assert_eq!(summary("ADVB", "более").1, "Degree=Cmp|PronType=Tot");
        assert_eq!(summary("ADVB", "наиболее").1, "Degree=Sup|PronType=Tot");
        assert_eq!(summary("PREP", "в").0, "ADP");
        assert_eq!(
            summary("NUMR,nomn", "пять").1,
            "Case=Nom|NumForm=Word|NumType=Card"
        );
        assert_eq!(
            summary("NPRO,1per sing,nomn", "я").1,
            "Case=Nom|Number=Sing|Person=1|PronType=Prs"
        );
        assert_eq!(
            summary("NPRO,neut sing,gent", "что").1,
            "Animacy=Inan|Case=Gen|Gender=Neut|Number=Sing|PronType=Int,Rel"
        );
        assert_eq!(
            summary("NPRO,masc sing,nomn", "кто").1,
            "Animacy=Anim|Case=Nom|Gender=Masc|Number=Sing|PronType=Int,Rel"
        );
        assert_eq!(
            summary("NPRO sing,datv", "себя").1,
            "Case=Dat|PronType=Prs|Reflex=Yes"
        );
        assert_eq!(
            summary("NPRO,neut sing,nomn", "это").1,
            "Animacy=Inan|Case=Nom|Gender=Neut|Number=Sing|PronType=Dem"
        );
        assert_eq!(
            summary("ADJF,Abbr,Anum plur,gent", "90-й").1,
            "Case=Gen|Number=Plur|NumForm=Combi|NumType=Ord"
        );
        assert_eq!(
            summary("NOUN,inan,femn,Abbr,Fixd,Geox sing,gent", "рф").1,
            "Abbr=Yes|NameType=Geo"
        );
        assert_eq!(summary("ADVB", "быстро").1, "Degree=Pos");
    }

    #[test]
    fn lexical_features() {
        let feats = |tag: &str, lemma: &str| summary(tag, lemma).1;
        assert_eq!(feats("ADVB", "там"), "Degree=Pos|PronType=Dem");
        assert_eq!(feats("ADVB", "никогда"), "Degree=Pos|PronType=Neg");
        assert_eq!(
            feats("ADJF,Apro masc,sing,nomn", "который"),
            "Case=Nom|Gender=Masc|Number=Sing|PronType=Rel"
        );
        assert_eq!(
            feats("ADJF,Apro masc,sing,nomn", "свой"),
            "Case=Nom|Gender=Masc|Number=Sing|Poss=Yes|PronType=Prs|Reflex=Yes"
        );
        assert_eq!(
            feats("NPRO sing,accs", "себя"),
            "Case=Acc|PronType=Prs|Reflex=Yes"
        );
        assert_eq!(
            feats("NUMR,nomn", "два"),
            "Case=Nom|NumForm=Word|NumType=Card"
        );
        assert_eq!(
            feats("ADJF,Abbr,Anum plur,gent", "90-й"),
            "Case=Gen|Number=Plur|NumForm=Combi|NumType=Ord"
        );
        let (_, kino, labels) = summary("NOUN,inan,neut,Fixd sing,gent", "кино");
        assert_eq!(
            kino,
            "Animacy=Inan|Case=Gen|Gender=Neut|InflClass=Ind|Number=Sing"
        );
        assert!(labels.is_empty());
        assert!(!feats("NOUN,inan,femn,Abbr,Fixd,Geox sing,gent", "рф").contains("InflClass"));
        assert_eq!(
            summary("ADJS,Apro masc,sing", "таков"),
            (
                "DET".into(),
                "Gender=Masc|Number=Sing|PronType=Dem|Variant=Short".into(),
                vec![]
            )
        );
        assert_eq!(feats("ADVB", "когда-либо"), "Degree=Pos|PronType=Ind");
        assert_eq!(feats("ADVB", "ниоткуда"), "Degree=Pos|PronType=Neg");
        assert!(feats("NPRO,masc sing,nomn", "кое-кто").contains("PronType=Ind"));
        assert_eq!(feats("CONJ", "притом"), "Degree=Pos|PronType=Dem");
        assert_eq!(
            feats("ADJF,Apro masc,sing,nomn", "оный"),
            "Case=Nom|Gender=Masc|Number=Sing|PronType=Dem"
        );
        assert!(feats("ADJF,Apro masc,sing,nomn", "ихний").contains("Poss=Yes|PronType=Prs"));
        assert_eq!(summary("ADVB,Dmns", "там").2, Vec::<String>::new());
        assert_eq!(summary("ADVB,Ques", "почему").2, Vec::<String>::new());
        assert_eq!(pronoun_series("ниже"), None);
        for emphatic in ["тут-то", "тогда-то", "так-то", "всего-то"] {
            assert_eq!(pronoun_series(emphatic), None, "{emphatic}");
        }
        assert_eq!(pronoun_series("кое-как"), Some("Ind"));
        assert_eq!(
            feats("ADJF,Apro masc,sing,nomn", "тот-то"),
            "Case=Nom|Gender=Masc|Number=Sing|PronType=Dem"
        );
        assert_eq!(
            summary("VERB,perf,intr plur,impr,incl", "пойти"),
            (
                "VERB".into(),
                "Aspect=Perf|Mood=Imp|Number=Plur|Person=1|VerbForm=Fin|Voice=Act".into(),
                vec!["incl".into(), "intr".into()]
            )
        );
    }

    #[test]
    fn future_of_be() {
        // SynTagRus: «будет» - Tense=Pres; помета futr отличает его от «есть»
        assert_eq!(
            summary("VERB,impf,intr sing,3per,futr,indc", "быть"),
            (
                "AUX".into(),
                "Mood=Ind|Number=Sing|Person=3|Tense=Pres|VerbForm=Fin|Voice=Act".into(),
                vec!["futr".into(), "intr".into()]
            )
        );
        assert!(summary("VERB,impf,intr sing,3per,pres,indc", "быть").2 == ["intr"]);
    }

    #[test]
    fn exceptions_merge_with_other_part_of_speech() {
        // «несколько» (ADVB) сливается с числительным в винительном, «что-то» - с местоимением
        assert_eq!(
            summary("ADVB", "несколько").1,
            summary("NUMR,inan accs", "несколько").1
        );
        assert_eq!(
            summary("ADVB", "что-то").1,
            summary("NPRO,neut sing,nomn", "что-то").1
        );
    }

    #[test]
    fn pronoun_features() {
        let feats = |tag: &str, lemma: &str| summary(tag, lemma).1;
        assert_eq!(
            feats("NPRO sing,nomn", "никто"),
            "Animacy=Anim|Case=Nom|Gender=Masc|Number=Sing|PronType=Neg"
        );
        assert_eq!(
            feats("NPRO plur,nomn", "многие"),
            "Animacy=Anim|Case=Nom|Number=Plur|PronType=Tot"
        );
        assert!(feats("NPRO,neut sing,nomn", "многое").starts_with("Animacy=Inan"));
        assert_eq!(feats("PRED,pres", "нельзя"), "Polarity=Neg");
        assert_eq!(
            feats("NPRO,neut sing,gent", "нечего"),
            "Case=Gen|Polarity=Neg"
        );
        // Винительного у «некого» в SynTagRus нет: родительный, как у «нечего»
        assert_eq!(feats("NPRO accs", "некого"), "Case=Gen|Polarity=Neg");
        // «некогда» - наречие «когда-то» без отрицания, предикатив - с отрицанием
        assert_eq!(feats("ADVB", "некогда"), "Degree=Pos|PronType=Ind");
        assert_eq!(feats("PRED,pres", "некогда"), "Polarity=Neg");
    }

    #[test]
    fn abbreviations() {
        // Склоняемая аббревиатура сохраняет признаки, несклоняемая и сокращение слова - нет
        assert_eq!(
            summary("NOUN,inan,masc,Abbr sing,gent", "вуз").1,
            "Abbr=Yes|Animacy=Inan|Case=Gen|Gender=Masc|Number=Sing"
        );
        let mut abbreviated = convert("NOUN,inan,neut sing,gent,Abbr", "");
        strip_abbreviation(&mut abbreviated);
        assert_eq!(feats_string(&abbreviated.feats), "Abbr=Yes");
        let mut word = convert("NOUN,inan,neut sing,gent", "");
        strip_abbreviation(&mut word);
        assert!(word.feats.contains_key("Case"));
    }

    #[test]
    fn substantive_pronouns() {
        let pronoun = |base: &str, tag: &str| {
            substantive_pronoun(base, tag)
                .map(|(lemma, ud)| (lemma, ud.pos, feats_string(&ud.feats)))
        };
        assert_eq!(
            pronoun("тот", "ADJF,Subx,Apro,Anph neut,sing,gent"),
            Some((
                "то",
                "PRON",
                "Animacy=Inan|Case=Gen|Gender=Neut|Number=Sing|PronType=Dem".into()
            ))
        );
        assert_eq!(
            pronoun("весь", "ADJF,Subx,Apro neut,sing,datv"),
            Some((
                "всё",
                "PRON",
                "Animacy=Inan|Case=Dat|Gender=Neut|Number=Sing|PronType=Tot".into()
            ))
        );
        assert_eq!(
            pronoun("весь", "ADJF,Subx,Apro anim,plur,accs"),
            Some((
                "все",
                "PRON",
                "Animacy=Anim|Case=Acc|Number=Plur|PronType=Tot".into()
            ))
        );
        assert_eq!(pronoun("весь", "ADJF,Subx,Apro inan,plur,accs"), None);
        assert_eq!(pronoun("тот", "ADJF,Subx,Apro masc,sing,gent"), None);
        assert_eq!(pronoun("этот", "ADJF,Apro neut,sing,gent"), None);
    }

    #[test]
    fn context_copies() {
        let copy = |base: &str, tag: &str| {
            context_copy(base, tag).map(|ud| (ud.pos, feats_string(&ud.feats)))
        };
        assert_eq!(
            copy("быть", "VERB,impf,intr sing,3per,pres,indc"),
            Some((
                "VERB",
                "Mood=Ind|Number=Sing|Person=3|Tense=Pres|VerbForm=Fin|Voice=Act".into()
            ))
        );
        assert_eq!(copy("бы", "PRCL"), Some(("PART", "".into())));
        assert_eq!(
            copy("один", "ADJF,Apro,Anum femn,sing,gent"),
            Some(("DET", "Case=Gen|Gender=Fem|Number=Sing|PronType=Ind".into()))
        );
        assert_eq!(
            copy("друг", "NOUN,anim,masc sing,ablt"),
            Some(("PRON", "Case=Ins|PronType=Rcp".into()))
        );
        assert_eq!(copy("друг", "NOUN,anim,masc plur,ablt"), None);
        assert_eq!(copy("друг", "NOUN,anim,masc sing,voct"), None);
        assert_eq!(copy("стать", "VERB,perf,intr sing,3per,futr,indc"), None);
    }

    #[test]
    fn numeral_gender_copies() {
        let genders: Vec<String> = numeral_genders("два", "NUMR gent")
            .iter()
            .map(|ud| feats_string(&ud.feats))
            .collect();
        assert_eq!(
            genders,
            [
                "Case=Gen|Gender=Fem|NumForm=Word|NumType=Card",
                "Case=Gen|Gender=Neut|NumForm=Word|NumType=Card"
            ]
        );
        assert_eq!(
            summary("NUMR gent", "два").1,
            "Case=Gen|Gender=Masc|NumForm=Word|NumType=Card"
        );
        assert!(numeral_genders("два", "NUMR masc,nomn").is_empty());
        assert!(numeral_genders("три", "NUMR gent").is_empty());
    }

    #[test]
    fn comparative_adverbs() {
        let short = ["хорошо".to_string()];
        assert_eq!(positive_adverbs("хороший", &short)[0], "хорошо");
        assert_eq!(positive_adverbs("поздний", &[]), ["поздно"]);
        assert_eq!(positive_adverbs("ранний", &[]), ["ранно", "рано"]);
        assert_eq!(positive_adverbs("маленький", &[]), ["мало"]);
        assert!(positive_adverbs("малый", &["мало".to_string()]).is_empty());
        assert_eq!(
            feats_string(&comparative_adverb("COMP,Qual", "мало").feats),
            "Degree=Cmp|PronType=Tot"
        );
        let ud = comparative_adverb("COMP,Qual", "поздно");
        assert_eq!(
            (ud.pos, feats_string(&ud.feats), ud.labels),
            ("ADV", "Degree=Cmp".into(), vec!["Qual".to_string()])
        );
    }

    #[test]
    fn unknown_grammemes_are_reported() {
        assert!(unknown_grammemes("NOUN,anim,masc,Sgtm sing,nomn").is_empty());
        assert!(unknown_grammemes("VERB,impf,tran plur,pres,indc").is_empty());
        assert_eq!(unknown_grammemes("ABCD,Xyzw sing,nomn"), ["ABCD", "Xyzw"]);
    }

    #[test]
    fn labels_keep_lexical_grammemes() {
        let (_, _, labels) = summary("NOUN,inan,masc,Slng sing,nomn", "комп");
        assert_eq!(labels, ["Slng"]);
        let (_, _, labels) = summary("ADVB,Prdx,Infr", "чуток");
        assert_eq!(labels, ["Infr", "Prdx"]);
        let (_, _, labels) = summary("VERB,impf,intr,INFN sing,3per,pres,indc", "идти");
        assert_eq!(labels, ["intr"]);
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
    fn counting_forms() {
        assert!(counting_form("NOUN,inan,masc plur,gent", "год", "лет"));
        assert!(!counting_form("NOUN,inan,masc plur,gent", "год", "годов"));
        assert!(!counting_form(
            "NOUN,anim,masc sing,nomn",
            "человек",
            "человек"
        ));
        assert!(counting_form(
            "NOUN,anim,masc plur,gent",
            "человек",
            "человек"
        ));
    }

    #[test]
    fn lemma_forms() {
        assert_eq!(
            lemma_form("NOUN,anim,femn,Sgtm,Surn sing,gent", "петров", "петровой"),
            Some(LemmaForm::With(&["NOUN", "femn", "sing", "nomn"]))
        );
        assert!(lemma_form("NOUN,anim,masc,Sgtm,Surn sing,gent", "петров", "петрова").is_none());
        assert_eq!(
            lemma_form(
                "NOUN,anim,masc,Patr sing,ablt",
                "василиевич",
                "васильевичем"
            ),
            Some(LemmaForm::With(&["NOUN", "masc", "sing", "nomn"]))
        );
        assert_eq!(
            lemma_form("ADJF,Supr,Qual femn,sing,gent", "большой", "наибольшей"),
            Some(LemmaForm::With(&["ADJF", "Supr", "masc", "sing", "nomn"]))
        );
        assert!(lemma_form("ADJS,Qual masc,sing", "нужный", "нужен").is_none());
        assert!(lemma_form("ADJS femn,sing", "должный", "должна").is_some());
        assert_eq!(
            lemma_form("ADJS,Apro masc,sing", "таковой", "таков"),
            Some(LemmaForm::With(&["ADJS", "masc", "sing"]))
        );
        assert_eq!(lemma_form("PREP", "о", "об"), Some(LemmaForm::Own));
        assert!(lemma_form("PREP", "в", "во").is_none());
        assert!(lemma_form("NOUN,inan,femn plur,gent", "деньга", "денег").is_some());
    }
}
