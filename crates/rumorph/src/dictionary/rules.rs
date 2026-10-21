//! Разбор токенов, которых нет в словаре: числа, знаки, латиница, римские числа, инициалы,
//! аббревиатуры, сокращения с точкой и наречия вида «по-русски». Части речи и признаки - по соглашениям SynTagRus,
//! доли неоднозначных разборов - по частотам в его обучающей части

use super::{Analysis, Dictionary, Source, UdTags};

/// Символы вне символьных блоков Unicode, которые в UD - `SYM`, а не `PUNCT`
const SYMBOLS: &[char] = &[
    '%', '‰', '$', '£', '¥', '°', '+', '=', '±', '×', '÷', '§', '©', '®',
];

/// Порядковые из цифр с окончанием в обучающей части SynTagRus: падеж, род, число,
/// одушевленность и сколько раз встретились; доля разбора по образцу - число плюс один
const NUMERAL_FEATS: &[(&str, &str, &str, &str, f64)] = &[
    ("Gen", "", "Plur", "", 196.0),
    ("Loc", "Masc", "Sing", "", 136.0),
    ("Gen", "Masc", "Sing", "", 132.0),
    ("Acc", "", "Plur", "Inan", 83.0),
    ("Loc", "", "Plur", "", 76.0),
    ("Gen", "Fem", "Sing", "", 38.0),
    ("Nom", "Masc", "Sing", "", 27.0),
    ("Nom", "Fem", "Sing", "", 20.0),
    ("Loc", "Fem", "Sing", "", 11.0),
    ("Acc", "Masc", "Sing", "Inan", 11.0),
    ("Acc", "Fem", "Sing", "", 10.0),
    ("Dat", "Masc", "Sing", "", 8.0),
    ("Gen", "Neut", "Sing", "", 8.0),
    ("Ins", "Masc", "Sing", "", 7.0),
    ("Nom", "", "Plur", "", 7.0),
    ("Acc", "Neut", "Sing", "", 7.0),
    ("Ins", "Fem", "Sing", "", 7.0),
    ("Loc", "Neut", "Sing", "", 5.0),
    ("Dat", "", "Plur", "", 3.0),
    ("Ins", "Neut", "Sing", "", 2.0),
];

/// Падежные окончания количественных числительных из цифр: «5-ти», «2-х», «3-мя», «4-м»
const CARDINAL_ENDINGS: &[&str] = &[
    "ти", "ть", "х", "ух", "ех", "ми", "мя", "м", "ем", "ём", "у",
];

/// Одиночные буквы, которые могут быть римским числом: доли римского числа и латиницы
/// по обучающей части SynTagRus плюс один («C» и «L» там - только латиница)
const ROMAN_LETTERS: &[(&str, f64, f64)] = &[
    ("I", 14.0, 10.0),
    ("V", 5.0, 2.0),
    ("X", 9.0, 12.0),
    ("L", 1.0, 2.0),
    ("C", 1.0, 6.0),
];

/// Номера тегов UD разборов по правилам в `Dictionary::ud_tags`
pub(super) struct Tags {
    punct: u16,
    symbol: u16,
    cardinal: u16,
    ordinal: u16,
    roman: u16,
    foreign: u16,
    given: u16,
    patronymic: u16,
    surname: u16,
    organization: u16,
    abbreviation: u16,
    adverb: u16,
    other: u16,
}

impl Tags {
    /// Находит теги в `ud_tags` и добавляет недостающие
    pub(super) fn add(ud_tags: &mut UdTags) -> Self {
        let mut tag = |pos: &str, feats: &[(&str, &str)]| ud_tags.find_or_add(pos, feats);
        let abbreviated = |name_type| [("Abbr", "Yes"), ("NameType", name_type)];
        Tags {
            punct: tag("PUNCT", &[]),
            symbol: tag("SYM", &[]),
            cardinal: tag("NUM", &[("NumForm", "Digit"), ("NumType", "Card")]),
            ordinal: tag("ADJ", &[("NumForm", "Digit"), ("NumType", "Ord")]),
            roman: tag("ADJ", &[("NumForm", "Roman"), ("NumType", "Ord")]),
            foreign: tag("X", &[("Foreign", "Yes")]),
            given: tag("PROPN", &abbreviated("Giv")),
            patronymic: tag("PROPN", &abbreviated("Pat")),
            surname: tag("PROPN", &abbreviated("Sur")),
            organization: tag("PROPN", &abbreviated("Com")),
            abbreviation: tag("NOUN", &[("Abbr", "Yes")]),
            adverb: tag("ADV", &[("Degree", "Pos")]),
            other: tag("X", &[]),
        }
    }
}

impl Dictionary {
    /// Разборы токена, которого нет в словаре; пустой список, если ни одно правило не подошло
    pub(super) fn rules(&self, word: &str) -> Vec<Analysis> {
        let tags = &self.rule_tags;
        let chars: Vec<char> = word.chars().collect();
        let lower = word.to_lowercase();
        if chars.is_empty() {
            return Vec::new();
        }
        if let Some(digits) = number(word) {
            if !digits {
                return self.shares(word, &[(word, tags.cardinal, 1.0)]);
            }
            // Четырехзначное число в пределах 1000-2100 - обычно год («в 2008 году»), но круглые
            // сотни - нет, кроме 1900 и 2000
            let (ordinal, cardinal) = match (chars.len(), word.parse::<u32>()) {
                (4, Ok(year @ 1000..=2100)) if year % 100 != 0 || year == 1900 || year == 2000 => {
                    (2508.0, 171.0)
                }
                (1 | 2, _) => (417.0, 4595.0),
                _ => (11.0, 1207.0),
            };
            return self.shares(
                word,
                &[
                    (word, tags.cardinal, cardinal),
                    (word, tags.ordinal, ordinal),
                ],
            );
        }
        if !token(&chars) {
            return Vec::new();
        }
        if chars.iter().all(|c| !c.is_alphanumeric()) {
            let tag = if chars.iter().all(|&c| symbol(c) || joiner(c)) {
                tags.symbol
            } else {
                tags.punct
            };
            return self.shares(word, &[(word, tag, 1.0)]);
        }
        if let Some(analyses) = self.numeral_with_ending(word) {
            return analyses;
        }
        // Наречия «по-бурятски», «по-молодому», «по-лисьи»
        if let Some(rest) = lower.strip_prefix("по-")
            && rest.chars().all(cyrillic)
            && ["ски", "цки", "ому", "ему", "ьи"]
                .iter()
                .any(|ending| rest.ends_with(ending))
        {
            return self.shares(word, &[(&lower, tags.adverb, 1.0)]);
        }
        if roman(word) {
            let (ordinal, foreign) =
                match ROMAN_LETTERS.iter().find(|(letter, _, _)| *letter == word) {
                    Some(&(_, ordinal, foreign)) => (ordinal, foreign),
                    // «CV», «CI», «XL» - чаще латинские сокращения, чем римские числа
                    None if chars.len() == 2 && word.contains(['C', 'L']) => (1.0, 3.0),
                    None => (471.0, 19.0),
                };
            return self.shares(
                word,
                &[(word, tags.roman, ordinal), (&lower, tags.foreign, foreign)],
            );
        }
        // Римские в нижнем регистре («xix») и диапазоны римских («II-III», «III–IV»); строчные
        // с «c», «l» («ci», «cl») - латинские слова
        let upper = word.to_uppercase();
        if chars.len() > 1 && chars.iter().all(|c| "ivx".contains(*c)) && roman(&upper) {
            return self.shares(
                word,
                &[(word, tags.roman, 1.0), (&lower, tags.foreign, 1.0)],
            );
        }
        if word.contains(RANGE) && word.split(RANGE).all(roman) {
            return self.shares(word, &[(word, tags.roman, 1.0)]);
        }
        if chars.iter().any(|c| c.is_alphabetic())
            && !chars.iter().any(|&c| cyrillic(c))
            && chars
                .iter()
                .all(|&c| c.is_alphanumeric() || "'’-.&_".contains(c))
        {
            return self.shares(word, &[(&lower, tags.foreign, 1.0)]);
        }
        // Инициал - заглавная буква с точкой или без нее («А.», «А»)
        if let [letter, '.'] | [letter] = chars[..]
            && cyrillic(letter)
            && letter.is_uppercase()
            && !"ЪЫЬ".contains(letter)
        {
            let lemma = letter.to_lowercase().to_string();
            return self.shares(
                word,
                &[
                    (&lemma, tags.given, 561.0),
                    (&lemma, tags.patronymic, 297.0),
                    (&lemma, tags.surname, 79.0),
                ],
            );
        }
        if let Some(analyses) = self.abbreviation(word) {
            return analyses;
        }
        if chars.len() > 1 && chars.iter().all(|&c| cyrillic(c) && c.is_uppercase()) {
            let mut analyses = self.shares(
                word,
                &[
                    (&lower, tags.organization, 510.0),
                    (&lower, tags.abbreviation, 327.0),
                ],
            );
            // Длинное слово капсом в SynTagRus - чаще обычное слово заглавными буквами, чем
            // аббревиатура: его разборы угадываются по форме с заглавной буквы
            if chars.len() >= LONG_CAPS {
                let capitalized: String = chars[..1]
                    .iter()
                    .chain(lower.chars().skip(1).collect::<Vec<_>>().iter())
                    .collect();
                let guessed = self.guess(&capitalized);
                if !guessed.is_empty() {
                    let share = if chars.len() > LONG_CAPS { 0.85 } else { 0.5 };
                    for analysis in &mut analyses {
                        analysis.prob *= 1.0 - share;
                    }
                    analyses.extend(guessed.into_iter().map(|analysis| Analysis {
                        word: word.to_owned(),
                        prob: analysis.prob * share,
                        ..analysis
                    }));
                    analyses.sort_by(|a, b| b.prob.total_cmp(&a.prob));
                }
            }
            return analyses;
        }
        Vec::new()
    }

    /// Запасной разбор токена, который не разобрали ни словарь, ни правила, ни угадывание
    /// («км/ч», «РосПил»): `X` без признаков, лемма - токен в нижнем регистре. Токен с пробелом,
    /// управляющим или невидимым символом разборов не получает
    pub(super) fn fallback(&self, word: &str) -> Vec<Analysis> {
        let chars: Vec<char> = word.chars().collect();
        if !token(&chars) {
            return Vec::new();
        }
        self.shares(word, &[(&word.to_lowercase(), self.rule_tags.other, 1.0)])
    }

    /// Разборы токена без лексемы с долями вероятности: «лемма, тег UD, вес»; словоформа - как
    /// в тексте, чтобы разбор восстанавливался из pickle тем же правилом
    fn shares(&self, word: &str, analyses: &[(&str, u16, f64)]) -> Vec<Analysis> {
        let total: f64 = analyses.iter().map(|&(_, _, weight)| weight).sum();
        let mut result: Vec<Analysis> = analyses
            .iter()
            .map(|&(lemma, ud, weight)| Analysis {
                word: word.to_owned(),
                lemma: lemma.to_owned(),
                ud,
                tag: None,
                prob: weight / total,
                source: Source::Rule,
                entry: None,
            })
            .collect();
        result.sort_by(|a, b| b.prob.total_cmp(&a.prob));
        result
    }

    /// Числительное из цифр с окончанием. Порядковое («2005-го», «90-х») - по образцу «1-го»
    /// из словаря: те же разборы и таблица словоизменения с основой из числа, доли - по частотам
    /// таких порядковых в SynTagRus. Короткое окончание без образца («5-ти», «2-мя») -
    /// количественное с леммой из цифр
    fn numeral_with_ending(&self, word: &str) -> Option<Vec<Analysis>> {
        let (digits, ending) = word.rsplit_once('-')?;
        if ending.is_empty() || !ending.chars().all(cyrillic) {
            return None;
        }
        // Римское с порядковым окончанием («XIX-го»), но не буква со словом («X-хромосома»)
        if roman(digits)
            && !self
                .candidates(&format!("1-{}", ending.to_lowercase()))
                .is_empty()
        {
            return Some(self.shares(word, &[(digits, self.rule_tags.roman, 1.0)]));
        }
        // Слева число или диапазон чисел: «2005-го», «10-15-го», «1840–1860-х»
        if !digits
            .split(RANGE)
            .all(|group| !group.is_empty() && group.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        let lower = word.to_lowercase();
        let mut analyses: Vec<Analysis> = self
            .merge(self.candidates(&format!("1-{}", ending.to_lowercase())))
            .into_iter()
            .filter_map(|template| {
                let entry = template.entry?;
                let lexeme = self.lexeme(entry.table, entry.form, &lower)?;
                Some(Analysis {
                    word: word.to_owned(),
                    lemma: self.spell(&lexeme, lexeme.lemma_form(entry.form)),
                    prob: self.numeral_weight(template.ud),
                    source: Source::Rule,
                    ..template
                })
            })
            .collect();
        if !analyses.is_empty() {
            let total: f64 = analyses.iter().map(|analysis| analysis.prob).sum();
            for analysis in &mut analyses {
                analysis.prob /= total;
            }
            analyses.sort_by(|a, b| b.prob.total_cmp(&a.prob));
            return Some(analyses);
        }
        // Падежные окончания количественных - в любом регистре («5-ТИ»), а прочие короткие -
        // только строчными: заглавные без образца («5-Б») - литеры, а не окончания
        let ending_lower = ending.to_lowercase();
        let cardinal = CARDINAL_ENDINGS.contains(&ending_lower.as_str())
            || (ending.chars().count() <= 3 && ending.chars().all(char::is_lowercase));
        cardinal.then(|| self.shares(word, &[(digits, self.rule_tags.cardinal, 1.0)]))
    }

    /// Словарные порядковые из цифр («90-е») получают те же доли по SynTagRus, что и разобранные
    /// по образцу: частот OpenCorpora у них нет или они мало что говорят
    pub(super) fn reweight_numerals(&self, analyses: &mut [Analysis]) {
        let combi = |analysis: &Analysis| self.has_code(analysis.ud, self.combined_numeral);
        if !analyses.iter().all(combi) {
            return;
        }
        for analysis in analyses.iter_mut() {
            analysis.prob = self.numeral_weight(analysis.ud);
        }
        let total: f64 = analyses.iter().map(|analysis| analysis.prob).sum();
        for analysis in analyses.iter_mut() {
            analysis.prob /= total;
        }
        analyses.sort_by(|a, b| b.prob.total_cmp(&a.prob));
    }

    /// Доля разбора порядкового из цифр по его падежу, роду, числу и одушевленности
    fn numeral_weight(&self, ud: u16) -> f64 {
        let feat = |name: &str| self.feat(ud, name).unwrap_or_default();
        let key = (
            feat("Case"),
            feat("Gender"),
            feat("Number"),
            feat("Animacy"),
        );
        1.0 + NUMERAL_FEATS
            .iter()
            .find(|&&(case, gender, number, animacy, _)| (case, gender, number, animacy) == key)
            .map_or(0.0, |&(.., count)| count)
    }

    /// Сокращение с точкой («т.», «им.») - разборы словарного сокращения без точки
    fn abbreviation(&self, word: &str) -> Option<Vec<Analysis>> {
        let lower = word.to_lowercase();
        let base = lower.strip_suffix('.')?;
        if base.is_empty() || !base.chars().all(cyrillic) {
            return None;
        }
        // Сокращение - признак Abbr=Yes или помета Abbr у склоняемой аббревиатуры
        let abbreviated = |analysis: &Analysis| {
            self.ud
                .get(analysis.ud)
                .is_some_and(|tag| tag.feat("Abbr") == Some("Yes") || tag.has_label("Abbr"))
        };
        let mut analyses: Vec<Analysis> = self
            .merge(self.candidates(base))
            .into_iter()
            .filter(abbreviated)
            .collect();
        let total: f64 = analyses.iter().map(|analysis| analysis.prob).sum();
        if analyses.is_empty() || total == 0.0 {
            return None;
        }
        for analysis in &mut analyses {
            analysis.word = word.to_owned();
            analysis.prob /= total;
            analysis.source = Source::Rule;
            analysis.entry = None;
        }
        Some(analyses)
    }
}

/// Длина слова капсом, с которой его разборы угадываются, а не только как аббревиатура
const LONG_CAPS: usize = 7;

/// Разделители диапазона чисел: дефис и тире
const RANGE: [char; 3] = ['-', '–', '—'];

/// Токен, который может получить разбор: непустой, без пробелов, управляющих и невидимых
/// символов и не из одних соединителей эмодзи
fn token(chars: &[char]) -> bool {
    !chars.is_empty()
        && !chars
            .iter()
            .any(|&c| c.is_whitespace() || c.is_control() || invisible(c))
        && !chars.iter().all(|&c| joiner(c))
}

pub(super) fn cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
}

/// Символ: знаки из `SYMBOLS` и символьные блоки Unicode - валюты, буквоподобные («№», «™»),
/// стрелки, математические операторы («−»), технические и геометрические знаки, эмодзи
fn symbol(c: char) -> bool {
    SYMBOLS.contains(&c)
        || matches!(
            c,
            '\u{20A0}'..='\u{20CF}'
                | '\u{2100}'..='\u{214F}'
                | '\u{2190}'..='\u{23FF}'
                | '\u{25A0}'..='\u{27BF}'
                | '\u{2B00}'..='\u{2BFF}'
                | '\u{1F000}'..='\u{1FAFF}'
        )
}

/// Символ форматирования (категория Unicode Cf): мягкий перенос, нулевые пробелы и соединители,
/// метки направления текста, BOM; кроме ZWJ и тегов, которые бывают внутри эмодзи
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200C}'
            | '\u{200E}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
    )
}

/// Невидимая часть составного эмодзи: соединитель, селекторы вариантов и теги флагов регионов
fn joiner(c: char) -> bool {
    matches!(
        c,
        '\u{200D}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0020}'..='\u{E007F}'
    )
}

/// Число цифрами: `Some(true)` - только цифры, `Some(false)` - со знаком, дробной частью,
/// разделителями групп, времени и дат или диапазон («3,14», «-5», «10 000», «17:04», «2/3»,
/// «01.02.2003», «1990-2000»)
fn number(word: &str) -> Option<bool> {
    let unsigned = word.strip_prefix(['+', '-', '−']).unwrap_or(word);
    unsigned
        .split([
            ',', '.', ':', '/', ' ', '\u{A0}', '\u{2007}', '\u{2009}', '\u{202F}', '-', '–', '—',
        ])
        .all(|group| !group.is_empty() && group.bytes().all(|b| b.is_ascii_digit()))
        .then(|| unsigned == word && unsigned.bytes().all(|b| b.is_ascii_digit()))
}

/// Римское число из I, V, X, L, C в каноническом виде: «XIV», но не «IIII» и не «IC»
fn roman(word: &str) -> bool {
    const DIGITS: [(&str, u32); 9] = [
        ("C", 100),
        ("XC", 90),
        ("L", 50),
        ("XL", 40),
        ("X", 10),
        ("IX", 9),
        ("V", 5),
        ("IV", 4),
        ("I", 1),
    ];
    if word.is_empty() || !word.chars().all(|c| "IVXLC".contains(c)) {
        return false;
    }
    let mut value = 0;
    let mut rest = word;
    while let Some(&(digit, amount)) = DIGITS.iter().find(|(digit, _)| rest.starts_with(digit)) {
        value += amount;
        rest = &rest[digit.len()..];
        if value >= 400 {
            return false;
        }
    }
    if !rest.is_empty() || value >= 400 {
        return false;
    }
    let mut canonical = String::new();
    let mut left = value;
    for (digit, amount) in DIGITS {
        while left >= amount {
            canonical.push_str(digit);
            left -= amount;
        }
    }
    canonical == word
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::tests::dictionary_of;

    /// Образец порядковых из цифр «1-й», «1-го» и сокращение «им»
    fn dictionary() -> Dictionary {
        let ordinal =
            |case| format!("Case={case}|Gender=Masc|NumForm=Combi|NumType=Ord|Number=Sing");
        dictionary_of(
            &[
                (
                    "ADJ",
                    "1-",
                    vec![("й", ordinal("Nom")), ("го", ordinal("Gen"))],
                ),
                (
                    "NOUN",
                    "им",
                    vec![("", "Abbr=Yes|Case=Gen|Number=Sing".to_owned())],
                ),
            ],
            &[],
        )
    }

    /// Лемма, часть речи и признаки разборов по правилам
    fn rules(dict: &Dictionary, word: &str) -> Vec<(String, String, String)> {
        dict.rules(word)
            .into_iter()
            .map(|analysis| {
                assert_eq!(
                    (analysis.word.as_str(), analysis.source),
                    (word, Source::Rule)
                );
                let tag = dict.ud_tag(analysis.ud).unwrap();
                (analysis.lemma, tag.pos().to_owned(), tag.feats_string())
            })
            .collect()
    }

    fn parts_of_speech(dict: &Dictionary, word: &str) -> Vec<String> {
        rules(dict, word)
            .into_iter()
            .map(|(_, pos, _)| pos)
            .collect()
    }

    fn analysis(lemma: &str, pos: &str, feats: &str) -> (String, String, String) {
        (lemma.to_owned(), pos.to_owned(), feats.to_owned())
    }

    #[test]
    fn numerals_by_template() {
        let dict = dictionary();
        let genitive = "Case=Gen|Gender=Masc|NumForm=Combi|NumType=Ord|Number=Sing";
        assert_eq!(
            rules(&dict, "2005-го"),
            [analysis("2005-й", "ADJ", genitive)]
        );
        assert_eq!(
            rules(&dict, "10-15-го"),
            [analysis("10-15-й", "ADJ", genitive)]
        );
        // Короткое окончание без образца - количественное с леммой из цифр
        let cardinal = "NumForm=Digit|NumType=Card";
        assert_eq!(rules(&dict, "5-ти"), [analysis("5", "NUM", cardinal)]);
        assert_eq!(rules(&dict, "3,14"), [analysis("3,14", "NUM", cardinal)]);
        // Год - чаще порядковое
        assert_eq!(parts_of_speech(&dict, "2008"), ["ADJ", "NUM"]);
        assert_eq!(parts_of_speech(&dict, "2050-летний"), Vec::<String>::new());
    }

    #[test]
    fn dictionary_numerals_weighted_by_syntagrus() {
        let ordinal = |feats: &str| format!("{feats}|NumForm=Combi|NumType=Ord|Number=Plur");
        let dict = dictionary_of(
            &[(
                "ADJ",
                "90-",
                vec![
                    ("е", ordinal("Case=Nom")),
                    ("х", ordinal("Animacy=Anim|Case=Acc")),
                    ("х", ordinal("Case=Gen")),
                    ("х", ordinal("Case=Loc")),
                ],
            )],
            &[],
        );
        let weight = |case: &str| {
            let analysis = dict
                .analyze("90-х")
                .into_iter()
                .find(|analysis| dict.feat(analysis.ud, "Case") == Some(case))
                .unwrap();
            dict.numeral_weight(analysis.ud)
        };
        // Доля - число в SynTagRus плюс один; сочетания, которых там нет, - единица
        assert_eq!(
            (weight("Gen"), weight("Loc"), weight("Acc")),
            (197.0, 77.0, 1.0)
        );
        let probs: Vec<(Option<&str>, f64)> = dict
            .analyze("90-х")
            .iter()
            .map(|analysis| (dict.feat(analysis.ud, "Case"), analysis.prob))
            .collect();
        assert_eq!(
            probs,
            [
                (Some("Gen"), 197.0 / 275.0),
                (Some("Loc"), 77.0 / 275.0),
                (Some("Acc"), 1.0 / 275.0)
            ]
        );
    }

    #[test]
    fn numerals_mixed_with_other_analyses_keep_dictionary_shares() {
        let dict = dictionary_of(
            &[
                (
                    "ADJ",
                    "90-",
                    vec![(
                        "х",
                        "Case=Gen|NumForm=Combi|NumType=Ord|Number=Plur".to_owned(),
                    )],
                ),
                (
                    "NOUN",
                    "90-",
                    vec![("х", "Case=Gen|Number=Plur".to_owned())],
                ),
            ],
            &[],
        );
        let probs: Vec<f64> = dict.analyze("90-х").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [0.5, 0.5]);
    }

    #[test]
    fn roman_and_latin() {
        let dict = dictionary();
        let roman = "NumForm=Roman|NumType=Ord";
        assert_eq!(rules(&dict, "XIX-го"), [analysis("XIX", "ADJ", roman)]);
        // Буква со словом - не римское порядковое
        assert!(rules(&dict, "X-хромосома").is_empty());
        assert_eq!(parts_of_speech(&dict, "xix"), ["ADJ", "X"]);
        assert_eq!(parts_of_speech(&dict, "CV"), ["X", "ADJ"]);
        assert_eq!(rules(&dict, "ci"), [analysis("ci", "X", "Foreign=Yes")]);
        assert_eq!(parts_of_speech(&dict, "II-III"), ["ADJ"]);
    }

    #[test]
    fn initials_abbreviations_and_symbols() {
        let dict = dictionary();
        let initials: Vec<_> = rules(&dict, "Х")
            .into_iter()
            .map(|(lemma, _, feats)| (lemma, feats))
            .collect();
        assert_eq!(
            initials,
            ["Giv", "Pat", "Sur"].map(|name| ("х".to_owned(), format!("Abbr=Yes|NameType={name}")))
        );
        assert_eq!(
            rules(&dict, "им."),
            [analysis("им", "NOUN", "Abbr=Yes|Case=Gen|Number=Sing")]
        );
        assert_eq!(parts_of_speech(&dict, "ГОСТ"), ["PROPN", "NOUN"]);
        assert_eq!(
            rules(&dict, "по-бурятски"),
            [analysis("по-бурятски", "ADV", "Degree=Pos")]
        );
        assert_eq!(parts_of_speech(&dict, "%"), ["SYM"]);
        assert_eq!(parts_of_speech(&dict, "«"), ["PUNCT"]);
        // Не подошло ни одно правило - запасной X; токен с пробелом разборов не получает
        assert!(rules(&dict, "км/ч").is_empty());
        let fallback = dict.fallback("км/ч").remove(0);
        assert_eq!((fallback.lemma.as_str(), fallback.prob), ("км/ч", 1.0));
        assert!(dict.fallback("км ч").is_empty());
    }

    #[test]
    fn numbers() {
        assert_eq!(number("2026"), Some(true));
        assert_eq!(number("3,14"), Some(false));
        assert_eq!(number("-5"), Some(false));
        assert_eq!(number("1.5"), Some(false));
        assert_eq!(number("10 000"), Some(false));
        assert_eq!(number("10\u{A0}000"), Some(false));
        assert_eq!(number("10\u{2009}000"), Some(false));
        assert_eq!(number("10\u{2007}000"), Some(false));
        assert_eq!(number("17:04"), Some(false));
        assert_eq!(number("2/3"), Some(false));
        assert_eq!(number("01.02.2003"), Some(false));
        assert_eq!(number("1,"), None);
        assert_eq!(number("1..2"), None);
        assert_eq!(number("12a"), None);
        assert_eq!(number(" "), None);
        assert_eq!(number(""), None);
    }

    #[test]
    fn symbols() {
        for c in ['%', '№', '€', '−', '♥', '→', '😀'] {
            assert!(symbol(c), "{c}");
        }
        for c in ['.', '«', '<', '-', '…'] {
            assert!(!symbol(c), "{c}");
        }
    }

    #[test]
    fn ranges() {
        assert_eq!(number("1990-2000"), Some(false));
        assert_eq!(number("37–38"), Some(false));
        assert_eq!(number("3—4"), Some(false));
        assert_eq!(number("5-"), None);
        assert_eq!(number("5-Б"), None);
    }

    #[test]
    fn tokens() {
        assert!(token(&['к', 'м', '/', 'ч']));
        assert!(!token(&[]));
        assert!(!token(&['к', ' ', 'м']));
        assert!(!token(&['к', '\u{200B}']));
        assert!(!token(&['\u{200D}']));
    }

    #[test]
    fn roman_numerals() {
        for word in ["I", "IV", "XIV", "XX", "XXI", "XLIX", "CCCXC"] {
            assert!(roman(word), "{word}");
        }
        for word in ["", "IIII", "IC", "VX", "CD", "MIX", "ii", "XXL"] {
            assert!(!roman(word), "{word}");
        }
        // Длинная строка из «C» отбрасывается без переполнения суммы
        assert!(!roman(&"C".repeat(100_000)));
    }
}
