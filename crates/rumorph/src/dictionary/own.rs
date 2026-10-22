//! Свои слова: слово изменяется по образцу словарного слова

use super::{Analysis, Dictionary, Entry, Source};
use crate::Error;

/// Образец своего слова: словарное слово в той же форме, что и свое слово (обычно в начальной),
/// и часть речи UD, если у образца есть лексемы разных частей речи («печь» - NOUN и VERB)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample<'a> {
    pub word: &'a str,
    pub pos: Option<&'a str>,
}

impl<'a> From<&'a str> for Sample<'a> {
    fn from(word: &'a str) -> Self {
        Sample { word, pos: None }
    }
}

impl<'a> From<(&'a str, &'a str)> for Sample<'a> {
    fn from((word, pos): (&'a str, &'a str)) -> Self {
        Sample {
            word,
            pos: Some(pos),
        }
    }
}

impl Dictionary {
    /// Словарь со своими словами: слово получает формы лексем образца со своей основой («зумер»
    /// по образцу «геймер» - «зумера», «зумерами»), разборы как у словарных слов. Если образец -
    /// начальная форма, берутся лексемы с этой леммой; лексемы одной части речи берутся все.
    /// Образцом может быть и свое слово, добавленное раньше. Словарь со своими словами
    /// и без них - `dictionary.clone().with_words(...)`: копия делит отображенный файл
    ///
    /// ```
    /// # #[cfg(feature = "bundled")] {
    /// use rumorph::Dictionary;
    ///
    /// let dictionary = Dictionary::bundled()?.with_words([("зумер", "геймер")])?;
    /// assert_eq!(dictionary.analyze("зумерами")[0].lemma, "зумер");
    /// # }
    /// # Ok::<(), rumorph::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// `Error::Word`, если образца нет в словаре, у него лексемы разных частей речи, а часть речи
    /// не задана, или слово не изменяется по образцу: у образца другое окончание
    pub fn with_words<'a, S: Into<Sample<'a>>>(
        mut self,
        words: impl IntoIterator<Item = (&'a str, S)>,
    ) -> Result<Self, Error> {
        for (word, sample) in words {
            for (spelling, entry) in self.sample_forms(word, sample.into())? {
                let forms = self
                    .own_words
                    .entry(spelling.replace('ё', "е"))
                    .or_default();
                if !forms.contains(&(spelling.clone(), entry)) {
                    forms.push((spelling, entry));
                }
            }
        }
        Ok(self)
    }

    /// Формы слова `word` по образцу `sample`: написание и форма таблицы каждой формы
    fn sample_forms(&self, word: &str, sample: Sample<'_>) -> Result<Vec<(String, Entry)>, Error> {
        let (word, like) = (word.to_lowercase(), sample.word.to_lowercase());
        let pos = |analysis: &Analysis| self.ud_tag(analysis.ud).map_or("", |tag| tag.pos());
        let mut analyses: Vec<Analysis> = self
            .candidates(&like)
            .into_iter()
            .filter(|analysis| analysis.entry.is_some())
            .collect();
        if analyses.is_empty() {
            return Err(Error::Word(format!("Образца «{like}» нет в словаре")));
        }
        // Образец в начальной форме - лексемы с этой леммой: у «белка» нет лексемы «белок»
        let lemma =
            |analysis: &Analysis| analysis.lemma.replace('ё', "е") == like.replace('ё', "е");
        if analyses.iter().any(lemma) {
            analyses.retain(lemma);
        }
        let mut found: Vec<&str> = analyses.iter().map(pos).collect();
        found.sort_unstable();
        found.dedup();
        match sample.pos {
            Some(wanted) => {
                analyses.retain(|analysis| pos(analysis) == wanted);
                if analyses.is_empty() {
                    return Err(Error::Word(format!(
                        "У образца «{like}» нет лексем {wanted}, есть {}",
                        found.join(", ")
                    )));
                }
            }
            None if found.len() > 1 => {
                return Err(Error::Word(format!(
                    "У образца «{like}» лексемы разных частей речи ({}): укажите часть речи",
                    found.join(", ")
                )));
            }
            None => {}
        }
        let mut lexemes: Vec<(u16, String)> = Vec::new();
        for analysis in &analyses {
            let Some(entry) = analysis.entry else {
                continue;
            };
            let Some(lexeme) = self.lexeme(entry.table, entry.form, &analysis.word) else {
                continue;
            };
            let (prefix, suffix) = lexeme.affixes(entry.form);
            let stem = word
                .strip_prefix(self.prefixes.at(prefix))
                .and_then(|rest| rest.strip_suffix(self.suffixes.at(suffix)));
            if let Some(stem) = stem
                && !lexemes.contains(&(entry.table, stem.to_owned()))
            {
                lexemes.push((entry.table, stem.to_owned()));
            }
        }
        if lexemes.is_empty() {
            return Err(Error::Word(format!(
                "«{word}» не изменяется по образцу «{like}»: у слова и образца должны быть одна \
                 форма и одно окончание"
            )));
        }
        let mut forms = Vec::new();
        for (table, stem) in &lexemes {
            let Some(lexeme) = self.table_lexeme(*table, stem) else {
                continue;
            };
            for form in 0..lexeme.cells.len() as u16 {
                forms.push((
                    self.spell(&lexeme, form),
                    Entry {
                        table: *table,
                        form,
                    },
                ));
            }
        }
        Ok(forms)
    }

    /// Разборы своих слов с написанием `word` - без частот
    pub(super) fn own_candidates(&self, word: &str, analyses: &mut Vec<Analysis>) {
        let lower = word.to_lowercase();
        let Some(forms) = self.own_forms(&lower) else {
            return;
        };
        for (spelling, entry) in forms {
            if !same_spelling(&lower, spelling) {
                continue;
            }
            let Some(lexeme) = self.lexeme(entry.table, entry.form, spelling) else {
                continue;
            };
            analyses.push(Analysis {
                word: spelling.clone(),
                lemma: self.lemma(&lexeme, entry.form),
                ud: lexeme.ud(entry.form),
                tag: Some(lexeme.tag(entry.form)),
                prob: 0.0,
                source: Source::Dictionary,
                entry: Some(*entry),
            });
        }
    }

    /// Формы своих слов, которые пишутся как `lower` с точностью до «ё»
    pub(super) fn own_forms(&self, lower: &str) -> Option<&Vec<(String, Entry)>> {
        self.own_words.get(&lower.replace('ё', "е"))
    }

    /// Есть ли свое слово с написанием `word`
    pub(super) fn is_own(&self, word: &str) -> bool {
        let lower = word.to_lowercase();
        self.own_forms(&lower).is_some_and(|forms| {
            forms
                .iter()
                .any(|(spelling, _)| same_spelling(&lower, spelling))
        })
    }
}

/// Написание словаря подходит к запросу: «е» запроса может обозначать «ё», а «ё» - только «ё»
fn same_spelling(input: &str, spelling: &str) -> bool {
    input.chars().count() == spelling.chars().count()
        && input
            .chars()
            .zip(spelling.chars())
            .all(|(own, known)| own == known || (own == 'е' && known == 'ё'))
}
