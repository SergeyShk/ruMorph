//! Разбор токенов, которых нет в словаре: числа, знаки, латиница, римские числа, инициалы,
//! аббревиатуры и сокращения с точкой. Части речи и признаки - по соглашениям SynTagRus,
//! доли неоднозначных разборов - по частотам в его обучающей части

use super::{Analysis, Dictionary, Source, UdTag};

/// Символы, которые в UD - `SYM`, а не `PUNCT`
const SYMBOLS: &[char] = &[
    '%', '‰', '$', '€', '£', '¥', '₽', '№', '°', '+', '=', '±', '×', '÷', '§', '©', '®', '™',
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
}

impl Tags {
    /// Находит теги в `ud_tags` и добавляет недостающие
    pub(super) fn add(ud_tags: &mut Vec<UdTag>) -> Self {
        let mut tag = |pos: &str, feats: &[(&str, &str)]| {
            let wanted = UdTag {
                pos: pos.to_owned(),
                feats: feats
                    .iter()
                    .map(|&(name, value)| (name.to_owned(), value.to_owned()))
                    .collect(),
                labels: Vec::new(),
            };
            match ud_tags.iter().position(|known| *known == wanted) {
                Some(number) => number as u16,
                None => {
                    ud_tags.push(wanted);
                    (ud_tags.len() - 1) as u16
                }
            }
        };
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
        if chars.iter().all(|c| !c.is_alphanumeric()) {
            let tag = if chars.iter().all(|c| SYMBOLS.contains(c)) {
                tags.symbol
            } else {
                tags.punct
            };
            return self.shares(word, &[(word, tag, 1.0)]);
        }
        if let Some(digits) = number(word) {
            if !digits {
                return self.shares(word, &[(word, tags.cardinal, 1.0)]);
            }
            // Четырехзначное число в пределах 1000-2100 - обычно год («в 2008 году»)
            let (ordinal, cardinal) = match (chars.len(), word.parse::<u32>()) {
                (4, Ok(1000..=2100)) => (2508.0, 171.0),
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
        if let Some(analyses) = self.numeral_with_ending(word) {
            return analyses;
        }
        if roman(word) {
            // «I» - еще и английское местоимение
            let (ordinal, foreign) = if word == "I" {
                (13.0, 9.0)
            } else {
                (470.0, 18.0)
            };
            return self.shares(
                word,
                &[(word, tags.roman, ordinal), (&lower, tags.foreign, foreign)],
            );
        }
        if chars.iter().any(|c| c.is_alphabetic())
            && !chars.iter().any(|&c| cyrillic(c))
            && chars
                .iter()
                .all(|&c| c.is_alphanumeric() || "'’-.&_".contains(c))
        {
            return self.shares(word, &[(&lower, tags.foreign, 1.0)]);
        }
        if let [letter, '.'] = chars[..]
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
        if let Some(analyses) = self.abbreviation(&lower) {
            return analyses;
        }
        if chars.len() > 1 && chars.iter().all(|&c| cyrillic(c) && c.is_uppercase()) {
            return self.shares(
                word,
                &[
                    (&lower, tags.organization, 510.0),
                    (&lower, tags.abbreviation, 327.0),
                ],
            );
        }
        Vec::new()
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
    /// из словаря: те же разборы и таблица словоизменения с основой из числа. Короткое
    /// окончание без образца («5-ти», «2-мя») - количественное с леммой из цифр
    fn numeral_with_ending(&self, word: &str) -> Option<Vec<Analysis>> {
        let (digits, ending) = word.split_once('-')?;
        if digits.is_empty()
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || ending.is_empty()
            || !ending.chars().all(|c| cyrillic(c) && c.is_lowercase())
        {
            return None;
        }
        let analyses: Vec<Analysis> = self
            .merge(self.candidates(&format!("1-{ending}")))
            .into_iter()
            .filter_map(|template| {
                let entry = template.entry?;
                let lexeme = self.lexeme(entry.table, entry.form, word)?;
                Some(Analysis {
                    word: word.to_owned(),
                    lemma: self.spell(&lexeme, lexeme.lemma_form(entry.form)),
                    source: Source::Rule,
                    ..template
                })
            })
            .collect();
        if !analyses.is_empty() {
            return Some(analyses);
        }
        (ending.chars().count() <= 3)
            .then(|| self.shares(word, &[(digits, self.rule_tags.cardinal, 1.0)]))
    }

    /// Сокращение с точкой («т.», «им.») - разборы словарного сокращения без точки
    fn abbreviation(&self, word: &str) -> Option<Vec<Analysis>> {
        let base = word.strip_suffix('.')?;
        if base.is_empty() || !base.chars().all(cyrillic) {
            return None;
        }
        let abbreviated = |analysis: &Analysis| {
            self.ud_tags[usize::from(analysis.ud)]
                .feats
                .iter()
                .any(|(name, value)| name == "Abbr" && value == "Yes")
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
            analysis.word.push('.');
            analysis.prob /= total;
            analysis.source = Source::Rule;
            analysis.entry = None;
        }
        Some(analyses)
    }
}

fn cyrillic(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
}

/// Число цифрами со знаком и дробной частью; `Some(true)` - только цифры
fn number(word: &str) -> Option<bool> {
    let unsigned = word.strip_prefix(['+', '-', '−']).unwrap_or(word);
    let (whole, fraction) = match unsigned.split_once([',', '.']) {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    (digits(whole) && fraction.is_none_or(digits)).then_some(unsigned == word && fraction.is_none())
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

    #[test]
    fn numbers() {
        assert_eq!(number("2026"), Some(true));
        assert_eq!(number("3,14"), Some(false));
        assert_eq!(number("-5"), Some(false));
        assert_eq!(number("1.5"), Some(false));
        assert_eq!(number("1,"), None);
        assert_eq!(number("12a"), None);
        assert_eq!(number(""), None);
    }

    #[test]
    fn roman_numerals() {
        for word in ["I", "IV", "XIV", "XX", "XXI", "XLIX", "CCCXC"] {
            assert!(roman(word), "{word}");
        }
        for word in ["", "IIII", "IC", "VX", "CD", "MIX", "ii", "XXL"] {
            assert!(!roman(word), "{word}");
        }
    }
}
