//! Свои слова: слово изменяется по образцу словарного слова

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use super::{Analysis, Dictionary, Entry, Source};
use crate::Error;
use crate::alphabet::Key;

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

/// Свои слова по байтам ключа автомата
pub(super) type OwnWords = HashMap<Box<[u8]>, Vec<OwnForm>, BuildHasherDefault<KeyHasher>>;

/// Форма своего слова: места «ё» в ключе, написание и форма таблицы
#[derive(Clone, PartialEq)]
pub(super) struct OwnForm {
    yo: u64,
    spelling: String,
    entry: Entry,
}

/// Хеш ключей своих слов: ищутся на каждом токене, поэтому быстрее SipHash; защита от подобранных
/// ключей не нужна - слова задает владелец словаря
#[derive(Clone, Default)]
pub(super) struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(byte)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    fn finish(&self) -> u64 {
        self.0
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
    /// не задана, или слово не изменяется по образцу: у образца другое окончание или у слова нет
    /// основы
    pub fn with_words<'a, S: Into<Sample<'a>>>(
        mut self,
        words: impl IntoIterator<Item = (&'a str, S)>,
    ) -> Result<Self, Error> {
        for (word, sample) in words {
            for (spelling, entry) in self.sample_forms(word, sample.into())? {
                // Буквы, которых нет в словаре, получают свободные байты алфавита копии
                if !self.alphabet.extend(&spelling) {
                    return Err(Error::Word(format!(
                        "В слове «{spelling}» слишком много букв, которых нет в словаре"
                    )));
                }
                let Some(key) = self.alphabet.input_key(&spelling) else {
                    return Err(Error::Word(format!(
                        "Слово «{spelling}» длиннее {} букв",
                        crate::alphabet::MAX_KEY
                    )));
                };
                let form = OwnForm {
                    yo: key.yo,
                    spelling,
                    entry,
                };
                let forms = self.own_words.entry(key.bytes().into()).or_default();
                if !forms.contains(&form) {
                    forms.push(form);
                }
            }
        }
        Ok(self)
    }

    /// Формы слова `word` по образцу `sample`: написание и форма таблицы каждой формы, на которую
    /// у образца есть запись (копии форм - «ивановы» в женской лексеме - не берутся)
    fn sample_forms(&self, word: &str, sample: Sample<'_>) -> Result<Vec<(String, Entry)>, Error> {
        let (word, like) = (word.to_lowercase(), sample.word.to_lowercase());
        if word.trim().is_empty() {
            return Err(Error::Word("Свое слово пустое".into()));
        }
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
        let mut forms = Vec::new();
        let mut lexemes: Vec<(u16, &str)> = Vec::new();
        for analysis in &analyses {
            let Some(entry) = analysis.entry else {
                continue;
            };
            let Some(sample_lexeme) = self.lexeme(entry.table, entry.form, &analysis.word) else {
                continue;
            };
            let (prefix, suffix) = sample_lexeme.affixes(entry.form);
            let Some(stem) = word
                .strip_prefix(self.prefixes.at(prefix))
                .and_then(|rest| rest.strip_suffix(self.suffixes.at(suffix)))
                .filter(|stem| !stem.is_empty())
            else {
                continue;
            };
            if lexemes.contains(&(entry.table, stem)) {
                continue;
            }
            lexemes.push((entry.table, stem));
            let Some(lexeme) = self.table_lexeme(entry.table, stem) else {
                continue;
            };
            for form in 0..lexeme.cells.len() as u16 {
                let entry = Entry {
                    table: entry.table,
                    form,
                };
                if self.has_record(&self.spell(&sample_lexeme, form), entry) {
                    forms.push((self.spell(&lexeme, form), entry));
                }
            }
        }
        if forms.is_empty() {
            return Err(Error::Word(format!(
                "«{word}» не изменяется по образцу «{like}»: у слова и образца должны быть одна \
                 форма и одно окончание, а у слова - основа"
            )));
        }
        Ok(forms)
    }

    /// Есть ли у написания `spelling` запись формы `entry` в автомате или в своих словах
    fn has_record(&self, spelling: &str, entry: Entry) -> bool {
        let Some(key) = self.alphabet.input_key(spelling) else {
            return false;
        };
        let packed = (u32::from(entry.table) << 16) | u32::from(entry.form);
        let recorded = self.spellings(&key).is_some_and(|spellings| {
            spellings.variants().any(|(yo, records)| {
                yo == key.yo
                    && self
                        .records_at(records)
                        .is_some_and(|records| records.contains(&packed))
            })
        });
        recorded || self.own_entries(&key).any(|own| own == entry)
    }

    /// Разборы своих слов с ключом `key`, кроме совпавших с уже собранными разборами `analyses`
    /// по написанию, лемме, части речи и признакам (словарное слово или слово по двум образцам
    /// не получает лишней доли); без частот
    pub(super) fn own_candidates(&self, key: &Key, analyses: &mut Vec<Analysis>) {
        let Some(forms) = self.own_words.get(key.bytes()) else {
            return;
        };
        let core = |ud: u16| self.ud_core.get(usize::from(ud)).copied();
        for own in forms {
            // Место «ё» запроса - «ё» и у своего слова, а «е» запроса может обозначать «ё»
            if own.yo & key.yo != key.yo {
                continue;
            }
            let entry = own.entry;
            let Some(lexeme) = self.lexeme(entry.table, entry.form, &own.spelling) else {
                continue;
            };
            let (lemma, ud) = (self.lemma(&lexeme, entry.form), lexeme.ud(entry.form));
            let known = analyses.iter().any(|analysis| {
                analysis.word == own.spelling
                    && analysis.lemma == lemma
                    && core(analysis.ud) == core(ud)
            });
            if known {
                continue;
            }
            analyses.push(Analysis {
                word: own.spelling.clone(),
                lemma,
                ud,
                tag: Some(lexeme.tag(entry.form)),
                prob: 0.0,
                source: Source::Dictionary,
                entry: Some(entry),
            });
        }
    }

    /// Есть ли свое слово с ключом `key`
    pub(super) fn is_own(&self, key: &Key) -> bool {
        self.own_words
            .get(key.bytes())
            .is_some_and(|forms| forms.iter().any(|own| own.yo & key.yo == key.yo))
    }

    /// Формы таблиц своего слова с ключом `key` и точно теми же местами «ё»
    pub(super) fn own_entries<'a>(&'a self, key: &'a Key) -> impl Iterator<Item = Entry> + 'a {
        (self.own_words.get(key.bytes()).into_iter().flatten())
            .filter(|own| own.yo == key.yo)
            .map(|own| own.entry)
    }
}
