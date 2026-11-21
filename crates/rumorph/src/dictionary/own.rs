//! Свои слова: слово изменяется по образцу словарного слова

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use fst::{Map, MapBuilder};

use super::rules::{blank, token};
use super::{Analysis, Dictionary, Entry, Source, normalized_input};
use crate::Error;
use crate::alphabet::{Key, MAX_KEY};

/// Образец своего слова: словарное слово в той же форме, что и свое слово (обычно в начальной),
/// и часть речи UD, если у образца есть лексемы разных частей речи («печь» - NOUN и VERB)
///
/// Создается из слова (`"геймер".into()`) или пары «слово, часть речи» (`("печь", "VERB").into()`)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Sample<'a> {
    /// Словарное слово
    pub word: &'a str,
    /// Часть речи UD (`"VERB"`): нужна, если у образца лексемы разных частей речи
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

/// Свои слова: лексемы своих основ с формами образцов, найденные по ключам форм. Ключ формы -
/// ключи приставки образца, основы и окончания образца: написания и ключи форм хранит только
/// автомат ключей
#[derive(Clone, Default)]
pub(super) struct OwnWords {
    /// Ключи приставок и окончаний образцов с местами «ё», каждый один раз
    affixes: Vec<Affix>,
    /// Формы лексем образцов: одни на все свои слова образца
    samples: Vec<SampleForms>,
    /// Ключи основ своих слов подряд
    stems: Vec<u8>,
    /// Лексемы своих слов в порядке добавления
    lexemes: Vec<OwnLexeme>,
    /// Ключ каждой формы своего слова: номер лексемы или, с `GROUP`, место группы в `groups`
    keys: Map<Vec<u8>>,
    /// Лексемы с формами одного ключа в порядке добавления: длина группы и номера лексем
    groups: Vec<u32>,
}

/// Ключ приставки или окончания образца и места «ё» в нем
type Affix = (Box<[u8]>, u64);

/// Бит значения `OwnWords::keys`: значение - место группы лексем
const GROUP: u64 = 1 << 32;

/// Формы лексемы образца, которые берут свои слова
#[derive(Clone)]
struct SampleForms {
    table: u16,
    /// Основа образца: с таблицей и формами отличает один образец от другого
    stem: Box<str>,
    forms: Box<[SampleForm]>,
}

/// Форма образца: номер в таблице, номера ключей ее приставки и окончания в `OwnWords::affixes`
/// и их общая длина: формы другой длины отбрасываются без ключей
#[derive(Clone, Copy)]
struct SampleForm {
    form: u16,
    len: u8,
    prefix: u32,
    suffix: u32,
}

/// Лексема своего слова: места «ё», начало и длина ключа основы в `OwnWords::stems` и номер образца
#[derive(Clone)]
struct OwnLexeme {
    yo: u64,
    stem: u32,
    sample: u32,
    len: u8,
}

/// Форма своего слова с ключом: места «ё» (с ключом дают написание) и форма таблицы
struct OwnForm {
    yo: u64,
    entry: Entry,
}

/// Лексема своего слова, как ее находит `sample_forms`: основа слова, таблица и основа образца,
/// формы таблицы с номерами их префиксов и суффиксов
struct OwnSpec {
    stem: String,
    table: u16,
    like: String,
    forms: Vec<(u16, usize, usize)>,
}

impl OwnWords {
    pub(super) fn is_empty(&self) -> bool {
        self.lexemes.is_empty()
    }

    /// Ключ основы лексемы
    fn stem(&self, lexeme: &OwnLexeme) -> &[u8] {
        let start = lexeme.stem as usize;
        (self.stems.get(start..start + usize::from(lexeme.len))).unwrap_or_default()
    }

    /// Ключи приставки и окончания формы образца с их местами «ё»
    fn affixes(&self, form: &SampleForm) -> Option<(&Affix, &Affix)> {
        Some((
            self.affixes.get(form.prefix as usize)?,
            self.affixes.get(form.suffix as usize)?,
        ))
    }

    /// Формы лексемы `number` с ключом `bytes`: места «ё» и форма таблицы каждой
    fn lexeme_forms<'a>(
        &'a self,
        number: u32,
        bytes: &'a [u8],
    ) -> impl Iterator<Item = OwnForm> + 'a {
        let lexeme = self.lexemes.get(number as usize);
        let sample = lexeme.and_then(|lexeme| self.samples.get(lexeme.sample as usize));
        let stem = lexeme.map_or(&[][..], |lexeme| self.stem(lexeme));
        let len = bytes.len().checked_sub(stem.len());
        (sample.into_iter().flat_map(|sample| sample.forms.iter()))
            .filter(move |form| len == Some(usize::from(form.len)))
            .filter_map(move |form| {
                let (lexeme, sample) = (lexeme?, sample?);
                let ((prefix, prefix_yo), (suffix, suffix_yo)) = self.affixes(form)?;
                let rest = bytes.strip_prefix(&**prefix)?.strip_prefix(stem)?;
                (*rest == **suffix).then(|| OwnForm {
                    yo: prefix_yo
                        | lexeme.yo << prefix.len()
                        | suffix_yo << (prefix.len() + stem.len()),
                    entry: Entry {
                        table: sample.table,
                        form: form.form,
                    },
                })
            })
    }

    /// Лексемы с формами ключа `bytes` в порядке добавления
    fn lexemes_of(&self, bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
        let (single, group) = match self.keys.get(bytes) {
            None => (None, &[][..]),
            Some(value) if value & GROUP == 0 => (Some(value as u32), &[][..]),
            Some(value) => {
                let at = (value & !GROUP) as usize;
                let len = self.groups.get(at).map_or(0, |&len| len as usize);
                (
                    None,
                    self.groups.get(at + 1..at + 1 + len).unwrap_or_default(),
                )
            }
        };
        single.into_iter().chain(group.iter().copied())
    }

    /// Формы своих слов с ключом `bytes`; форма двух лексем с одной таблицей и теми же местами «ё» -
    /// один раз, от лексемы, добавленной раньше
    fn forms(&self, bytes: &[u8]) -> Vec<OwnForm> {
        let mut found: Vec<OwnForm> = Vec::new();
        for number in self.lexemes_of(bytes) {
            for own in self.lexeme_forms(number, bytes) {
                if !(found.iter()).any(|known| known.entry == own.entry && known.yo == own.yo) {
                    found.push(own);
                }
            }
        }
        found
    }

    /// Строит автомат ключей форм всех лексем
    fn index(&mut self) -> Result<(), Error> {
        // Ключи - одним буфером, у каждого длина и байты, - с их местами и лексемами: буферы
        // занимают больше всего памяти, поэтому их размер считается заранее
        let forms = |lexeme: &OwnLexeme| {
            let sample = self.samples.get(lexeme.sample as usize);
            (sample.into_iter().flat_map(|sample| sample.forms.iter()))
                .filter_map(|form| self.affixes(form))
        };
        let (count, size) = (self.lexemes.iter()).fold((0, 0), |(count, size), lexeme| {
            forms(lexeme).fold(
                (count, size),
                |(count, size), ((prefix, _), (suffix, _))| {
                    (
                        count + 1,
                        size + 1 + prefix.len() + usize::from(lexeme.len) + suffix.len(),
                    )
                },
            )
        });
        if u32::try_from(size).is_err() {
            return Err(Error::Word("У своих слов слишком много форм".into()));
        }
        let mut bytes = Vec::with_capacity(size);
        let mut keys: Vec<(u32, u32)> = Vec::with_capacity(count);
        for (number, lexeme) in self.lexemes.iter().enumerate() {
            for ((prefix, _), (suffix, _)) in forms(lexeme) {
                keys.push((bytes.len() as u32, number as u32));
                bytes.push((prefix.len() + usize::from(lexeme.len) + suffix.len()) as u8);
                bytes.extend_from_slice(prefix);
                bytes.extend_from_slice(self.stem(lexeme));
                bytes.extend_from_slice(suffix);
            }
        }
        let key = |&(start, _): &(u32, u32)| {
            let start = start as usize;
            &bytes[start + 1..start + 1 + usize::from(bytes[start])]
        };
        keys.sort_unstable_by(|a, b| key(a).cmp(key(b)).then(a.1.cmp(&b.1)));
        keys.dedup_by(|later, first| later.1 == first.1 && key(later) == key(first));
        let mut builder = MapBuilder::memory();
        let mut groups: Vec<u32> = Vec::new();
        for run in keys.chunk_by(|a, b| key(a) == key(b)) {
            let value = match run {
                [(_, lexeme)] => u64::from(*lexeme),
                _ => {
                    let at = groups.len() as u64;
                    groups.push(run.len() as u32);
                    groups.extend(run.iter().map(|&(_, lexeme)| lexeme));
                    GROUP | at
                }
            };
            (builder.insert(key(&run[0]), value))
                .map_err(|error| Error::Word(format!("Ключи своих слов: {error}")))?;
        }
        self.keys = builder.into_map();
        self.groups = groups;
        // Векторы росли удвоением, а анализатор держит их, пока жив
        self.affixes.shrink_to_fit();
        self.samples.shrink_to_fit();
        self.stems.shrink_to_fit();
        self.lexemes.shrink_to_fit();
        self.groups.shrink_to_fit();
        Ok(())
    }
}

impl Dictionary {
    /// Словарь со своими словами: слово получает формы лексем образца со своей основой («вайбкодер»
    /// по образцу «геймер» - «вайбкодера», «вайбкодерами»), разборы как у словарных слов. Слово и образец
    /// приводятся, как запрос `analyze`. Если образец - начальная форма, берутся лексемы с этой
    /// леммой; лексемы одной части речи берутся все. Образцом может быть и свое слово, добавленное
    /// раньше. Словарь со своими словами и без них - `dictionary.clone().with_words(...)`: копия
    /// делит отображенный файл. Каждый вызов строит поиск всех своих слов заново: много слов -
    /// одним вызовом
    ///
    /// ```
    /// # #[cfg(feature = "bundled")] {
    /// use rumorph::Dictionary;
    ///
    /// let dictionary = Dictionary::bundled()?.with_words([("вайбкодер", "геймер")])?;
    /// assert_eq!(dictionary.analyze("вайбкодерами")[0].lemma, "вайбкодер");
    /// # }
    /// # Ok::<(), rumorph::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// `Error::Word`, если образца нет в словаре, у него лексемы разных частей речи, а часть речи
    /// не задана, слово не изменяется по образцу (у образца другое окончание или у слова нет
    /// основы), слово пустое или в нем пробел
    pub fn with_words<'a, S: Into<Sample<'a>>>(
        mut self,
        words: impl IntoIterator<Item = (&'a str, S)>,
    ) -> Result<Self, Error> {
        // Ключи аффиксов, образцы и лексемы, которые уже есть: слова одного образца делят их,
        // а лексема, добавленная снова, не повторяется
        let own = &self.own_words;
        let mut affixes: HashMap<Affix, u32> = (own.affixes.iter().enumerate())
            .map(|(number, affix)| (affix.clone(), number as u32))
            .collect();
        let mut samples: HashMap<(u16, Box<str>, Vec<u16>), u32> = (own.samples.iter())
            .enumerate()
            .map(|(number, sample)| {
                let forms = sample.forms.iter().map(|form| form.form).collect();
                ((sample.table, sample.stem.clone(), forms), number as u32)
            })
            .collect();
        let mut lexemes: HashSet<(Box<[u8]>, u64, u32)> = (own.lexemes.iter())
            .map(|lexeme| (own.stem(lexeme).into(), lexeme.yo, lexeme.sample))
            .collect();
        let mut prefixes: HashSet<u32> = (own.samples.iter())
            .flat_map(|sample| sample.forms.iter().map(|form| form.prefix))
            .collect();
        // Лексемы вызова, которых еще нет в поиске, по ключам основ: если образец может быть
        // формой одной из них, поиск строится заново до него
        let mut fresh: HashMap<Box<[u8]>, Vec<u32>> = HashMap::new();
        for (word, sample) in words {
            let sample = sample.into();
            if !fresh.is_empty() && self.fresh_sample(sample.word, &fresh, &prefixes) {
                Arc::make_mut(&mut self.own_words).index()?;
                fresh.clear();
            }
            for spec in self.sample_forms(word, sample)? {
                let forms: Vec<u16> = spec.forms.iter().map(|&(form, ..)| form).collect();
                let id = (spec.table, Box::from(spec.like.as_str()), forms);
                let sample = match samples.get(&id) {
                    Some(&number) => number,
                    None => {
                        let keys: Vec<(u16, Key, Key)> = (spec.forms.iter())
                            .filter_map(|&(form, prefix, suffix)| {
                                Some((
                                    form,
                                    self.alphabet.input_key(self.prefixes.at(prefix))?,
                                    self.alphabet.input_key(self.suffixes.at(suffix))?,
                                ))
                            })
                            .collect();
                        let own = Arc::make_mut(&mut self.own_words);
                        let mut number = |key: &Key| {
                            let affix = (Box::<[u8]>::from(key.bytes()), key.yo);
                            *affixes.entry(affix).or_insert_with_key(|affix| {
                                own.affixes.push(affix.clone());
                                (own.affixes.len() - 1) as u32
                            })
                        };
                        let forms: Box<[SampleForm]> = (keys.iter())
                            .map(|(form, prefix, suffix)| SampleForm {
                                form: *form,
                                len: (prefix.bytes().len() + suffix.bytes().len()) as u8,
                                prefix: number(prefix),
                                suffix: number(suffix),
                            })
                            .collect();
                        prefixes.extend(forms.iter().map(|form| form.prefix));
                        own.samples.push(SampleForms {
                            table: spec.table,
                            stem: id.1.clone(),
                            forms,
                        });
                        let number = (own.samples.len() - 1) as u32;
                        samples.insert(id, number);
                        number
                    }
                };
                let spell = |&(_, prefix, suffix): &(u16, usize, usize)| {
                    [
                        self.prefixes.at(prefix),
                        &spec.stem,
                        self.suffixes.at(suffix),
                    ]
                    .concat()
                };
                // Буквы, которых нет в словаре, получают свободные байты алфавита копии
                if !self.alphabet.extend(&spec.stem) {
                    return Err(Error::Word(format!(
                        "В слове «{}» слишком много букв, которых нет в словаре",
                        spec.forms.first().map(spell).unwrap_or_default()
                    )));
                }
                let stem = self.alphabet.input_key(&spec.stem);
                let long = (spec.forms.iter()).find(|form| {
                    let (prefix, suffix) = (self.prefixes.at(form.1), self.suffixes.at(form.2));
                    stem.as_ref().is_none_or(|stem| {
                        stem.bytes().len() + prefix.chars().count() + suffix.chars().count()
                            > MAX_KEY
                    })
                });
                if let Some(form) = long {
                    return Err(Error::Word(format!(
                        "Слово «{}» длиннее {MAX_KEY} букв",
                        spell(form)
                    )));
                }
                let Some(key) = stem else {
                    continue;
                };
                if lexemes.insert((key.bytes().into(), key.yo, sample)) {
                    let own = Arc::make_mut(&mut self.own_words);
                    let number = own.lexemes.len() as u32;
                    fresh.entry(key.bytes().into()).or_default().push(number);
                    own.lexemes.push(OwnLexeme {
                        yo: key.yo,
                        stem: own.stems.len() as u32,
                        sample,
                        len: key.bytes().len() as u8,
                    });
                    own.stems.extend_from_slice(key.bytes());
                }
            }
        }
        Arc::make_mut(&mut self.own_words).index()?;
        Ok(self)
    }

    /// Может ли образец `sample` быть формой лексемы вызова, которой еще нет в поиске (`fresh` -
    /// по ключам основ): после приставки образца в ключе - основа такой лексемы и окончание ее формы
    fn fresh_sample(
        &self,
        sample: &str,
        fresh: &HashMap<Box<[u8]>, Vec<u32>>,
        prefixes: &HashSet<u32>,
    ) -> bool {
        let like = normalized_input(sample)
            .unwrap_or_else(|| sample.to_owned())
            .to_lowercase();
        let Some(key) = self.alphabet.input_key(&like) else {
            return false;
        };
        let bytes = key.bytes();
        let own = &self.own_words;
        (prefixes.iter())
            .filter_map(|&prefix| own.affixes.get(prefix as usize))
            .filter_map(|(prefix, _)| bytes.strip_prefix(&**prefix))
            .any(|rest| {
                (1..=rest.len()).any(|len| {
                    (fresh.get(&rest[..len])).is_some_and(|lexemes| {
                        (lexemes.iter())
                            .any(|&lexeme| own.lexeme_forms(lexeme, bytes).next().is_some())
                    })
                })
            })
    }

    /// Лексемы слова `word` по образцу `sample`: основа слова, таблица и основа образца и формы
    /// таблицы, на которые у образца есть запись (копии форм - «ивановы» в женской лексеме -
    /// не берутся)
    fn sample_forms(&self, word: &str, sample: Sample<'_>) -> Result<Vec<OwnSpec>, Error> {
        // Слово и образец приводятся, как запрос `analyze`: без ударений, с кириллицей вместо
        // латинских двойников
        let normal = |text: &str| {
            normalized_input(text)
                .as_deref()
                .unwrap_or(text)
                .to_lowercase()
        };
        let (word, like) = (normal(word), normal(sample.word));
        if blank(&word) {
            return Err(Error::Word("Свое слово пустое".into()));
        }
        let chars: Vec<char> = word.chars().collect();
        // Невидимые символы в слове с видимыми `normal` убрал: остается пробел
        if !token(&chars) {
            return Err(Error::Word(format!(
                "Свое слово «{}» с пробелом не найдется в тексте",
                word.escape_debug()
            )));
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
        let mut specs: Vec<OwnSpec> = Vec::new();
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
            let forms: Vec<(u16, usize, usize)> = (0..lexeme.cells.len() as u16)
                .filter(|&form| {
                    let entry = Entry {
                        table: entry.table,
                        form,
                    };
                    self.has_record(&self.spell(&sample_lexeme, form), entry)
                })
                .map(|form| {
                    let (prefix, suffix) = lexeme.affixes(form);
                    (form, prefix, suffix)
                })
                .collect();
            if !forms.is_empty() {
                specs.push(OwnSpec {
                    stem: stem.to_owned(),
                    table: entry.table,
                    like: sample_lexeme.stem.to_owned(),
                    forms,
                });
            }
        }
        if specs.is_empty() {
            return Err(Error::Word(format!(
                "«{word}» не изменяется по образцу «{like}»: у слова и образца должны быть одна \
                 форма и одно окончание, а у слова - основа"
            )));
        }
        Ok(specs)
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
        let core = |ud: u16| self.ud_core.get(usize::from(ud)).copied();
        for own in self.own_words.forms(key.bytes()) {
            // Место «ё» запроса - «ё» и у своего слова, а «е» запроса может обозначать «ё»
            if own.yo & key.yo != key.yo {
                continue;
            }
            let entry = own.entry;
            let spelling = self.alphabet.decode(key.bytes(), own.yo);
            let Some(lexeme) = self.lexeme(entry.table, entry.form, &spelling) else {
                continue;
            };
            let (lemma, ud) = (self.lemma(&lexeme, entry.form), lexeme.ud(entry.form));
            let known = analyses.iter().any(|analysis| {
                analysis.word == spelling
                    && analysis.lemma == lemma
                    && core(analysis.ud) == core(ud)
            });
            if known {
                continue;
            }
            let tag = lexeme.tag(entry.form);
            analyses.push(Analysis {
                word: spelling,
                lemma,
                ud,
                tag: Some(tag),
                prob: 0.0,
                source: Source::Dictionary,
                entry: Some(entry),
            });
        }
    }

    /// Есть ли свое слово с ключом `key`
    pub(super) fn is_own(&self, key: &Key) -> bool {
        let (own, bytes) = (&self.own_words, key.bytes());
        (own.lexemes_of(bytes))
            .flat_map(|number| own.lexeme_forms(number, bytes))
            .any(|own| own.yo & key.yo == key.yo)
    }

    /// Формы таблиц своего слова с ключом `key` и точно теми же местами «ё»
    pub(super) fn own_entries(&self, key: &Key) -> impl Iterator<Item = Entry> + use<> {
        let yo = key.yo;
        (self.own_words.forms(key.bytes()).into_iter())
            .filter(move |own| own.yo == yo)
            .map(|own| own.entry)
    }
}
