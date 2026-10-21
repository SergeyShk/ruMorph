//! Угадывание незнакомых слов по окончанию: формы словарных слов с тем же окончанием и их
//! таблицы словоизменения. Слово с заглавной буквы угадывается в основном как имя собственное

use super::{Analysis, Dictionary, Entry, Source};

/// Индекс угадывания незнакомых слов по окончанию
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GuessIndex {
    /// Нарицательные слова
    Common,
    /// Имена собственные: для слов с заглавной буквы
    Proper,
}

impl GuessIndex {
    /// Наибольшая длина окончания в буквах
    pub const MAX_ENDING: usize = 5;

    /// Первый байт ключа окончания в автомате угадывания
    pub(crate) fn byte(self) -> u8 {
        match self {
            GuessIndex::Common => 0,
            GuessIndex::Proper => 1,
        }
    }
}

/// Доля имен собственных среди незнакомых словарю слов с заглавной буквы в SynTagRus
const PROPER_SHARE: f64 = 0.89;

/// Доля имен собственных среди незнакомых слов с заглавной буквы на -ский, -цкий и их формы
/// («Тильзитского», «Стэнфордская»): в SynTagRus это чаще прилагательные (0,43 - имена), а доля
/// прилагательных делится между несколькими их разборами, поэтому здесь она ниже
const ADJECTIVAL_PROPER_SHARE: f64 = 0.25;

/// Число имен собственных (не аббревиатур) в обучающей части SynTagRus: у имени в словаре полная
/// парадигма множественного числа, а в текстах оно редкость, поэтому угаданная форма весит
/// по частоте своего числа («Шатов» - фамилия, а не «Шат» во множественном числе). Падеж
/// остается за частотами словаря: «Копытова» - чаще родительный, чем женская фамилия
const PROPER_NUMBER: &[(&str, f64)] = &[("Sing", 34328.0), ("Plur", 815.0)];

/// Слова длиннее не угадываются: окончание у них ничего не говорит, а копии слова в разборах
/// дорого стоят
const MAX_GUESSED_LENGTH: usize = 100;

impl Dictionary {
    /// Угаданные разборы незнакомого слова кириллицей в нижнем регистре или с заглавной буквы
    pub(super) fn guess(&self, word: &str) -> Vec<Analysis> {
        let chars: Vec<char> = word.chars().collect();
        if chars.is_empty()
            || chars.len() > MAX_GUESSED_LENGTH
            || !chars.iter().all(|&c| super::rules::cyrillic(c))
        {
            return Vec::new();
        }
        let lower = word.to_lowercase();
        // Аббревиатура с падежным окончанием («МЭРТа», «ЦУПе») угадывается, как слово с заглавной
        let capitals = chars.iter().take_while(|c| c.is_uppercase()).count();
        let abbreviation_with_ending = capitals >= 2
            && (1..=3).contains(&(chars.len() - capitals))
            && chars[capitals..].iter().all(|c| c.is_lowercase());
        // Лемма такой аббревиатуры - ее заглавная часть («КВНу» - «квн», а не «квна»)
        let stem: Option<String> =
            abbreviation_with_ending.then(|| lower.chars().take(capitals).collect());
        let proper = if adjectival(&lower) {
            ADJECTIVAL_PROPER_SHARE
        } else {
            PROPER_SHARE
        };
        let indexes: &[(GuessIndex, f64)] = if chars.iter().all(|c| c.is_lowercase()) {
            &[(GuessIndex::Common, 1.0)]
        } else if chars[1..].iter().all(|c| c.is_lowercase()) || abbreviation_with_ending {
            &[
                (GuessIndex::Proper, proper),
                (GuessIndex::Common, 1.0 - proper),
            ]
        } else {
            return Vec::new();
        };
        let mut analyses = Vec::new();
        for &(index, share) in indexes {
            let found: Vec<(Analysis, f64)> = self
                .guess_in(index, word, &lower, stem.as_deref())
                .into_iter()
                .map(|(analysis, count)| {
                    let weight = match index {
                        GuessIndex::Proper => self.proper_weight(analysis.ud),
                        GuessIndex::Common => 1.0,
                    };
                    (analysis, f64::from(count) * weight)
                })
                .collect();
            // Формы с нулевым числом словоформ отброшены, поэтому сумма больше нуля
            let total: f64 = found.iter().map(|(_, weight)| weight).sum();
            analyses.extend(found.into_iter().map(|(analysis, weight)| Analysis {
                prob: share * weight / total,
                ..analysis
            }));
        }
        let total: f64 = analyses.iter().map(|analysis| analysis.prob).sum();
        for analysis in &mut analyses {
            analysis.prob /= total;
        }
        self.merge(analyses)
    }

    /// Разборы по самому длинному окончанию слова, для которого в индексе есть подходящие формы,
    /// с числом словоформ словаря; словоформа разбора - как в тексте, лемма - в нижнем регистре.
    /// Окончание через «е» находит и формы с «ё»: «ё» суффикса формы подставляется в словоформу.
    /// С основой `stem` берутся только разборы с леммой, равной ей
    fn guess_in(
        &self,
        index: GuessIndex,
        word: &str,
        lower: &str,
        stem: Option<&str>,
    ) -> Vec<(Analysis, u32)> {
        let bounds: Vec<usize> = lower.char_indices().map(|(at, _)| at).collect();
        for length in (1..=GuessIndex::MAX_ENDING.min(bounds.len().saturating_sub(1))).rev() {
            let ending = &lower[bounds[bounds.len() - length]..];
            let Some(forms) = self
                .alphabet
                .key(index.byte(), ending)
                .and_then(|key| self.guesses.get(key.bytes()))
                .and_then(|offset| self.guess_records_at(offset))
            else {
                continue;
            };
            let found: Vec<(Analysis, u32)> = forms
                .iter()
                .filter(|&&[_, count]| count > 0)
                .filter_map(|&[packed, count]| {
                    let (table, form) = ((packed >> 16) as u16, packed as u16);
                    let (word, lower) = self.with_suffix(table, form, word, lower)?;
                    let lexeme = self.lexeme(table, form, &lower)?;
                    let lemma = self.spell(&lexeme, lexeme.lemma_form(form));
                    // Основа без гласной («тов» - «т», «Лем» - «лй») - не образец, а совпадение
                    match stem {
                        Some(stem) if lemma != stem => return None,
                        None if !lemma.chars().any(vowel) => return None,
                        _ => {}
                    }
                    let analysis = Analysis {
                        word,
                        lemma,
                        ud: lexeme.ud(form),
                        tag: Some(lexeme.tag(form)),
                        prob: 0.0,
                        source: Source::Guess,
                        entry: Some(Entry { table, form }),
                    };
                    Some((analysis, count))
                })
                .collect();
            if !found.is_empty() {
                return found;
            }
        }
        Vec::new()
    }

    /// Словоформа как в тексте и в нижнем регистре, в которых «е» на месте «ё» суффикса формы
    /// заменена на «ё»; `None`, если слово не кончается суффиксом формы
    fn with_suffix(
        &self,
        table: u16,
        form: u16,
        word: &str,
        lower: &str,
    ) -> Option<(String, String)> {
        let start = *self.table_offsets.get(usize::from(table))? as usize;
        let forms = self.paradigm(*self.tables().get(start)? as u16)?;
        if usize::from(form) >= forms.len() / 3 {
            return None;
        }
        let suffix = self.suffixes.at(usize::from(forms[usize::from(form)]));
        if lower.ends_with(suffix) {
            return Some((word.to_owned(), lower.to_owned()));
        }
        if !suffix.contains('ё') || !lower.ends_with(&suffix.replace('ё', "е")) {
            return None;
        }
        // «е» и «ё» занимают одинаково байт, поэтому граница суффикса та же; меняются только
        // «е» на местах «ё», регистр остальных букв сохраняется
        let replace = |text: &str| {
            let start = text.len() - suffix.len();
            let tail: String = text[start..]
                .chars()
                .zip(suffix.chars())
                .map(|pair| match pair {
                    ('е', 'ё') => 'ё',
                    ('Е', 'ё') => 'Ё',
                    (letter, _) => letter,
                })
                .collect();
            [&text[..start], &tail].concat()
        };
        Some((replace(word), replace(lower)))
    }

    /// Вес угаданного имени собственного по частоте его числа в SynTagRus
    fn proper_weight(&self, ud: u16) -> f64 {
        let number = self.feat(ud, "Number");
        PROPER_NUMBER
            .iter()
            .find(|&&(known, _)| Some(known) == number)
            .map_or(1.0, |&(_, count)| count)
    }

    /// Список форм для угадывания: пары `(таблица << 16) | форма`, число словоформ
    fn guess_records_at(&self, offset: u64) -> Option<&[[u32; 2]]> {
        let pairs = super::list(self.guess_records(), u32::try_from(offset).ok()?, 2)?;
        Some(pairs.as_chunks::<2>().0)
    }
}

fn vowel(c: char) -> bool {
    "аеёиоуыэюя".contains(c)
}

/// Слово на -ский, -цкий или их форму: у слова с заглавной буквы это чаще прилагательное
fn adjectival(lower: &str) -> bool {
    const ENDINGS: &[&str] = &[
        "ий", "ая", "ое", "ие", "ого", "ому", "им", "ом", "ой", "ую", "их", "ими", "ей", "ою",
    ];
    ENDINGS.iter().any(|ending| {
        lower
            .strip_suffix(ending)
            .is_some_and(|stem| stem.ends_with("ск") || stem.ends_with("цк"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjectival_endings() {
        for word in ["тильзитского", "стэнфордская", "бельгийские"]
        {
            assert!(adjectival(word), "{word}");
        }
        for word in ["шатов", "москва", "киска"] {
            assert!(!adjectival(word), "{word}");
        }
    }
}
