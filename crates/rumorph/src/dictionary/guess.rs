//! Угадывание незнакомых слов по окончанию: формы словарных слов с тем же окончанием и их
//! таблицы словоизменения. Слово с заглавной буквы угадывается в основном как имя собственное

use super::{Analysis, Dictionary, Source};
use crate::{Entry, GuessIndex};

/// Доля имен собственных среди незнакомых словарю слов с заглавной буквы в SynTagRus
const PROPER_SHARE: f64 = 0.89;

impl Dictionary {
    /// Угаданные разборы незнакомого слова кириллицей в нижнем регистре или с заглавной буквы
    pub(super) fn guess(&self, word: &str) -> Vec<Analysis> {
        let chars: Vec<char> = word.chars().collect();
        if chars.is_empty() || !chars.iter().all(|&c| super::rules::cyrillic(c)) {
            return Vec::new();
        }
        let indexes: &[(GuessIndex, f64)] = if chars.iter().all(|c| c.is_lowercase()) {
            &[(GuessIndex::Common, 1.0)]
        } else if chars[1..].iter().all(|c| c.is_lowercase()) {
            &[
                (GuessIndex::Proper, PROPER_SHARE),
                (GuessIndex::Common, 1.0 - PROPER_SHARE),
            ]
        } else {
            return Vec::new();
        };
        let lower = word.to_lowercase();
        let mut analyses = Vec::new();
        for &(index, share) in indexes {
            let found = self.guess_in(index, word, &lower);
            let total: f64 = found.iter().map(|(_, count)| f64::from(*count)).sum();
            analyses.extend(found.into_iter().map(|(analysis, count)| Analysis {
                prob: share * f64::from(count) / total,
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
    /// с числом словоформ словаря; словоформа разбора - как в тексте, лемма - в нижнем регистре
    fn guess_in(&self, index: GuessIndex, word: &str, lower: &str) -> Vec<(Analysis, u32)> {
        let bounds: Vec<usize> = lower.char_indices().map(|(at, _)| at).collect();
        for length in (1..=GuessIndex::MAX_ENDING.min(bounds.len().saturating_sub(1))).rev() {
            let ending = &lower[bounds[bounds.len() - length]..];
            let Some(forms) = self
                .guesses
                .get(index.key(ending))
                .and_then(|offset| self.guess_records_at(offset))
            else {
                continue;
            };
            let found: Vec<(Analysis, u32)> = forms
                .iter()
                .filter_map(|&[packed, count]| {
                    let (table, form) = ((packed >> 16) as u16, packed as u16);
                    let lexeme = self.lexeme(table, form, lower)?;
                    // Основа слова как в тексте нужна, чтобы формы сохраняли заглавную букву
                    self.lexeme(table, form, word)?;
                    let analysis = Analysis {
                        word: word.to_owned(),
                        lemma: self.spell(&lexeme, lexeme.lemma_form(form)),
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

    /// Список форм для угадывания: пары `(таблица << 16) | форма`, число словоформ
    fn guess_records_at(&self, offset: u64) -> Option<&[[u32; 2]]> {
        let start = usize::try_from(offset).ok()?;
        let count = *self.guess_records.get(start)? as usize;
        let end = count.checked_mul(2)?.checked_add(start)?.checked_add(1)?;
        let (pairs, rest) = self.guess_records.get(start + 1..end)?.as_chunks::<2>();
        rest.is_empty().then_some(pairs)
    }
}
