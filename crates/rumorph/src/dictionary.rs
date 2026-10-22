use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use fst::Map;

use crate::Error;
use crate::alphabet::{Alphabet, Key, yo_order};
use crate::format::{
    self, ENTRY_KEYS, Layout, MAX_LIST, MAX_SPELLINGS, Slice, Storage, Strings, WordValue,
};

mod guess;
mod own;
mod parts;
mod rules;
mod ud;

#[cfg(any(test, feature = "build"))]
pub use guess::GuessIndex;
pub use own::Sample;
use parts::with_particle;
pub use ud::UdTag;
use ud::UdTags;

/// Откуда взят разбор
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
    /// Словоформа есть в словаре
    Dictionary,
    /// Токена нет в словаре, разбор - по правилу: число, знак, латиница, римское число, инициал,
    /// аббревиатура или сокращение с точкой
    Rule,
    /// Слова нет в словаре, разбор угадан: по частям слова через дефис или с приставкой
    /// либо по окончанию
    Guess,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Dictionary => "dictionary",
            Source::Rule => "rule",
            Source::Guess => "guess",
        }
    }
}

/// Разбор словоформы в словаре: форма таблицы словоизменения
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    /// Номер таблицы, который вернул `Builder::add_table`
    pub table: u16,
    /// Номер формы словоформы в парадигме таблицы
    pub form: u16,
}

/// Разбор словоформы
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Analysis {
    /// Словоформа в написании словаря: с буквой «ё», если она есть в словаре
    pub word: String,
    /// Начальная форма по соглашениям UD
    pub lemma: String,
    /// Номер тега UD в `Dictionary::ud_tags`
    pub ud: u16,
    /// Номер тега OpenCorpora в `Dictionary::tags`; у разборов по правилам его может не быть
    pub tag: Option<u16>,
    /// Вероятность разбора; сумма по всем разборам словоформы равна 1
    pub prob: f64,
    pub source: Source,
    /// Форма таблицы словоизменения; у разборов без лексемы (число, знак, латиница) ее нет
    pub entry: Option<Entry>,
}

/// Тег UD для подбора формы: номер строки части речи, коды значений признаков
/// из `Dictionary::features` по порядку (0 - признака нет) и номера строк помет
struct Codes<'a> {
    pos: u16,
    values: &'a [u16],
    labels: &'a [u16],
}

/// Лексема: формы парадигмы с общей основой и их пары «форма леммы, тег UD» из таблицы
struct Lexeme<'a> {
    table: u16,
    /// Номера суффиксов, тегов и префиксов форм тремя блоками
    forms: &'a [u16],
    /// `(форма леммы << 16) | тег UD` каждой формы
    cells: &'a [u32],
    stem: &'a str,
}

impl Lexeme<'_> {
    fn lemma_form(&self, form: u16) -> u16 {
        (self.cells[usize::from(form)] >> 16) as u16
    }

    fn ud(&self, form: u16) -> u16 {
        self.cells[usize::from(form)] as u16
    }

    fn tag(&self, form: u16) -> u16 {
        self.forms[self.cells.len() + usize::from(form)]
    }

    /// Номера префикса и суффикса формы
    fn affixes(&self, form: u16) -> (usize, usize) {
        let form = usize::from(form);
        (
            usize::from(self.forms[2 * self.cells.len() + form]),
            usize::from(self.forms[form]),
        )
    }
}

/// Словарь словоформ
///
/// Файл словаря отображается в память: процессы с одним словарем делят его память, а копия
/// (`clone`) - отображенный файл. Пока словарь открыт, файл нельзя перезаписывать на месте
#[derive(Clone)]
pub struct Dictionary {
    storage: Arc<Storage>,
    layout: Layout,
    meta: String,
    tags: Strings,
    ud: UdTags,
    /// Номер пары «часть речи, признаки» тега UD: разборы, которые отличаются только пометами, сливаются
    ud_core: Vec<u16>,
    /// Коды значений признаков пар `ud_core` подряд, по `features.len()` на пару
    ud_values: Vec<u16>,
    rule_tags: rules::Tags,
    /// Коды признаков и помет, которые `closest` проверяет у каждой формы
    preferences: Preferences,
    /// Код признака NumForm=Combi: порядковые из цифр с окончанием («90-е»)
    combined_numeral: Option<(usize, u16)>,
    /// Тег UD - имя собственное (`PROPN`): по номеру тега
    proper_nouns: Vec<bool>,
    /// Граммемы сокращения-существительного для `ABBREVIATION_LEMMAS`: по номеру тега OpenCorpora
    abbreviation_grammemes: Vec<u8>,
    /// Свои слова (`with_words`) по ключу автомата
    own_words: own::OwnWords,
    features: BTreeMap<String, BTreeSet<String>>,
    alphabet: Alphabet,
    prefixes: Strings,
    suffixes: Strings,
    /// Начало каждой таблицы словоизменения в `tables`
    table_offsets: Vec<u32>,
    words: Map<Slice>,
    word_count: usize,
    guesses: Map<Slice>,
}

impl std::fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dictionary")
            .field("meta", &self.meta)
            .field("word_count", &self.word_count)
            .field("ud_tags", &self.ud.len())
            .finish_non_exhaustive()
    }
}

/// Написания ключа словоформы: места «ё» и начала списков разборов, а также списки частот
/// написаний
enum Spellings<'a> {
    Single {
        yo: u64,
        records: u32,
        probabilities: Option<u32>,
    },
    /// Тройки «места «ё» (два числа), начало списка» из секции `variants`
    Many {
        variants: &'a [[u32; 3]],
        probabilities: &'a [[u32; 3]],
    },
}

impl Spellings<'_> {
    /// Написания: места «ё» и начало списка разборов
    fn variants(&self) -> impl Iterator<Item = (u64, u32)> {
        let (single, many) = match *self {
            Spellings::Single { yo, records, .. } => (Some((yo, records)), &[][..]),
            Spellings::Many { variants, .. } => (None, variants),
        };
        single
            .into_iter()
            .chain(many.iter().map(|&triple| unpack(triple)))
    }

    /// Начало списка частот написания с «ё» на местах `yo`
    fn probabilities(&self, yo: u64) -> Option<u32> {
        match *self {
            Spellings::Single {
                yo: own,
                probabilities,
                ..
            } => probabilities.filter(|_| own == yo),
            Spellings::Many { probabilities, .. } => probabilities
                .iter()
                .map(|&triple| unpack(triple))
                .find(|&(own, _)| own == yo)
                .map(|(_, offset)| offset),
        }
    }
}

fn unpack([low, high, offset]: [u32; 3]) -> (u64, u32) {
    (u64::from(low) | u64::from(high) << 32, offset)
}

/// Список частот: пары «ключ, доля в миллионных», ключи - теги или формы таблиц
#[derive(Clone, Copy)]
struct Probabilities<'a> {
    by_entry: bool,
    pairs: &'a [u32],
}

impl Probabilities<'_> {
    /// Ключ разбора в списке: форма таблицы или тег
    fn key(self, analysis: &Analysis) -> Option<u32> {
        if self.by_entry {
            analysis
                .entry
                .map(|entry| (u32::from(entry.table) << 16) | u32::from(entry.form))
        } else {
            analysis.tag.map(u32::from)
        }
    }
}

/// Написания ключа, которые подходят к запросу: места «ё» и начало списка разборов
struct Matches {
    items: [(u64, u32); MAX_SPELLINGS],
    len: usize,
}

impl Matches {
    fn as_slice(&self) -> &[(u64, u32)] {
        &self.items[..self.len]
    }
}

impl Dictionary {
    /// Читает словарь из файла: обычный файл отображается в память, а канал, устройство и файл,
    /// который не отображается (WASI, часть сетевых файловых систем), читаются
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let file = std::fs::File::open(path)?;
        let metadata = file.metadata()?;
        if metadata.is_file() && metadata.len() > 0 {
            // SAFETY: отображение только читается; перезаписывать файл, пока словарь открыт, нельзя
            // (документация `Dictionary`)
            if let Ok(map) = unsafe { memmap2::Mmap::map(&file) } {
                return Self::from_storage(Storage::Mapped(map));
            }
        }
        Self::from_storage(format::read(file)?)
    }

    /// Читает словарь из байтов файла словаря
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error> {
        Self::from_storage(Storage::copy(data))
    }

    /// Читает словарь из данных программы (`include_bytes!`): байты, выровненные по 8,
    /// читаются без копии
    pub fn from_static(data: &'static [u8]) -> Result<Self, Error> {
        Self::from_storage(Storage::of_static(data))
    }

    /// Словарь OpenCorpora из крейта `rumorph-dict-ru`; словарь проверяется при каждом вызове
    /// (около 3 мс), поэтому его создают один раз и делят между потоками
    ///
    /// ```
    /// let dictionary = rumorph::Dictionary::bundled()?;
    /// assert_eq!(dictionary.analyze("стали")[0].lemma, "стать");
    /// # Ok::<(), rumorph::Error>(())
    /// ```
    #[cfg(feature = "bundled")]
    #[cfg_attr(docsrs, doc(cfg(feature = "bundled")))]
    pub fn bundled() -> Result<Self, Error> {
        if rumorph_dict_ru::DICTIONARY.is_empty() {
            return Err(Error::Format(
                "крейт rumorph-dict-ru собран без файла словаря".into(),
            ));
        }
        Self::from_static(rumorph_dict_ru::DICTIONARY)
    }

    fn from_storage(storage: Storage) -> Result<Self, Error> {
        let layout = format::layout(storage.bytes())?;
        let storage = Arc::new(storage.native(&layout));
        let bytes = |range: &Range<usize>| &storage.bytes()[range.clone()];
        let meta = std::str::from_utf8(bytes(&layout.meta))
            .map_err(|_| Error::Format("строка не в UTF-8".into()))?
            .to_owned();
        let alphabet = std::str::from_utf8(bytes(&layout.alphabet))
            .map_err(|_| Error::Format("строка не в UTF-8".into()))
            .and_then(Alphabet::new)?;
        let mut ud = UdTags::read(
            &Strings::read(bytes(&layout.ud_strings))?,
            &bytes(&layout.ud_tags)
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&pair| u16::from_le_bytes(pair))
                .collect::<Vec<_>>(),
        )?;
        // Таблицы ссылаются только на теги файла, а теги правил добавляются после них
        let ud_count = ud.len();
        let mut dictionary = Dictionary {
            tags: Strings::read(bytes(&layout.tags))?,
            prefixes: Strings::read(bytes(&layout.prefixes))?,
            suffixes: Strings::read(bytes(&layout.suffixes))?,
            words: automaton(&storage, &layout.words)?,
            guesses: automaton(&storage, &layout.guesses)?,
            word_count: 0,
            table_offsets: Vec::new(),
            rule_tags: rules::Tags::add(&mut ud),
            ud,
            ud_core: Vec::new(),
            ud_values: Vec::new(),
            features: BTreeMap::new(),
            preferences: Preferences::default(),
            combined_numeral: None,
            proper_nouns: Vec::new(),
            abbreviation_grammemes: Vec::new(),
            own_words: own::OwnWords::default(),
            meta,
            alphabet,
            layout,
            storage: Arc::clone(&storage),
        };
        if dictionary.ud.len() > usize::from(u16::MAX) + 1 {
            return Err(Error::Format("больше 65 536 тегов UD".into()));
        }
        dictionary.check_paradigms()?;
        dictionary.table_offsets = dictionary.check_tables(ud_count)?;
        dictionary.check_lists()?;
        dictionary.word_count = dictionary.words.len() + dictionary.check_variants()?;
        dictionary.index_ud_tags()?;
        Ok(dictionary)
    }

    /// Слияние тегов по части речи и признакам, коды значений признаков и коды для `closest`
    fn index_ud_tags(&mut self) -> Result<(), Error> {
        let numbers = |number: usize| self.ud.numbers(number as u16).unwrap_or_default();
        let mut cores: HashMap<(u16, &[u16]), u16> = HashMap::new();
        // Признаки первого тега каждой пары «часть речи, признаки»
        let mut core_feats = Vec::new();
        for number in 0..self.ud.len() {
            let (pos, feats, _) = numbers(number);
            let next = cores.len() as u16;
            let core = *cores.entry((pos, feats)).or_insert(next);
            if core == next {
                core_feats.push(feats);
            }
            self.ud_core.push(core);
        }
        let mut pairs: Vec<(u16, u16)> = core_feats
            .iter()
            .flat_map(|feats| feats.as_chunks::<2>().0)
            .map(|&[name, value]| (name, value))
            .collect();
        pairs.sort_unstable();
        pairs.dedup();
        let string = |number: u16| self.ud.string(number);
        for &(name, value) in &pairs {
            self.features
                .entry(string(name).to_owned())
                .or_default()
                .insert(string(value).to_owned());
        }
        // Коды занимают features.len() чисел на пару «часть речи, признаки»: число признаков
        // ограничено, чтобы файл не задавал гигабайты
        if self.features.len() > MAX_FEATURES {
            return Err(Error::Format(format!("больше {MAX_FEATURES} признаков UD")));
        }
        let mut positions: HashMap<(&str, &str), (usize, u16)> = HashMap::new();
        for (index, (name, values)) in self.features.iter().enumerate() {
            for (position, value) in values.iter().enumerate() {
                let code = u16::try_from(position + 1)
                    .map_err(|_| Error::Format("больше 65 535 значений признака UD".into()))?;
                positions.insert((name, value), (index, code));
            }
        }
        let codes: Vec<(usize, u16)> = pairs
            .iter()
            .map(|&(name, value)| {
                positions
                    .get(&(string(name), string(value)))
                    .copied()
                    .unwrap_or_default()
            })
            .collect();
        let count = self.features.len();
        let mut values = vec![0; core_feats.len() * count];
        for (row, feats) in values.chunks_mut(count.max(1)).zip(&core_feats) {
            for &[name, value] in feats.as_chunks::<2>().0 {
                if let Ok(at) = pairs.binary_search(&(name, value)) {
                    let (index, code) = codes[at];
                    row[index] = code;
                }
            }
        }
        self.ud_values = values;
        let code = |name: &str, value: &str| feature_code(&self.features, name, value);
        self.preferences = Preferences {
            infinitive: code("VerbForm", "Inf"),
            finite: code("VerbForm", "Fin"),
            short: code("Variant", "Short"),
            animate: code("Animacy", "Anim"),
            first_person: code("Person", "1"),
            inclusive: self.ud.string_number("incl"),
            counting: self.ud.string_number("Coun"),
        };
        self.combined_numeral = code("NumForm", "Combi");
        self.proper_nouns = self.ud.iter().map(|tag| tag.pos() == "PROPN").collect();
        self.abbreviation_grammemes = self
            .tags
            .iter()
            .map(rules::abbreviation_grammemes)
            .collect();
        Ok(())
    }

    /// Описание источника словаря
    pub fn meta(&self) -> &str {
        &self.meta
    }

    /// Тег OpenCorpora с номером `Analysis::tag`
    pub fn tag(&self, number: u16) -> Option<&str> {
        self.tags.get(usize::from(number))
    }

    /// Теги OpenCorpora по номерам
    pub fn tags(&self) -> impl ExactSizeIterator<Item = &str> {
        self.tags.iter()
    }

    /// Тег UD с номером `Analysis::ud`
    pub fn ud_tag(&self, number: u16) -> Option<UdTag<'_>> {
        self.ud.get(number)
    }

    /// Теги UD по номерам
    pub fn ud_tags(&self) -> impl ExactSizeIterator<Item = UdTag<'_>> {
        self.ud.iter()
    }

    /// Признаки UD, которые встречаются в словаре, и их значения
    pub fn features(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.features
    }

    /// Число словоформ в словаре
    pub fn word_count(&self) -> usize {
        self.word_count
    }

    /// Разборы словоформы по убыванию вероятности; регистр для поиска не важен, «е» может
    /// обозначать «ё»
    ///
    /// Регистр меняет доли имен собственных: у строчного слова они идут последними, у слова
    /// с заглавной буквы их доля выше, чем в частотах словаря.
    /// Разборы OpenCorpora, которые совпали в написании, лемме, части речи и признаках UD,
    /// сливаются в один с суммой вероятностей и пометами более вероятного. Токен, которого нет
    /// в словаре, разбирается по правилам (`Source::Rule`), а незнакомое слово кириллицей
    /// угадывается (`Source::Guess`): слово через дефис - по последней части, слово с известной
    /// приставкой - по слову без нее, остальное - по окончанию. Токен, который не разобрался,
    /// получает запасной разбор `X`; пустой список - только у токена с пробелом, управляющим
    /// или невидимым символом
    pub fn analyze(&self, word: &str) -> Vec<Analysis> {
        if let Some(normal) = normalized_input(word) {
            return self.analyze(&normal);
        }
        let mut analyses = self.merge(self.candidates(word));
        if !analyses.is_empty() {
            self.reweight_numerals(&mut analyses);
            self.weigh_by_case(word, &mut analyses);
            return analyses;
        }
        let analyses = self.rules(word);
        if !analyses.is_empty() {
            return analyses;
        }
        let analyses = self.parts(word);
        if !analyses.is_empty() {
            return analyses;
        }
        let analyses = self.guess(word);
        if !analyses.is_empty() {
            return analyses;
        }
        self.fallback(word)
    }

    /// Разборы словаря до слияния с вероятностями P(тег | словоформа); без частот вероятности нулевые
    fn candidates(&self, word: &str) -> Vec<Analysis> {
        // Своих слов с буквами вне алфавита словаря нет
        let Some(key) = self.alphabet.input_key(word) else {
            return Vec::new();
        };
        let Some(spellings) = self.spellings(&key) else {
            let mut analyses = Vec::new();
            if !self.own_words.is_empty() {
                self.own_candidates(&key, &mut analyses);
            }
            return analyses;
        };
        let matches = self.matches(&spellings, key.yo);
        let mut analyses: Vec<Analysis> = Vec::new();
        // Разборы каждого написания идут подряд: начало, конец и список частот
        let mut groups = [(0, 0, None); MAX_SPELLINGS];
        let input = spellings
            .probabilities(key.yo)
            .and_then(|offset| self.probabilities_at(offset));
        for (group, &(yo, records)) in groups.iter_mut().zip(matches.as_slice()) {
            let spelling = self.alphabet.spelling(&key, yo);
            let start = analyses.len();
            let records = self.records_at(records).unwrap_or_default();
            analyses.reserve(records.len());
            // Формы лексемы идут подряд (у сокращения «в» их 12): одна лемма ищется один раз
            let mut previous = None;
            for &entry in records {
                let (table, form) = ((entry >> 16) as u16, entry as u16);
                let Some(lexeme) = self.lexeme(table, form, &spelling) else {
                    continue;
                };
                let tag = lexeme.tag(form);
                let bits = self.abbreviation_grammemes.get(usize::from(tag)).copied();
                // Основы форм лексемы - один отрезок написания: сравниваются без сравнения строк
                let stem = (lexeme.stem.as_ptr(), lexeme.stem.len());
                let key = Some((table, stem, lexeme.lemma_form(form), bits));
                let lemma = match analyses.last() {
                    Some(last) if previous == key => last.lemma.clone(),
                    _ => self.lemma(&lexeme, form),
                };
                previous = key;
                analyses.push(Analysis {
                    lemma,
                    ud: lexeme.ud(form),
                    tag: Some(tag),
                    word: String::new(),
                    prob: 0.0,
                    source: Source::Dictionary,
                    entry: Some(Entry { table, form }),
                });
            }
            // Написание отдается последнему разбору, остальным - копии
            if let Some((last, rest)) = analyses[start..].split_last_mut() {
                for analysis in rest {
                    analysis.word.clone_from(&spelling);
                }
                last.word = spelling;
            }
            // P(тег | словоформа) берется для написания из запроса, а без частот у него - для
            // написания разбора в словаре («еще» - частоты «ещё»)
            let table = match input {
                Some(table) => Some(table),
                None if yo == key.yo => None,
                None => spellings
                    .probabilities(yo)
                    .and_then(|offset| self.probabilities_at(offset)),
            };
            *group = (start, analyses.len(), table);
        }
        // Свои слова - без частот: у них средняя доля разборов с частотами
        if !self.own_words.is_empty() {
            self.own_candidates(&key, &mut analyses);
        }
        self.assign_probabilities(&groups[..matches.len], input.is_some(), &mut analyses);
        analyses
    }

    /// Множитель долей имен собственных у слова с заглавной буквы: для сборщика словаря, который
    /// учитывает его в долях по корпусу
    #[cfg(feature = "build")]
    #[doc(hidden)]
    pub const TITLECASE_PROPN: f64 = rules::TITLECASE_PROPN;

    /// Разборы словаря до слияния с долями из списков частот (без частот - нулевыми): для сборщика
    /// словаря, который пересчитывает частоты
    #[cfg(feature = "build")]
    #[doc(hidden)]
    pub fn candidate_analyses(&self, word: &str) -> Vec<Analysis> {
        self.candidates(word)
    }

    /// Доли разборов словоформы заданы по формам таблиц (частоты корпуса UD), а не по тегам:
    /// для сверки с pymorphy3
    #[doc(hidden)]
    pub fn table_frequencies(&self, word: &str) -> bool {
        let Some(key) = self.alphabet.input_key(word) else {
            return false;
        };
        let Some(spellings) = self.spellings(&key) else {
            return false;
        };
        let by_entry = |yo: u64| {
            spellings
                .probabilities(yo)
                .and_then(|offset| self.probabilities_at(offset))
                .is_some_and(|list| list.by_entry)
        };
        by_entry(key.yo)
            || self
                .matches(&spellings, key.yo)
                .as_slice()
                .iter()
                .any(|&(yo, _)| by_entry(yo))
    }

    /// Разборы OpenCorpora до перевода в UD и слияния: написание, тег, начальная форма парадигмы
    /// и вероятность (без частот - равная доля); для проверки словаря
    #[doc(hidden)]
    pub fn opencorpora(&self, word: &str) -> Vec<(String, &str, String, f64)> {
        let analyses = self.candidates(word);
        let uniform = analyses.iter().all(|analysis| analysis.prob == 0.0);
        let share = 1.0 / analyses.len() as f64;
        analyses
            .into_iter()
            .filter_map(|analysis| {
                let entry = analysis.entry?;
                let lexeme = self.lexeme(entry.table, entry.form, &analysis.word)?;
                let lemma = self.spell(&lexeme, 0);
                let prob = if uniform { share } else { analysis.prob };
                Some((
                    analysis.word,
                    self.tags.get(usize::from(analysis.tag?))?,
                    lemma,
                    prob,
                ))
            })
            .collect()
    }

    /// Слияние разборов с одинаковыми написанием (с точностью до «е» на месте «ё»), леммой,
    /// частью речи и признаками; у слитого разбора написание с меньшим числом «ё» - точное
    /// написание запроса («недалеко», а не «недалёко»). Без частот каждый слитый разбор получает
    /// равную долю, и порядок словаря сохраняется
    fn merge(&self, mut merged: Vec<Analysis>) -> Vec<Analysis> {
        let yo_count = |word: &str| word.chars().filter(|&ch| matches!(ch, 'ё' | 'Ё')).count();
        let core = |analysis: &Analysis| self.ud_core.get(usize::from(analysis.ud));
        // Слитые разборы собираются в начале списка
        let mut kept = 0;
        for i in 0..merged.len() {
            let analysis = &merged[i];
            let same = (0..kept).find(|&j| {
                let known = &merged[j];
                core(known) == core(analysis)
                    && known.lemma == analysis.lemma
                    && same_up_to_yo(&known.word, &analysis.word)
            });
            match same {
                Some(j) => {
                    // Сумма долей из-за округления бывает чуть больше 1
                    let prob = (merged[j].prob + merged[i].prob).min(1.0);
                    if yo_count(&merged[i].word) < yo_count(&merged[j].word) {
                        merged.swap(i, j);
                    }
                    merged[j].prob = prob;
                }
                None => {
                    merged.swap(kept, i);
                    kept += 1;
                }
            }
        }
        merged.truncate(kept);
        if merged.iter().all(|analysis| analysis.prob == 0.0) {
            let prob = 1.0 / merged.len() as f64;
            for analysis in &mut merged {
                analysis.prob = prob;
            }
        } else {
            merged.sort_by(|a, b| b.prob.total_cmp(&a.prob));
        }
        merged
    }

    /// Разбор словоформы `word` как формы `entry` таблицы словоизменения с источником `source`;
    /// вероятность - 1 (для восстановления сохраненного разбора). Словарный разбор должен быть
    /// в словаре, у угаданного или разобранного по правилу проверяется только, что словоформа
    /// подходит к форме таблицы
    pub fn analysis(&self, word: &str, entry: Entry, source: Source) -> Option<Analysis> {
        // Угаданное слово с частицей («выпивал-то»): форма - у первой части
        if source == Source::Guess
            && let Some((base, particle)) = parts::split_particle(word)
            && self
                .lexeme(entry.table, entry.form, &base.to_lowercase())
                .is_some()
        {
            return self
                .analysis(base, entry, source)
                .map(|analysis| with_particle(analysis, particle));
        }
        // Лемма - в нижнем регистре, как у угадывания; словоформа - как сохранена
        let lower = word.to_lowercase();
        let lexeme = self.lexeme(entry.table, entry.form, &lower)?;
        if source == Source::Dictionary {
            // Запись словоформы ссылается на эту форму или на форму той же таблицы с тем же
            // написанием и тегом: копии форм («ивановы» в женской лексеме) записей не имеют
            let same = |packed: u32| {
                let form = packed as u16;
                (packed >> 16) as u16 == entry.table
                    && (form == entry.form
                        || (lexeme.affixes(form) == lexeme.affixes(entry.form)
                            && lexeme.tag(form) == lexeme.tag(entry.form)))
            };
            let key = self.alphabet.spelling_key(word)?;
            let recorded = (self.spellings(&key))
                .and_then(|spellings| spellings.variants().find(|&(yo, _)| yo == key.yo))
                .and_then(|(_, offset)| self.records_at(offset))
                .is_some_and(|records| records.iter().any(|&packed| same(packed)));
            let own = || {
                self.own_entries(&key)
                    .any(|own| same((u32::from(own.table) << 16) | u32::from(own.form)))
            };
            if !recorded && !own() {
                return None;
            }
        }
        Some(Analysis {
            word: word.to_owned(),
            lemma: self.lemma(&lexeme, entry.form),
            ud: lexeme.ud(entry.form),
            tag: Some(lexeme.tag(entry.form)),
            prob: 1.0,
            source,
            entry: Some(entry),
        })
    }

    /// Формы лексемы разбора в порядке парадигмы: формы с той же леммой, вероятность каждой - 1
    ///
    /// Формы, которые совпали в написании, части речи и признаках, сливаются, как в `analyze`
    pub fn forms(&self, analysis: &Analysis) -> Vec<Analysis> {
        if let Some((base, particle)) = self.without_particle(analysis) {
            return self
                .forms(&base)
                .into_iter()
                .map(|form| with_particle(form, &particle))
                .collect();
        }
        if let Some(lower) = self.lowercased(analysis) {
            return self
                .forms(&lower)
                .into_iter()
                .map(|form| capitalized_like(&analysis.word, form))
                .collect();
        }
        if analysis.entry.is_none() {
            return vec![Analysis {
                prob: 1.0,
                ..analysis.clone()
            }];
        }
        let Some((lexeme, forms)) = self.lexeme_forms(analysis) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        forms
            .into_iter()
            .filter(|&form| {
                seen.insert((
                    lexeme.affixes(form),
                    self.ud_core[usize::from(lexeme.ud(form))],
                ))
            })
            .map(|form| self.form(analysis, &lexeme, form))
            .collect()
    }

    /// Форма лексемы с признаками `feats`; из подходящих - ближайшая к разбору по части речи,
    /// остальным признакам и пометам, при равенстве - первая в парадигме
    pub fn inflect(&self, analysis: &Analysis, feats: &[(&str, &str)]) -> Option<Analysis> {
        if let Some((base, particle)) = self.without_particle(analysis) {
            return self
                .inflect(&base, feats)
                .map(|form| with_particle(form, &particle));
        }
        if let Some(lower) = self.lowercased(analysis) {
            return self
                .inflect(&lower, feats)
                .map(|form| capitalized_like(&analysis.word, form));
        }
        if analysis.entry.is_none() {
            let tag = self.ud.get(analysis.ud)?;
            let own = feats
                .iter()
                .all(|&(name, value)| tag.feat(name) == Some(value));
            return own.then(|| Analysis {
                prob: 1.0,
                ..analysis.clone()
            });
        }
        let (lexeme, forms) = self.lexeme_forms(analysis)?;
        let form = self.closest(analysis, &lexeme, &forms, feats, &[])?;
        Some(self.form(analysis, &lexeme, form))
    }

    /// Форма, согласованная с количеством `number`: «1 рубль», «2 рубля», «5 рублей»
    ///
    /// Согласуются существительные, прилагательные, местоименные прилагательные и причастия
    /// в любом падеже: «о 2 рублях», «вижу двух котов», «две новые книги». Существительные
    /// с адъективным склонением согласуются как прилагательные («два учёных»), из форм
    /// родительного падежа множественного числа берется счетная («5 вольт»). У слов без
    /// единственного числа («сутки») берется множественное: «1 сутки», «2 суток»
    pub fn agree(&self, analysis: &Analysis, number: u64) -> Option<Analysis> {
        self.agree_with(analysis, Some(number))
    }

    /// Форма, согласованная с дробным количеством («1,5 рубля», «о 2,5 рубля»): родительный падеж
    /// единственного числа в любом падеже, у слов без единственного числа - множественного
    pub fn agree_fraction(&self, analysis: &Analysis) -> Option<Analysis> {
        self.agree_with(analysis, None)
    }

    /// Форма, согласованная с целым количеством `number` или с дробным (`None`)
    fn agree_with(&self, analysis: &Analysis, number: Option<u64>) -> Option<Analysis> {
        if let Some((base, particle)) = self.without_particle(analysis) {
            return self
                .agree_with(&base, number)
                .map(|form| with_particle(form, &particle));
        }
        if let Some(lower) = self.lowercased(analysis) {
            return self
                .agree_with(&lower, number)
                .map(|form| capitalized_like(&analysis.word, form));
        }
        let tag = self.ud.get(analysis.ud)?;
        let feat = |name: &str| tag.feat(name);
        // Разделительный падеж («чаю») согласуется как родительный
        let case = match feat("Case")? {
            "Par" => "Gen",
            case => case,
        };
        let (lexeme, forms) = self.lexeme_forms(analysis)?;
        let nominal = matches!(tag.pos(), "NOUN" | "PROPN");
        // Женские фамилии на -ова, -ина («Иванова») склоняются как краткие прилагательные:
        // «две Ивановы»; несклоняемые («Абрамович») и на согласный («Брик») - нет
        let female_surname = feat("NameType") == Some("Sur")
            && feat("Gender") == Some("Fem")
            && feat("InflClass").is_none()
            && ["ова", "ева", "ёва", "ина", "ына"]
                .iter()
                .any(|ending| analysis.lemma.ends_with(ending));
        let adjectival = matches!(tag.pos(), "ADJ" | "DET")
            || feat("VerbForm") == Some("Part")
            || (nominal && (female_surname || self.adjectival_declension(&lexeme, &forms)));
        if !adjectival && !nominal {
            return None;
        }
        let gen_sing = [("Case", "Gen"), ("Number", "Sing")];
        let gen_plur = [("Case", "Gen"), ("Number", "Plur")];
        let counting = self.preferences.counting.as_slice();
        let Some(number) = number else {
            let form = [&gen_sing[..], &gen_plur[..]]
                .into_iter()
                .find_map(|feats| self.closest(analysis, &lexeme, &forms, feats, &[]))?;
            return Some(self.form(analysis, &lexeme, form));
        };
        let one = number % 10 == 1 && number % 100 != 11;
        let few = (2..=4).contains(&(number % 10)) && !(12..=14).contains(&(number % 100));
        let animate = feat("Animacy") == Some("Anim");
        let sing = [("Case", case), ("Number", "Sing")];
        let plur = [("Case", case), ("Number", "Plur")];
        let plur_inanimate = [("Case", case), ("Number", "Plur"), ("Animacy", "Inan")];
        let targets: Vec<&[(&str, &str)]> = if one {
            vec![&sing, &plur]
        } else if !matches!(case, "Nom" | "Acc") {
            vec![&plur]
        } else if case == "Acc" && animate && (adjectival || (few && number < 5)) {
            // «вижу двух котов», «вижу двадцать два учёных», но «вижу двадцать два кота»
            vec![&gen_plur]
        } else if adjectival && few && feat("Gender") == Some("Fem") {
            vec![&plur_inanimate, &plur]
        } else if few && !adjectival {
            vec![&gen_sing, &gen_plur]
        } else {
            vec![&gen_plur]
        };
        let form = targets
            .into_iter()
            .find_map(|feats| self.closest(analysis, &lexeme, &forms, feats, counting))?;
        Some(self.form(analysis, &lexeme, form))
    }

    /// Склоняется ли существительное как прилагательное («учёный», «столовая», «Толстой»):
    /// по окончаниям именительного и родительного падежей единственного числа
    fn adjectival_declension(&self, lexeme: &Lexeme, forms: &[u16]) -> bool {
        let ends = |case: &str, endings: &[&str]| {
            let Some(required) = [("Case", case), ("Number", "Sing")]
                .into_iter()
                .map(|(name, value)| self.code(name, value))
                .collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            forms
                .iter()
                .find(|&&form| {
                    self.codes(lexeme.ud(form)).is_some_and(|codes| {
                        required
                            .iter()
                            .all(|&(name, value)| codes.values[name] == value)
                    })
                })
                .is_some_and(|&form| {
                    let word = self.spell(lexeme, form);
                    endings.iter().any(|ending| word.ends_with(ending))
                })
        };
        ends("Nom", &["ый", "ий", "ой", "ое", "ее", "ая", "яя"])
            && ends("Gen", &["ого", "его", "ой", "ей"])
    }

    /// Форма из `forms` с признаками `feats`, ближайшая к разбору: больше общих с ним части речи,
    /// признаков и помет, а также желательных помет `prefer`, затем меньше лишних помет и того,
    /// чего не просили, - краткой формы, одушевленности, инклюзива («кошкой», а не «кошкою»;
    /// «новый», а не «нового»; «сделай», а не «сделаем»), затем ближе по написанию к разбору,
    /// если у его формы есть вариант написания («листьев», а не «листов»), затем раньше
    /// в парадигме - поэтому без рода и числа берется мужской род единственного числа
    fn closest(
        &self,
        analysis: &Analysis,
        lexeme: &Lexeme,
        forms: &[u16],
        feats: &[(&str, &str)],
        prefer: &[u16],
    ) -> Option<u16> {
        let required = feats
            .iter()
            .map(|&(name, value)| self.code(name, value))
            .collect::<Option<Vec<_>>>()?;
        let current = self.codes(analysis.ud)?;
        let mut wanted = current.values.to_vec();
        let preferences = &self.preferences;
        // У инфинитива, который сам не подходит под запрос, без запроса формы глагола желательна
        // личная форма, а не причастие
        if let (Some((form_name, infinitive)), Some((_, finite))) =
            (preferences.infinitive, preferences.finite)
            && wanted[form_name] == infinitive
            && required.iter().all(|&(name, _)| name != form_name)
            && required
                .iter()
                .any(|&(name, value)| current.values[name] != value)
        {
            wanted[form_name] = finite;
        }
        for &(name, value) in &required {
            wanted[name] = value;
        }
        let mut labels = current.labels.to_vec();
        for &label in prefer {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        let unwanted = |feature: Option<(usize, u16)>, codes: &Codes| {
            feature
                .is_some_and(|(index, code)| codes.values[index] == code && wanted[index] != code)
        };
        // Форма разбора с вариантом написания («листья» и «листы»): из равных берется ближайшая
        // по написанию
        let own = lexeme.affixes(analysis.entry.map_or(0, |entry| entry.form));
        let twin = forms
            .iter()
            .any(|&form| lexeme.ud(form) == analysis.ud && lexeme.affixes(form) != own);
        // Оценка формы: общее с разбором, лишнее, общее начало написания
        type Score = (usize, Reverse<usize>, usize);
        let mut best: Option<(u16, Score)> = None;
        for &form in forms {
            let ud = lexeme.ud(form);
            if self
                .values(ud)
                .is_none_or(|values| required.iter().any(|&(name, value)| values[name] != value))
            {
                continue;
            }
            let Some(codes) = self.codes(ud) else {
                continue;
            };
            let same = wanted
                .iter()
                .zip(codes.values)
                .filter(|&(wanted, value)| *wanted != 0 && wanted == value)
                .count()
                + labels
                    .iter()
                    .filter(|label| codes.labels.contains(label))
                    .count()
                + usize::from(codes.pos == current.pos);
            let extra = codes
                .labels
                .iter()
                .filter(|label| !labels.contains(label))
                .count()
                + usize::from(unwanted(preferences.short, &codes))
                + usize::from(unwanted(preferences.animate, &codes))
                + usize::from(preferences.inclusive.is_some_and(|label| {
                    codes.labels.contains(&label)
                        && preferences
                            .first_person
                            .is_none_or(|(index, code)| wanted[index] != code)
                }));
            let common = if twin {
                self.spell(lexeme, form)
                    .chars()
                    .zip(analysis.word.chars())
                    .take_while(|(a, b)| a == b)
                    .count()
            } else {
                0
            };
            let score = (same, Reverse(extra), common);
            if best.is_none_or(|(_, top)| score > top) {
                best = Some((form, score));
            }
        }
        best.map(|(form, _)| form)
    }

    /// Номер признака в `features` и код значения
    fn code(&self, name: &str, value: &str) -> Option<(usize, u16)> {
        feature_code(&self.features, name, value)
    }

    /// Коды тега UD для подбора формы
    fn codes(&self, number: u16) -> Option<Codes<'_>> {
        let (pos, _, labels) = self.ud.numbers(number)?;
        Some(Codes {
            pos,
            values: self.values(number)?,
            labels,
        })
    }

    /// Коды значений признаков тега UD
    fn values(&self, number: u16) -> Option<&[u16]> {
        let count = self.features.len();
        let start = usize::from(*self.ud_core.get(usize::from(number))?) * count;
        self.ud_values.get(start..start + count)
    }

    /// Часть речи тега UD; у несуществующего тега - пустая строка
    fn pos(&self, number: u16) -> &str {
        self.ud.get(number).map_or("", UdTag::pos)
    }

    /// Значение признака `name` тега UD
    fn feat(&self, number: u16, name: &str) -> Option<&str> {
        self.ud.get(number)?.feat(name)
    }

    /// Есть ли у тега UD значение признака с кодом `code` из `features`
    fn has_code(&self, number: u16, code: Option<(usize, u16)>) -> bool {
        code.is_some_and(|(index, code)| {
            self.values(number)
                .is_some_and(|values| values.get(index) == Some(&code))
        })
    }

    /// Лексема разбора и номера ее форм с той же леммой по порядку
    fn lexeme_forms<'a>(&'a self, analysis: &'a Analysis) -> Option<(Lexeme<'a>, Vec<u16>)> {
        let entry = analysis.entry?;
        let lexeme = self.lexeme(entry.table, entry.form, &analysis.word)?;
        let own = lexeme.lemma_form(entry.form);
        // Лемма формы та же, если ее форма леммы пишется так же: префиксы и суффиксы словаря
        // не повторяются, поэтому написания сравниваются по их номерам
        let affixes = lexeme.affixes(own);
        let forms = (0..lexeme.cells.len() as u16)
            .filter(|&form| {
                let lemma_form = lexeme.lemma_form(form);
                lemma_form == own || lexeme.affixes(lemma_form) == affixes
            })
            .collect();
        Some((lexeme, forms))
    }

    /// Разбор со словоформой в нижнем регистре, если лексема строится только по ней («2005-ГО»)
    fn lowercased(&self, analysis: &Analysis) -> Option<Analysis> {
        let entry = analysis.entry?;
        if self
            .lexeme(entry.table, entry.form, &analysis.word)
            .is_some()
        {
            return None;
        }
        let word = analysis.word.to_lowercase();
        (word != analysis.word && self.lexeme(entry.table, entry.form, &word).is_some()).then(
            || Analysis {
                word,
                ..analysis.clone()
            },
        )
    }

    /// Лексема словоформы `word` - формы `form` таблицы `table`
    fn lexeme<'a>(&'a self, table: u16, form: u16, word: &'a str) -> Option<Lexeme<'a>> {
        let mut lexeme = self.table_lexeme(table, "")?;
        if usize::from(form) >= lexeme.cells.len() {
            return None;
        }
        let (prefix, suffix) = lexeme.affixes(form);
        lexeme.stem = word
            .strip_prefix(self.prefixes.at(prefix))?
            .strip_suffix(self.suffixes.at(suffix))?;
        Some(lexeme)
    }

    /// Лексема таблицы `table` с основой `stem`
    fn table_lexeme<'a>(&'a self, table: u16, stem: &'a str) -> Option<Lexeme<'a>> {
        let start = *self.table_offsets.get(usize::from(table))? as usize;
        let tables = self.tables();
        let forms = self.paradigm(*tables.get(start)? as u16)?;
        let cells = tables.get(start + 1..start + 1 + forms.len() / 3)?;
        Some(Lexeme {
            table,
            forms,
            cells,
            stem,
        })
    }

    /// Лемма формы лексемы; у сокращения - полное слово («млн» - «миллион»)
    fn lemma(&self, lexeme: &Lexeme, form: u16) -> String {
        let lemma = self.spell(lexeme, lexeme.lemma_form(form));
        match self.abbreviation_lemma(&lemma, lexeme.tag(form), lexeme.ud(form)) {
            Some(full) => full.to_owned(),
            None => lemma,
        }
    }

    /// Написание формы лексемы
    fn spell(&self, lexeme: &Lexeme, form: u16) -> String {
        let (prefix, suffix) = lexeme.affixes(form);
        [
            self.prefixes.at(prefix),
            lexeme.stem,
            self.suffixes.at(suffix),
        ]
        .concat()
    }

    /// Разбор формы лексемы разбора `analysis`
    fn form(&self, analysis: &Analysis, lexeme: &Lexeme, form: u16) -> Analysis {
        let (prefix, suffix) = lexeme.affixes(form);
        let prefix = self.prefixes.at(prefix);
        let mut stem = lexeme.stem.chars();
        // У слова с заглавной буквы заглавной становится приставка формы: «Покринжовее»
        let word = match (prefix.chars().next(), stem.next()) {
            (Some(head), Some(first)) if first.is_uppercase() => [
                head.to_uppercase().to_string().as_str(),
                &prefix[head.len_utf8()..],
                first.to_lowercase().to_string().as_str(),
                stem.as_str(),
                self.suffixes.at(suffix),
            ]
            .concat(),
            _ => self.spell(lexeme, form),
        };
        // Окончание формы слова капсом - тоже заглавными: «ЧАТБОТОВЕД» - «ЧАТБОТОВЕДА»
        let word = if caps(&analysis.word) {
            word.to_uppercase()
        } else {
            word
        };
        Analysis {
            word,
            lemma: analysis.lemma.clone(),
            ud: lexeme.ud(form),
            tag: Some(lexeme.tag(form)),
            prob: 1.0,
            source: analysis.source,
            entry: Some(Entry {
                table: lexeme.table,
                form,
            }),
        }
    }

    /// Есть ли словоформа в словаре
    pub fn is_known(&self, word: &str) -> bool {
        let normal = normalized_input(word);
        let Some(key) = self.alphabet.input_key(normal.as_deref().unwrap_or(word)) else {
            return false;
        };
        self.spellings(&key)
            .is_some_and(|spellings| spellings.variants().any(|(yo, _)| yo & key.yo == key.yo))
            || (!self.own_words.is_empty() && self.is_own(&key))
    }

    fn paradigm(&self, paradigm: u16) -> Option<&[u16]> {
        let i = paradigm as usize;
        let offsets = self.paradigm_offsets();
        let (start, end) = (*offsets.get(i)?, *offsets.get(i + 1)?);
        self.paradigms().get(start as usize..end as usize)
    }

    /// Список разборов: `(таблица << 16) | форма` каждого
    fn records_at(&self, offset: u32) -> Option<&[u32]> {
        list(self.records(), offset, 1)
    }

    /// Список частот: пары «тег, P(тег | словоформа) в миллионных долях»
    fn probabilities_at(&self, offset: u32) -> Option<Probabilities<'_>> {
        let lists = self.probability_records();
        let header = *lists.get(offset as usize)?;
        let count = (header & !ENTRY_KEYS) as usize;
        if count > MAX_LIST {
            return None;
        }
        let start = offset as usize + 1;
        Some(Probabilities {
            by_entry: header & ENTRY_KEYS != 0,
            pairs: lists.get(start..start + 2 * count)?,
        })
    }

    /// Доля тега или формы таблицы делится поровну между разборами с этим ключом и той же таблицей частот
    /// (с частотами из запроса - всеми, иначе - того же написания) и нормируется. Без частот
    /// вероятности остаются нулевыми: равные доли раздает `merge` после слияния. Если частоты
    /// есть только у части написаний («совершённого», но не «совершенного»), разбор написания без
    /// частот получает среднюю долю разборов с частотами
    fn assign_probabilities(
        &self,
        groups: &[(usize, usize, Option<Probabilities>)],
        input: bool,
        analyses: &mut [Analysis],
    ) {
        let table = |i: usize| {
            groups
                .iter()
                .find(|&&(start, end, _)| (start..end).contains(&i))
                .and_then(|&(_, _, table)| table)
        };
        if groups.iter().all(|&(_, _, table)| table.is_none()) {
            return;
        }
        let mut total = 0.0;
        let (mut known_total, mut known_count) = (0.0, 0);
        for i in 0..analyses.len() {
            let Some(table) = table(i) else {
                continue;
            };
            let key = |analysis: &Analysis| table.key(analysis);
            let (own, spelling) = (key(&analyses[i]), analyses[i].word.as_str());
            let share = analyses
                .iter()
                .filter(|other| key(other) == own && (input || other.word == spelling))
                .count() as f64;
            let prob = table
                .pairs
                .as_chunks::<2>()
                .0
                .iter()
                .find(|[known, _]| Some(*known) == own)
                .map_or(0.0, |[_, millionths]| f64::from(*millionths));
            analyses[i].prob = prob / share;
            total += analyses[i].prob;
            known_total += analyses[i].prob;
            known_count += 1;
        }
        // Разбор написания без частот получает среднюю долю разборов с частотами
        let mean = known_total / f64::from(known_count);
        for (i, analysis) in analyses.iter_mut().enumerate() {
            if table(i).is_none() {
                analysis.prob = mean;
                total += mean;
            }
        }
        if total > 0.0 {
            for analysis in analyses.iter_mut() {
                analysis.prob /= total;
            }
            analyses.sort_by(|a, b| b.prob.total_cmp(&a.prob));
        }
    }

    /// Написания ключа словоформы
    fn spellings(&self, key: &Key) -> Option<Spellings<'_>> {
        Some(match WordValue::unpack(self.words.get(key.bytes())?) {
            WordValue::Single {
                yo,
                records,
                probabilities,
            } => Spellings::Single {
                yo,
                records,
                probabilities,
            },
            WordValue::Many(offset) => {
                let lists = self.variants().get(offset as usize..)?;
                let (variants, rest) = triples(lists)?;
                let (probabilities, _) = triples(rest)?;
                Spellings::Many {
                    variants,
                    probabilities,
                }
            }
        })
    }

    /// Написания, в которых есть все «ё» запроса (места `yo`): сначала точное совпадение,
    /// затем варианты с «ё» по местам слева направо
    fn matches(&self, spellings: &Spellings, yo: u64) -> Matches {
        let mut matches = Matches {
            items: [(0, 0); MAX_SPELLINGS],
            len: 0,
        };
        for (own, records) in spellings.variants() {
            // Написаний не больше `MAX_SPELLINGS`: это проверено при загрузке
            if own & yo == yo && matches.len < MAX_SPELLINGS {
                matches.items[matches.len] = (own, records);
                matches.len += 1;
            }
        }
        if yo != 0 {
            matches.items[..matches.len].sort_by(|a, b| yo_order(a.0 & !yo, b.0 & !yo));
        }
        matches
    }

    /// Число словоформ сверх ключей автомата: ключи с несколькими написаниями из секции
    /// `variants`; списки обходятся, проверяются длины и ссылки на списки разборов и частот
    fn check_variants(&self) -> Result<usize, Error> {
        let invalid = |msg: &str| Err(Error::Format(msg.into()));
        let (records, probabilities) = (self.records().len(), self.probability_records().len());
        let mut extra = 0;
        let mut rest = self.variants();
        while !rest.is_empty() {
            let Some((variants, after)) = triples(rest) else {
                return invalid("список написаний обрезан");
            };
            let Some((frequencies, after)) = triples(after) else {
                return invalid("список написаний обрезан");
            };
            if variants.is_empty()
                || variants.len() > MAX_SPELLINGS
                || variants
                    .iter()
                    .any(|&[_, _, offset]| offset as usize >= records)
                || frequencies
                    .iter()
                    .any(|&[_, _, offset]| offset as usize >= probabilities)
            {
                return invalid("неверный список написаний");
            }
            extra += variants.len() - 1;
            rest = after;
        }
        Ok(extra)
    }

    fn numbers<T: bytemuck::Pod>(&self, range: &Range<usize>) -> &[T] {
        format::numbers(&self.storage.bytes()[range.clone()])
    }

    fn paradigm_offsets(&self) -> &[u32] {
        self.numbers(&self.layout.paradigm_offsets)
    }

    fn paradigms(&self) -> &[u16] {
        self.numbers(&self.layout.paradigms)
    }

    fn tables(&self) -> &[u32] {
        self.numbers(&self.layout.tables)
    }

    fn records(&self) -> &[u32] {
        self.numbers(&self.layout.records)
    }

    fn variants(&self) -> &[u32] {
        self.numbers(&self.layout.variants)
    }

    fn probability_records(&self) -> &[u32] {
        self.numbers(&self.layout.probability_records)
    }

    fn guess_records(&self) -> &[u32] {
        self.numbers(&self.layout.guess_records)
    }
}

/// Список «длина, элементы по `width` чисел» с места `offset` секции `lists`
fn list(lists: &[u32], offset: u32, width: usize) -> Option<&[u32]> {
    let start = offset as usize + 1;
    let count = *lists.get(start - 1)? as usize;
    if count > MAX_LIST {
        return None;
    }
    lists.get(start..start + count * width)
}

/// Список «длина, тройки чисел» в начале `lists` и остаток после него
fn triples(lists: &[u32]) -> Option<(&[[u32; 3]], &[u32])> {
    let (&count, rest) = lists.split_first()?;
    let count = usize::try_from(count)
        .ok()
        .filter(|&count| count <= MAX_LIST)?;
    let (list, rest) = rest.split_at_checked(3 * count)?;
    Some((list.as_chunks::<3>().0, rest))
}

/// Автомат секции словаря. Испорченный автомат грузится без ошибок, но паникует при обходе;
/// порчу ловит контрольная сумма файла, а адрес корня проверяется здесь: `Fst::new` проверяет
/// его только у пустого автомата
fn automaton(storage: &Arc<Storage>, range: &Range<usize>) -> Result<Map<Slice>, Error> {
    let slice = Slice {
        storage: Arc::clone(storage),
        range: range.clone(),
    };
    let map = Map::new(slice).map_err(|err| Error::Format(err.to_string()))?;
    // Корень пишется последним, за ним - 20 байт: число ключей, адрес корня и контрольная сумма
    let bytes = map.as_fst().as_bytes();
    let end = bytes.len() - 4;
    let root = u64::from_le_bytes(bytes[end - 8..end].try_into().unwrap_or_default());
    let empty = root == 0 && bytes.len() == 36;
    if !empty && root.checked_add(21) != Some(bytes.len() as u64) {
        return Err(Error::Format("неверный адрес корня автомата".into()));
    }
    Ok(map)
}

/// Номер признака `name` в `features` и код значения `value`
fn feature_code(
    features: &BTreeMap<String, BTreeSet<String>>,
    name: &str,
    value: &str,
) -> Option<(usize, u16)> {
    let (index, (_, values)) = features
        .iter()
        .enumerate()
        .find(|(_, (known, _))| *known == name)?;
    let position = values.iter().position(|known| known == value)?;
    Some((index, position as u16 + 1))
}

/// Признаки и пометы, которые `closest` берет, только если их просят, личная форма
/// для инфинитива и счетная форма для согласования с числом
#[derive(Clone, Default)]
struct Preferences {
    infinitive: Option<(usize, u16)>,
    finite: Option<(usize, u16)>,
    short: Option<(usize, u16)>,
    animate: Option<(usize, u16)>,
    first_person: Option<(usize, u16)>,
    inclusive: Option<u16>,
    counting: Option<u16>,
}

/// Слово в обычной записи: знаки и буквы-двойники (`normalized_marks`, `unified_script`)
/// приведены к обычным. `None`, если менять нечего
fn normalized_input(word: &str) -> Option<String> {
    // Слово из ASCII или из кириллицы с цифрами и знаками ASCII: менять нечего
    if word.is_ascii()
        || word
            .chars()
            .all(|c| rules::cyrillic(c) || (c.is_ascii() && !c.is_ascii_alphabetic()))
    {
        return None;
    }
    let unified = unified_script(word);
    normalized_marks(unified.as_deref().unwrap_or(word)).or(unified)
}

/// Буквы-двойники другой письменности в отрезке букв - письменностью отрезка: ее задают буквы
/// без двойника («eгo» с латинскими «e», «o» - «его», «ХХI» с кириллическими «Х» - «XXI»), а если
/// двойники все буквы - большинство, при равенстве кириллица («cтали» - «стали», «сedar» -
/// «cedar»). Отрезок с буквами без двойника обеих письменностей и отрезки через дефис или точку
/// («ТНК-BP», «Газета.ru») не меняются. `None`, если менять нечего
fn unified_script(word: &str) -> Option<String> {
    // Смешение возможно только в слове с латинской буквой и не-ASCII символом
    if word.is_ascii() || !word.bytes().any(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let chars: Vec<char> = word.chars().collect();
    let mut normal = String::with_capacity(word.len());
    let mut changed = false;
    let mut at = 0;
    while at < chars.len() {
        let start = at;
        // Комбинирующие знаки, мягкий перенос и соединители слов - внутри отрезка: их убирают
        // после приведения, и половинки слова приводятся вместе
        while at < chars.len()
            && (chars[at].is_alphabetic()
                || matches!(
                    chars[at],
                    '\u{300}'..='\u{36F}' | '\u{AD}' | '\u{2060}' | '\u{FEFF}'
                ))
        {
            at += 1;
        }
        let run = &chars[start..at];
        let latin = |c: char| c.is_ascii_alphabetic();
        // «ѐ» и «ѝ» - кириллица: знак ударения в них снимается уже после приведения
        let cyrillic = |c: char| rules::cyrillic(c) || matches!(c, 'ѐ' | 'ѝ' | 'Ѐ' | 'Ѝ');
        let count = |script: &dyn Fn(char) -> bool, own: bool| {
            run.iter()
                .filter(|&&c| script(c) && own != has_twin(c))
                .count()
        };
        let (latin_own, cyrillic_own) = (count(&latin, true), count(&cyrillic, true));
        let latin_all = latin_own + count(&latin, false);
        let cyrillic_all = cyrillic_own + count(&cyrillic, false);
        let to_cyrillic = match (latin_own > 0, cyrillic_own > 0) {
            (true, true) => None,
            (true, false) => Some(false),
            (false, true) => Some(true),
            (false, false) => Some(cyrillic_all >= latin_all),
        };
        let unified: Option<String> = to_cyrillic
            .filter(|_| latin_all > 0 && cyrillic_all > 0)
            .and_then(|to_cyrillic| {
                run.iter()
                    .map(|&c| match (to_cyrillic, latin(c), rules::cyrillic(c)) {
                        (true, true, _) => twin(c, LATIN_TWINS, CYRILLIC_TWINS),
                        (false, _, true) => twin(c, CYRILLIC_TWINS, LATIN_TWINS),
                        _ => Some(c),
                    })
                    .collect()
            });
        match unified {
            Some(unified) => {
                normal.push_str(&unified);
                changed = true;
            }
            None => normal.extend(run),
        }
        if at < chars.len() {
            normal.push(chars[at]);
            at += 1;
        }
    }
    changed.then_some(normal)
}

/// Есть ли у буквы двойник другой письменности
fn has_twin(c: char) -> bool {
    LATIN_TWINS.contains(c) || CYRILLIC_TWINS.contains(c)
}

/// Латинские буквы, которые пишутся как кириллические, и их кириллические двойники
const LATIN_TWINS: &str = "aceopxyABCEHKMOPTXY";
const CYRILLIC_TWINS: &str = "асеорхуАВСЕНКМОРТХУ";

/// Двойник буквы `c` из `from` в `to`
fn twin(c: char, from: &str, to: &str) -> Option<char> {
    let index = from.chars().position(|known| known == c)?;
    to.chars().nth(index)
}

/// Слово в составленной записи, с обычным дефисом и без знаков ударения, мягких переносов
/// и соединителей слов: «и» со знаком краткой и «е» с диерезисом (разложенный Unicode, NFD) -
/// «й» и «ё», неразрывный дефис и дефис U+2010 - «-», «ѐ» и «ѝ» (знак ударения в составленной
/// букве) - «е» и «и»; знак ударения после кириллической буквы, мягкий перенос и соединители
/// слов U+2060 и U+FEFF убираются. `None`, если менять нечего
fn normalized_marks(word: &str) -> Option<String> {
    const MARKS: [char; 13] = [
        '\u{306}', '\u{308}', '\u{2010}', '\u{2011}', '\u{300}', '\u{301}', '\u{AD}', '\u{2060}',
        '\u{FEFF}', 'ѐ', 'ѝ', 'Ѐ', 'Ѝ',
    ];
    if !word.contains(MARKS) {
        return None;
    }
    let mut normal = String::with_capacity(word.len());
    for c in word.chars() {
        let last = normal.chars().last();
        let composed = match (last, c) {
            (Some('и'), '\u{306}') => Some('й'),
            (Some('И'), '\u{306}') => Some('Й'),
            (Some('е'), '\u{308}') => Some('ё'),
            (Some('Е'), '\u{308}') => Some('Ё'),
            _ => None,
        };
        let stress = matches!(c, '\u{300}' | '\u{301}') && last.is_some_and(rules::cyrillic);
        match (composed, c) {
            (Some(letter), _) => {
                normal.pop();
                normal.push(letter);
            }
            (None, '\u{2010}' | '\u{2011}') => normal.push('-'),
            (None, '\u{AD}' | '\u{2060}' | '\u{FEFF}') => {}
            (None, 'ѐ') => normal.push('е'),
            (None, 'ѝ') => normal.push('и'),
            (None, 'Ѐ') => normal.push('Е'),
            (None, 'Ѝ') => normal.push('И'),
            (None, _) if stress => {}
            (None, c) => normal.push(c),
        }
    }
    (normal != word).then_some(normal)
}

/// Совпадают ли написания с точностью до «е» на месте «ё»
fn same_up_to_yo(a: &str, b: &str) -> bool {
    a == b
        || (a.len() == b.len()
            && a.chars().zip(b.chars()).all(|(x, y)| {
                x == y || matches!((x, y), ('е', 'ё') | ('ё', 'е') | ('Е', 'Ё') | ('Ё', 'Е'))
            }))
}

/// Слово капсом: в нем есть буквы, и все заглавные
fn caps(word: &str) -> bool {
    word.chars().any(char::is_alphabetic) && !word.chars().any(char::is_lowercase)
}

/// Форма в регистре слова, от которого формы строились в нижнем регистре: капсом, если слово
/// капсом («ПИН-КОДАМ»), иначе общее начало слова и формы - в регистре слова («УрО» → «УрО»,
/// «Уру»), а без общего начала - с заглавной первой буквой («Покринжовее» → «Кринжового»)
fn capitalized_like(original: &str, form: Analysis) -> Analysis {
    if caps(original) {
        return Analysis {
            word: form.word.to_uppercase(),
            ..form
        };
    }
    let mut source = original.chars();
    let (mut shared, mut same) = (0, true);
    let mut word = String::with_capacity(form.word.len());
    for letter in form.word.chars() {
        match source.next() {
            Some(own) if same && own.to_lowercase().eq(letter.to_lowercase()) => {
                word.push(own);
                shared += 1;
            }
            _ => {
                same = false;
                word.push(letter);
            }
        }
    }
    if shared > 0 {
        return Analysis { word, ..form };
    }
    let upper = original.chars().next().is_some_and(char::is_uppercase);
    let mut letters = form.word.chars();
    match letters.next() {
        Some(first) if upper && first.is_lowercase() => Analysis {
            word: first.to_uppercase().chain(letters).collect(),
            ..form
        },
        _ => form,
    }
}

/// Наибольшее число признаков UD в словаре: в словаре OpenCorpora их 22
const MAX_FEATURES: usize = 255;

impl Dictionary {
    /// Парадигмы ссылаются только на существующие суффиксы, теги и префиксы
    fn check_paradigms(&self) -> Result<(), Error> {
        let (offsets, paradigms) = (self.paradigm_offsets(), self.paradigms());
        let invalid = |msg: &str| Err(Error::Format(msg.into()));
        if offsets.first() != Some(&0)
            || offsets.last().map(|&end| end as usize) != Some(paradigms.len())
        {
            return invalid("границы парадигм не совпадают с таблицей парадигм");
        }
        if offsets.len() - 1 > usize::from(u16::MAX) + 1 {
            return invalid("больше 65 536 парадигм");
        }
        let limits = [self.suffixes.len(), self.tags.len(), self.prefixes.len()];
        for bounds in offsets.windows(2) {
            let (start, end) = (bounds[0] as usize, bounds[1] as usize);
            if end <= start || (end - start) % 3 != 0 {
                return invalid("парадигма неверной длины");
            }
            let Some(forms) = paradigms.get(start..end) else {
                return invalid("парадигма за пределами таблицы парадигм");
            };
            for (block, limit) in forms.chunks_exact(forms.len() / 3).zip(limits) {
                if block.iter().any(|&id| usize::from(id) >= limit) {
                    return invalid("парадигма ссылается за пределы таблицы");
                }
            }
        }
        Ok(())
    }

    /// Таблицы словоизменения ссылаются на существующие парадигмы, формы и теги UD; вывод -
    /// начала таблиц
    fn check_tables(&self, ud_count: usize) -> Result<Vec<u32>, Error> {
        let invalid = |msg: &str| Err(Error::Format(msg.into()));
        let (tables, paradigm_offsets) = (self.tables(), self.paradigm_offsets());
        let mut offsets = Vec::new();
        let mut start = 0;
        while start < tables.len() {
            if offsets.len() > usize::from(u16::MAX) {
                return invalid("больше 65 536 таблиц словоизменения");
            }
            let paradigm = tables[start] as usize;
            let Some(&[begin, end]) = paradigm
                .checked_add(2)
                .and_then(|stop| paradigm_offsets.get(paradigm..stop))
            else {
                return invalid("таблица ссылается на несуществующую парадигму");
            };
            let count = (end - begin) as usize / 3;
            let Some(cells) = tables.get(start + 1..start + 1 + count) else {
                return invalid("таблица словоизменения обрезана");
            };
            if cells
                .iter()
                .any(|&cell| (cell >> 16) as usize >= count || (cell & 0xFFFF) as usize >= ud_count)
            {
                return invalid("таблица ссылается на несуществующую форму или тег UD");
            }
            offsets.push(start as u32);
            start += 1 + count;
        }
        Ok(offsets)
    }

    /// Списки разборов, частот и форм для угадывания ровно закрывают свои секции, они не длиннее
    /// `MAX_LIST`, а таблицы, формы и теги в них существуют
    fn check_lists(&self) -> Result<(), Error> {
        let (tables, offsets) = (self.tables(), self.paradigm_offsets());
        // Число форм каждой таблицы: таблицы и парадигмы уже проверены
        let form_counts: Vec<usize> = self
            .table_offsets
            .iter()
            .map(|&start| {
                let paradigm = tables[start as usize] as usize;
                (offsets[paradigm + 1] - offsets[paradigm]) as usize / 3
            })
            .collect();
        let entry = |packed: u32| {
            form_counts
                .get((packed >> 16) as usize)
                .is_some_and(|&count| ((packed & 0xFFFF) as usize) < count)
        };
        walk_lists(self.records(), 1, |items| {
            items.iter().all(|&packed| entry(packed))
        })?;
        // Длина списка частот - с битом ключей: список проверяется своим обходом
        let lists = self.probability_records();
        let mut start = 0;
        while start < lists.len() {
            let Some(probabilities) = self.probabilities_at(start as u32) else {
                return Err(Error::Format(
                    "список частот обрезан или длиннее 255 элементов".into(),
                ));
            };
            let valid = probabilities.pairs.chunks(2).all(|pair| {
                if probabilities.by_entry {
                    entry(pair[0])
                } else {
                    (pair[0] as usize) < self.tags.len()
                }
            });
            if !valid {
                return Err(Error::Format(
                    "список частот ссылается на несуществующий тег или форму".into(),
                ));
            }
            start += 1 + probabilities.pairs.len();
        }
        walk_lists(self.guess_records(), 2, |items| {
            items.chunks(2).all(|pair| entry(pair[0]))
        })
    }
}

/// Обход списков вида «длина, элементы по `width` чисел»
fn walk_lists(lists: &[u32], width: usize, valid: impl Fn(&[u32]) -> bool) -> Result<(), Error> {
    let invalid = |msg: &str| Err(Error::Format(msg.into()));
    let mut start = 0;
    while start < lists.len() {
        let count = lists[start] as usize;
        if count > MAX_LIST {
            return invalid("список длиннее 255 элементов");
        }
        let end = start + 1 + count * width;
        let Some(items) = lists.get(start + 1..end) else {
            return invalid("список обрезан");
        };
        if !valid(items) {
            return invalid("список ссылается на несуществующую таблицу, форму или тег");
        }
        start = end;
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::builder::Builder;
    use crate::format::Sections;

    const TAGS: [&str; 6] = [
        "NOUN,anim,masc sing,nomn",
        "NOUN,anim,masc sing,gent",
        "VERB,perf,intr plur,past,indc",
        "NOUN,inan,femn sing,gent",
        "NOUN,inan,femn plur,nomn",
        "ADJF,Supr,Qual masc,sing,nomn",
    ];

    fn entry(table: u16, form: u16) -> Entry {
        Entry { table, form }
    }

    /// Парадигма из форм `(суффикс, тег, префикс)` и ее таблица: лемма - форма 0, тег UD - номер
    /// тега OpenCorpora; вывод - номер таблицы
    fn lexeme_table(builder: &mut Builder, forms: &[(u16, u16, u16)]) -> u16 {
        let paradigm = builder.add_paradigm(forms).unwrap();
        let cells: Vec<(u16, u16)> = forms.iter().map(|&(_, tag, _)| (0, tag)).collect();
        builder.add_table(paradigm, &cells).unwrap()
    }

    /// Сборщик с тегами UD «X|Tag=номер»: номер тега UD совпадает с номером тега OpenCorpora
    fn builder(tags: &[&str], prefixes: &[&str], suffixes: &[&str]) -> Builder {
        let owned = |items: &[&str]| items.iter().map(|item| item.to_string()).collect();
        let mut builder = Builder::new(owned(tags), owned(prefixes), owned(suffixes));
        for number in 0..tags.len() {
            builder
                .add_ud_tag("X", &format!("Tag={number}"), &[])
                .unwrap();
        }
        builder
    }

    /// ёж (ежа), стать (стали), сталь (стали), большой (наибольший), все и всё
    fn dictionary_bytes() -> Vec<u8> {
        let mut builder = builder(
            &TAGS,
            &["", "наи"],
            &["", "ёж", "ежа", "ть", "ли", "ь", "и", "ой", "ий"],
        );
        builder.meta("тест");
        let hedgehog = lexeme_table(&mut builder, &[(1, 0, 0), (2, 1, 0)]);
        let become_ = lexeme_table(&mut builder, &[(3, 2, 0), (4, 2, 0)]);
        let steel = lexeme_table(&mut builder, &[(5, 3, 0), (6, 3, 0), (6, 4, 0)]);
        let big = lexeme_table(&mut builder, &[(7, 5, 0), (8, 5, 1)]);
        let plain = lexeme_table(&mut builder, &[(0, 0, 0)]);
        builder.add_word("ёж", &[entry(hedgehog, 0)]).unwrap();
        builder.add_word("ежа", &[entry(hedgehog, 1)]).unwrap();
        builder
            .add_word(
                "стали",
                &[entry(steel, 1), entry(steel, 2), entry(become_, 1)],
            )
            .unwrap();
        builder.add_word("наибольший", &[entry(big, 1)]).unwrap();
        builder.add_word("все", &[entry(plain, 0)]).unwrap();
        builder.add_word("всё", &[entry(plain, 0)]).unwrap();
        builder.add_probability("стали", 2, 970_000).unwrap();
        builder.add_probability("стали", 3, 20_000).unwrap();
        builder.add_probability("стали", 4, 10_000).unwrap();
        builder.build().unwrap()
    }

    fn dictionary() -> Dictionary {
        Dictionary::from_bytes(&dictionary_bytes()).unwrap()
    }

    fn summary(analyses: &[Analysis]) -> Vec<(&str, &str, u16)> {
        analyses
            .iter()
            .map(|a| (a.word.as_str(), a.lemma.as_str(), a.tag.unwrap()))
            .collect()
    }

    #[test]
    fn lemma_from_paradigm() {
        let dict = dictionary();
        assert_eq!(summary(&dict.analyze("ежа")), [("ежа", "ёж", 1)]);
        assert_eq!(
            summary(&dict.analyze("наибольший")),
            [("наибольший", "большой", 5)]
        );
    }

    #[test]
    fn lemma_from_another_form() {
        let mut builder = builder(&["NOUN,masc", "NOUN,femn"], &[""], &["", "а", "ой"]);
        let paradigm = builder
            .add_paradigm(&[(0, 0, 0), (1, 1, 0), (2, 1, 0)])
            .unwrap();
        // Женские формы - с леммой «петрова», формой 1
        let surname = builder
            .add_table(paradigm, &[(0, 0), (1, 1), (1, 1)])
            .unwrap();
        builder.add_word("петровой", &[entry(surname, 2)]).unwrap();
        builder.add_word("петрова", &[entry(surname, 1)]).unwrap();
        builder.add_word("петров", &[entry(surname, 0)]).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        assert_eq!(dict.analyze("петровой")[0].lemma, "петрова");
        let words = |word: &str| -> Vec<String> {
            let analysis = &dict.analyze(word)[0];
            dict.forms(analysis)
                .into_iter()
                .map(|form| form.word)
                .collect()
        };
        assert_eq!(words("петровой"), ["петрова", "петровой"]);
        assert_eq!(words("петров"), ["петров"]);
        assert_eq!(
            dict.opencorpora("петровой"),
            [(
                "петровой".to_string(),
                "NOUN,femn",
                "петров".to_string(),
                1.0
            )]
        );
    }

    /// Формы лексемы: окончание и признаки UD
    pub(crate) type Forms = Vec<(&'static str, String)>;

    /// Лексемы «кот», «учёный», «сутки», «молоко», «новый» и «нести»
    fn lexicon() -> Dictionary {
        const NOUN: [&str; 12] = [
            "Case=Nom|Number=Sing",
            "Case=Gen|Number=Sing",
            "Case=Dat|Number=Sing",
            "Case=Acc|Number=Sing",
            "Case=Ins|Number=Sing",
            "Case=Loc|Number=Sing",
            "Case=Nom|Number=Plur",
            "Case=Gen|Number=Plur",
            "Case=Dat|Number=Plur",
            "Case=Acc|Number=Plur",
            "Case=Ins|Number=Plur",
            "Case=Loc|Number=Plur",
        ];
        let noun = |animacy: &str, endings: [&'static str; 12]| -> Forms {
            endings
                .into_iter()
                .zip(NOUN)
                .filter(|(ending, _)| *ending != "-")
                .map(|(ending, feats)| (ending, format!("Animacy={animacy}|{feats}")))
                .collect()
        };
        let lexemes: Vec<(&str, &str, Forms)> = vec![
            (
                "NOUN",
                "кот",
                noun(
                    "Anim",
                    [
                        "", "а", "у", "а", "ом", "е", "ы", "ов", "ам", "ов", "ами", "ах",
                    ],
                ),
            ),
            (
                "NOUN",
                "учён",
                noun(
                    "Anim",
                    [
                        "ый", "ого", "ому", "ого", "ым", "ом", "ые", "ых", "ым", "ых", "ыми", "ых",
                    ],
                ),
            ),
            (
                "NOUN",
                "сут",
                noun(
                    "Inan",
                    [
                        "-", "-", "-", "-", "-", "-", "ки", "ок", "кам", "ки", "ками", "ках",
                    ],
                ),
            ),
            (
                "NOUN",
                "молок",
                noun(
                    "Inan",
                    ["о", "а", "у", "о", "ом", "е", "-", "-", "-", "-", "-", "-"],
                ),
            ),
            (
                "ADJ",
                "нов",
                [
                    ("ый", "Case=Nom|Gender=Masc|Number=Sing"),
                    ("ая", "Case=Nom|Gender=Fem|Number=Sing"),
                    ("ую", "Case=Acc|Gender=Fem|Number=Sing"),
                    ("ые", "Case=Nom|Number=Plur"),
                    ("ых", "Case=Gen|Number=Plur"),
                    ("ых", "Animacy=Anim|Case=Acc|Number=Plur"),
                    ("ые", "Animacy=Inan|Case=Acc|Number=Plur"),
                ]
                .map(|(ending, feats)| (ending, feats.to_string()))
                .to_vec(),
            ),
            (
                "VERB",
                "нес",
                vec![
                    ("ти", "VerbForm=Inf".into()),
                    ("ла", "Gender=Fem|Tense=Past".into()),
                ],
            ),
        ];
        dictionary_of(&lexemes, &[])
    }

    /// Словарь из лексем «часть речи, основа, формы» с тегами OpenCorpora «часть речи признаки»
    /// и частотами `(словоформа, часть речи и признаки формы, доля в миллионных)`; лемма - форма 0
    pub(crate) fn dictionary_bytes_of(
        lexemes: &[(&str, &str, Forms)],
        probabilities: &[(&str, &str, u32)],
    ) -> Vec<u8> {
        let mut tags: Vec<String> = Vec::new();
        let mut suffixes: Vec<String> = Vec::new();
        for (pos, _, forms) in lexemes {
            for (ending, feats) in forms {
                let tag = format!("{pos} {feats}");
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
                if !suffixes.iter().any(|suffix| suffix == ending) {
                    suffixes.push(ending.to_string());
                }
            }
        }
        let mut builder = Builder::new(tags.clone(), vec![String::new()], suffixes.clone());
        for tag in &tags {
            let (pos, feats) = tag.split_once(' ').unwrap();
            builder.add_ud_tag(pos, feats, &[]).unwrap();
        }
        let id = |items: &[String], item: &str| {
            items.iter().position(|known| known == item).unwrap() as u16
        };
        for (pos, stem, forms) in lexemes {
            let paradigm: Vec<(u16, u16, u16)> = forms
                .iter()
                .map(|(ending, feats)| {
                    (
                        id(&suffixes, ending),
                        id(&tags, &format!("{pos} {feats}")),
                        0,
                    )
                })
                .collect();
            let table = lexeme_table(&mut builder, &paradigm);
            for (form, (ending, _)) in forms.iter().enumerate() {
                builder
                    .add_word(&format!("{stem}{ending}"), &[entry(table, form as u16)])
                    .unwrap();
            }
        }
        for &(word, tag, millionths) in probabilities {
            builder
                .add_probability(word, id(&tags, tag), millionths)
                .unwrap();
        }
        builder.build().unwrap()
    }

    pub(crate) fn dictionary_of(
        lexemes: &[(&str, &str, Forms)],
        probabilities: &[(&str, &str, u32)],
    ) -> Dictionary {
        Dictionary::from_bytes(&dictionary_bytes_of(lexemes, probabilities)).unwrap()
    }

    /// Разбор словоформы с признаками `feats`
    fn find(dict: &Dictionary, word: &str, feats: &str) -> Analysis {
        dict.analyze(word)
            .into_iter()
            .find(|analysis| {
                dict.ud_tag(analysis.ud)
                    .unwrap()
                    .feats_string()
                    .ends_with(feats)
            })
            .unwrap()
    }

    #[test]
    fn own_words_follow_sample() {
        let dict = lexicon().with_words([("бот", "кот")]).unwrap();
        let analysis = &dict.analyze("ботами")[0];
        assert_eq!(
            (analysis.lemma.as_str(), analysis.source),
            ("бот", Source::Dictionary)
        );
        assert!(dict.is_known("Боты") && !lexicon().is_known("боты"));
        let forms: Vec<String> = dict.forms(analysis).into_iter().map(|f| f.word).collect();
        assert_eq!(forms[..3], ["бот", "бота", "боту"]);
        let dative = dict.inflect(analysis, &[("Case", "Dat"), ("Number", "Sing")]);
        assert_eq!(dative.unwrap().word, "боту");
        // Разбор восстанавливается по форме таблицы, как из pickle
        let entry = analysis.entry.unwrap();
        let restored = dict.analysis("ботами", entry, Source::Dictionary).unwrap();
        assert_eq!(restored.lemma, "бот");
        assert!(
            lexicon()
                .analysis("ботами", entry, Source::Dictionary)
                .is_none()
        );
        // «е» запроса может обозначать «ё» своего слова; словарное слово не дублируется
        let dict = dict
            .with_words([("мочёный", "учёный"), ("кот", "кот")])
            .unwrap();
        assert_eq!(dict.analyze("моченого")[0].word, "мочёного");
        let probs = |dict: &Dictionary| -> Vec<f64> {
            dict.analyze("кота").iter().map(|a| a.prob).collect()
        };
        assert_eq!(probs(&dict), probs(&lexicon()));
        let error = |words: &[(&str, Sample)]| {
            let Err(Error::Word(message)) = lexicon().with_words(words.iter().copied()) else {
                panic!("{words:?}");
            };
            message
        };
        assert_eq!(
            error(&[("бот", "пёс".into())]),
            "Образца «пёс» нет в словаре"
        );
        assert!(error(&[("бот", "учёный".into())]).contains("не изменяется по образцу"));
        assert_eq!(error(&[(" ", "кот".into())]), "Свое слово пустое");
        assert!(error(&[("ый", "учёный".into())]).contains("основа"));
        let both = dictionary_of(
            &[
                (
                    "NOUN",
                    "печ",
                    vec![("ь", "Case=Nom|Number=Sing".to_owned())],
                ),
                ("VERB", "печ", vec![("ь", "VerbForm=Inf".to_owned())]),
            ],
            &[],
        );
        let Err(Error::Word(message)) = both.with_words([("зечь", "печь")]) else {
            panic!("неоднозначный образец");
        };
        assert!(message.contains("NOUN, VERB"), "{message}");
        let both = dictionary_of(
            &[
                (
                    "NOUN",
                    "печ",
                    vec![("ь", "Case=Nom|Number=Sing".to_owned())],
                ),
                ("VERB", "печ", vec![("ь", "VerbForm=Inf".to_owned())]),
            ],
            &[],
        );
        let dict = both.with_words([("зечь", ("печь", "VERB"))]).unwrap();
        let poses: Vec<&str> = dict
            .analyze("зечь")
            .iter()
            .map(|analysis| dict.ud_tag(analysis.ud).unwrap().pos())
            .collect();
        assert_eq!(poses, ["VERB"]);
    }

    #[test]
    fn own_word_after_two_samples() {
        // Одинаковые разборы по двум образцам из разных таблиц - один разбор, без лишней доли
        let noun = |case: &str| format!("Case={case}|Number=Sing");
        let dict = || {
            dictionary_of(
                &[
                    ("NOUN", "бо", vec![("й", noun("Nom")), ("я", noun("Gen"))]),
                    ("NOUN", "стро", vec![("й", noun("Nom")), ("ю", noun("Dat"))]),
                    ("DET", "мо", vec![("й", "Case=Nom".to_owned())]),
                ],
                &[("мой", "DET Case=Nom", 900_000)],
            )
        };
        let probs = |dict: Dictionary| -> Vec<f64> {
            dict.analyze("мой").iter().map(|a| a.prob).collect()
        };
        let one = probs(dict().with_words([("мой", "бой")]).unwrap());
        let two = probs(
            dict()
                .with_words([("мой", "бой"), ("мой", "строй")])
                .unwrap(),
        );
        assert_eq!((one.len(), &one), (2, &two));
    }

    #[test]
    fn own_words_in_query_spelling() {
        // Слово и образец приводятся, как запрос: ударение, мягкий перенос, неразрывный дефис
        // и латинские двойники не мешают найти слово
        let dict = lexicon()
            .with_words([
                ("бо\u{301}т", "кот"),
                ("xот", "ко\u{301}т"),
                ("ро\u{AD}т", "кот"),
                ("бот\u{2011}кот", "кот"),
            ])
            .unwrap();
        for (word, lemma) in [
            ("ботами", "бот"),
            ("хотами", "хот"),
            ("xотами", "хот"),
            ("ротами", "рот"),
            ("бот-котами", "бот-кот"),
            ("бот\u{2011}котами", "бот-кот"),
        ] {
            let analysis = &dict.analyze(word)[0];
            assert_eq!(
                (analysis.lemma.as_str(), analysis.source),
                (lemma, Source::Dictionary),
                "{word}"
            );
            assert!(dict.is_known(word), "{word}");
        }
        for word in [" бот", "бот\n", "бо т", "бо\u{200B}т"] {
            let Err(Error::Word(message)) = lexicon().with_words([(word, "кот")]) else {
                panic!("{word:?}");
            };
            assert!(message.contains("не найдется в тексте"), "{message}");
        }
    }

    #[test]
    fn forms_of_lexeme() {
        let dict = lexicon();
        let forms: Vec<String> = dict
            .forms(&dict.analyze("коту")[0])
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(
            forms,
            [
                "кот",
                "кота",
                "коту",
                "кота",
                "котом",
                "коте",
                "коты",
                "котов",
                "котам",
                "котов",
                "котами",
                "котах"
            ]
        );
        assert!(
            dict.forms(&dict.analyze("коту")[0])
                .iter()
                .all(|form| form.prob == 1.0 && form.lemma == "кот")
        );
    }

    #[test]
    fn inflect_keeps_other_features() {
        let dict = lexicon();
        let inflect = |word: &str, feats: &[(&str, &str)]| {
            dict.inflect(&dict.analyze(word)[0], feats)
                .map(|form| form.word)
        };
        assert_eq!(inflect("кот", &[("Case", "Gen")]).as_deref(), Some("кота"));
        assert_eq!(
            inflect("кот", &[("Case", "Gen"), ("Number", "Plur")]).as_deref(),
            Some("котов")
        );
        assert_eq!(
            inflect("котам", &[("Number", "Sing")]).as_deref(),
            Some("коту")
        );
        assert_eq!(inflect("кот", &[]).as_deref(), Some("кот"));
        // Одушевленность, которой нет ни в разборе, ни в запросе, не выбирается: «новые», а не «новых»
        assert_eq!(
            inflect("новые", &[("Case", "Acc")]).as_deref(),
            Some("новые")
        );
        assert_eq!(inflect("кот", &[("Tense", "Past")]), None);
        assert_eq!(inflect("сутки", &[("Number", "Sing")]), None);
        assert_eq!(
            inflect("нести", &[("Tense", "Past")]).as_deref(),
            Some("несла")
        );
    }

    #[test]
    fn agree_with_number() {
        let dict = lexicon();
        let agree = |analysis: &Analysis, numbers: &[u64]| -> Vec<Option<String>> {
            numbers
                .iter()
                .map(|&number| dict.agree(analysis, number).map(|form| form.word))
                .collect()
        };
        let words = |words: &[&str]| -> Vec<Option<String>> {
            words
                .iter()
                .map(|word| (*word != "-").then(|| word.to_string()))
                .collect()
        };
        let numbers = [0, 1, 2, 5, 11, 12, 21, 22, 25, 101, 112, 1_000_002];
        assert_eq!(
            agree(&dict.analyze("кот")[0], &numbers),
            words(&[
                "котов",
                "кот",
                "кота",
                "котов",
                "котов",
                "котов",
                "кот",
                "кота",
                "котов",
                "кот",
                "котов",
                "кота"
            ])
        );
        // Винительный падеж одушевленных: «вижу двух котов», но «вижу двадцать два кота»
        assert_eq!(
            agree(&find(&dict, "кота", "Case=Acc|Number=Sing"), &[1, 2, 5, 22]),
            words(&["кота", "котов", "котов", "кота"])
        );
        assert_eq!(
            agree(&dict.analyze("коту")[0], &[1, 2, 21]),
            words(&["коту", "котам", "коту"])
        );
        assert_eq!(
            agree(&dict.analyze("сутки")[0], &[1, 2, 5]),
            words(&["сутки", "суток", "суток"])
        );
        assert_eq!(
            agree(&dict.analyze("молоко")[0], &[1, 2, 5]),
            words(&["молоко", "молока", "-"])
        );
        assert_eq!(
            agree(&dict.analyze("новый")[0], &[1, 2, 5]),
            words(&["новый", "новых", "новых"])
        );
        assert_eq!(
            agree(&dict.analyze("новая")[0], &[1, 2, 5]),
            words(&["новая", "новые", "новых"])
        );
        assert_eq!(agree(&dict.analyze("новую")[0], &[2]), words(&["новые"]));
        assert_eq!(agree(&dict.analyze("нести")[0], &[2]), words(&["-"]));
        // Существительное с адъективным склонением согласуется как прилагательное
        assert_eq!(
            agree(&dict.analyze("учёный")[0], &[1, 2, 5, 22]),
            words(&["учёный", "учёных", "учёных", "учёных"])
        );
        assert_eq!(
            agree(&find(&dict, "учёного", "Case=Acc|Number=Sing"), &[2, 22]),
            words(&["учёных", "учёных"])
        );
        assert_eq!(agree(&dict.analyze("кот")[0], &[22]), words(&["кота"]));
    }

    #[test]
    fn spellings_up_to_yo() {
        // Угадывание в смешанном регистре ставит заглавную «Ё» на место заглавной «Е»
        assert!(same_up_to_yo("жилем", "жилём") && same_up_to_yo("ЖилЕм", "ЖилЁм"));
        assert!(!same_up_to_yo("ЖилЕм", "Жилём") && !same_up_to_yo("жилем", "жилам"));
    }

    #[test]
    fn merged_analyses_share_forms() {
        // «мешать» непереходный и переходный - одна таблица, как их объединяет сборщик;
        // причастие есть только у переходного
        let mut builder = Builder::new(
            vec!["INFN".into(), "VERB".into(), "PRTF".into()],
            vec![String::new()],
            vec!["ть".into(), "л".into(), "емый".into()],
        );
        let tags = [
            ("VerbForm=Inf", "intr"),
            ("Tense=Past|VerbForm=Fin", "intr"),
            ("VerbForm=Inf", "tran"),
            ("Tense=Past|VerbForm=Fin", "tran"),
            ("VerbForm=Part|Voice=Pass", "tran"),
        ];
        let cells: Vec<(u16, u16)> = tags
            .iter()
            .map(|&(feats, label)| (0, builder.add_ud_tag("VERB", feats, &[label]).unwrap()))
            .collect();
        let paradigm = builder
            .add_paradigm(&[(0, 0, 0), (1, 1, 0), (0, 0, 0), (1, 1, 0), (2, 2, 0)])
            .unwrap();
        let table = builder.add_table(paradigm, &cells).unwrap();
        builder
            .add_word("мешать", &[entry(table, 0), entry(table, 2)])
            .unwrap();
        builder
            .add_word("мешал", &[entry(table, 1), entry(table, 3)])
            .unwrap();
        builder.add_word("мешаемый", &[entry(table, 4)]).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let analyses = dict.analyze("мешать");
        assert_eq!(analyses.len(), 1);
        for word in ["мешать", "мешаемый"] {
            let words: Vec<String> = dict
                .forms(&dict.analyze(word)[0])
                .into_iter()
                .map(|form| form.word)
                .collect();
            assert_eq!(words, ["мешать", "мешал", "мешаемый"], "{word}");
        }
        let found = dict.inflect(&analyses[0], &[("Voice", "Pass")]).unwrap();
        assert_eq!(found.word, "мешаемый");
        let analysis = |word: &str, form: u16, source: Source| {
            dict.analysis(word, Entry { table, form }, source)
        };
        assert_eq!(analysis("мешаемый", 4, Source::Dictionary), Some(found));
        assert_eq!(analysis("мешаемый", 1, Source::Dictionary), None);
        assert_eq!(analysis("мешал", 4, Source::Dictionary), None);
        // Угаданному разбору словоформа в словаре не нужна
        let guessed = analysis("Зашаемый", 4, Source::Guess).unwrap();
        assert_eq!(
            (guessed.lemma.as_str(), guessed.source),
            ("зашать", Source::Guess)
        );
        assert_eq!(analysis("Зашаемый", 4, Source::Dictionary), None);
    }

    /// Словарь для угадывания: нарицательное «кот» (кот, кота) и фамилия «петров» (петров, петрова)
    fn guessing() -> Dictionary {
        let mut builder = Builder::new(
            vec!["NOUN,nomn".into(), "NOUN,gent".into()],
            vec![String::new()],
            vec![String::new(), "а".into()],
        );
        let nominative = builder.add_ud_tag("NOUN", "Case=Nom", &[]).unwrap();
        let genitive = builder.add_ud_tag("NOUN", "Case=Gen", &[]).unwrap();
        let surname = builder
            .add_ud_tag("PROPN", "Case=Gen|NameType=Sur", &[])
            .unwrap();
        let surname_nominative = builder
            .add_ud_tag("PROPN", "Case=Nom|NameType=Sur", &[])
            .unwrap();
        let paradigm = builder.add_paradigm(&[(0, 0, 0), (1, 1, 0)]).unwrap();
        let common = builder
            .add_table(paradigm, &[(0, nominative), (0, genitive)])
            .unwrap();
        let proper = builder
            .add_table(paradigm, &[(0, surname_nominative), (0, surname)])
            .unwrap();
        builder.add_word("кот", &[entry(common, 0)]).unwrap();
        builder.add_word("кота", &[entry(common, 1)]).unwrap();
        builder.add_word("петров", &[entry(proper, 0)]).unwrap();
        builder.add_word("петрова", &[entry(proper, 1)]).unwrap();
        builder
            .add_guesses(GuessIndex::Common, "а", &[(entry(common, 1), 5)])
            .unwrap();
        builder
            .add_guesses(GuessIndex::Common, "ова", &[(entry(common, 1), 1)])
            .unwrap();
        builder
            .add_guesses(GuessIndex::Proper, "ова", &[(entry(proper, 1), 3)])
            .unwrap();
        Dictionary::from_bytes(&builder.build().unwrap()).unwrap()
    }

    #[test]
    fn guess_by_longest_ending() {
        let dict = guessing();
        let analyses = dict.analyze("зумова");
        let summary: Vec<(&str, &str, Source)> = analyses
            .iter()
            .map(|a| (a.word.as_str(), a.lemma.as_str(), a.source))
            .collect();
        assert_eq!(summary, [("зумова", "зумов", Source::Guess)]);
        assert_eq!(dict.pos(analyses[0].ud), "NOUN");
        assert_eq!(dict.analyze("зума")[0].lemma, "зум");
        assert_eq!(dict.analyze("зумы"), dict.fallback("зумы"));
        assert_eq!(dict.analyze("зумова1"), dict.fallback("зумова1"));
    }

    #[test]
    fn capitalized_guess_prefers_proper_nouns() {
        let dict = guessing();
        let analyses = dict.analyze("Зумова");
        let summary: Vec<(&str, &str, &str)> = analyses
            .iter()
            .map(|a| (a.word.as_str(), a.lemma.as_str(), dict.pos(a.ud)))
            .collect();
        assert_eq!(
            summary,
            [("Зумова", "зумов", "PROPN"), ("Зумова", "зумов", "NOUN")]
        );
        assert!((analyses[0].prob - 0.91).abs() < 1e-12);
        // Формы угаданного слова сохраняют заглавную букву
        let forms: Vec<String> = dict
            .forms(&analyses[0])
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["Зумов", "Зумова"]);
        // У слова капсом с нулевым окончанием окончания форм тоже заглавные
        let entry = Entry {
            form: 0,
            ..analyses[0].entry.unwrap()
        };
        let caps = dict.analysis("ЗУМОВ", entry, Source::Guess).unwrap();
        let forms: Vec<String> = dict.forms(&caps).into_iter().map(|f| f.word).collect();
        assert_eq!(forms, ["ЗУМОВ", "ЗУМОВА"]);
        // Слитное название в смешанном регистре угадывается, как слово с заглавной
        let mixed: Vec<(String, f64)> = dict
            .analyze("ЗуМова")
            .into_iter()
            .map(|a| (a.lemma, a.prob))
            .collect();
        let capitalized: Vec<(String, f64)> =
            analyses.into_iter().map(|a| (a.lemma, a.prob)).collect();
        assert_eq!(mixed, capitalized);
    }

    #[test]
    fn guess_with_yo_in_suffix() {
        // «котёнок» - суффикс с «ё», «баранка» - с «е»; окончание через «е» находит обе, но первой
        // идет «баранка», окончание с «ё» - только «котёнок»
        let mut builder = Builder::new(
            vec!["NOUN,nomn".into(), "NOUN,gent".into()],
            vec![String::new()],
            vec!["ёнок".into(), "ёнка".into(), "енка".into()],
        );
        let nominative = builder.add_ud_tag("NOUN", "Case=Nom", &[]).unwrap();
        let genitive = builder.add_ud_tag("NOUN", "Case=Gen", &[]).unwrap();
        let young = builder.add_paradigm(&[(0, 0, 0), (1, 1, 0)]).unwrap();
        let young = builder
            .add_table(young, &[(0, nominative), (0, genitive)])
            .unwrap();
        let plain = builder.add_paradigm(&[(2, 0, 0)]).unwrap();
        let plain = builder.add_table(plain, &[(0, nominative)]).unwrap();
        builder.add_word("котёнок", &[entry(young, 0)]).unwrap();
        builder.add_word("котёнка", &[entry(young, 1)]).unwrap();
        builder.add_word("баренка", &[entry(plain, 0)]).unwrap();
        builder
            .add_guesses(
                GuessIndex::Common,
                "енка",
                &[(entry(plain, 0), 5), (entry(young, 1), 3)],
            )
            .unwrap();
        builder
            .add_guesses(GuessIndex::Common, "ёнка", &[(entry(young, 1), 3)])
            .unwrap();
        builder
            .add_guesses(GuessIndex::Common, "нка", &[(entry(young, 1), 3)])
            .unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let summary = |word: &str| -> Vec<(String, String)> {
            dict.analyze(word)
                .into_iter()
                .map(|a| (a.word, a.lemma))
                .collect()
        };
        assert_eq!(
            summary("слонёнка"),
            [("слонёнка".to_string(), "слонёнок".to_string())]
        );
        let written_with_e = summary("слоненка");
        assert_eq!(
            written_with_e[1],
            ("слонёнка".to_string(), "слонёнок".to_string())
        );
        let forms: Vec<String> = dict
            .forms(&dict.analyze("слонёнка")[0])
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["слонёнок", "слонёнка"]);
        // Суффикс на все слово: регистр букв сохраняется, меняется только «е» на месте «ё»
        let whole = dict.analyze("Енка");
        assert_eq!(
            (whole[0].word.as_str(), whole[0].lemma.as_str()),
            ("Ёнка", "ёнок")
        );
        let forms: Vec<String> = dict
            .forms(&whole[0])
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["Ёнок", "Ёнка"]);
        // Длинное слово капсом: написание угаданного разбора - с «ё» суффикса, формы капсом
        let caps = dict.analyze("СЛОНЕНКА");
        let young = caps.iter().find(|a| a.lemma == "слонёнок").unwrap();
        assert_eq!(young.word, "СЛОНЁНКА");
        let forms: Vec<String> = dict.forms(young).into_iter().map(|f| f.word).collect();
        assert_eq!(forms, ["СЛОНЁНОК", "СЛОНЁНКА"]);
    }

    #[test]
    fn capitalized_guess_with_prefixed_form() {
        // Сравнительная степень с приставкой «по»: «Покринжовее», а не «поКринжовее»
        let mut builder = Builder::new(
            vec!["ADJF".into(), "COMP".into()],
            vec![String::new(), "по".into()],
            vec!["ый".into(), "ее".into()],
        );
        let positive = builder.add_ud_tag("ADJ", "Degree=Pos", &[]).unwrap();
        let comparative = builder.add_ud_tag("ADJ", "Degree=Cmp", &[]).unwrap();
        let paradigm = builder.add_paradigm(&[(0, 0, 0), (1, 1, 1)]).unwrap();
        let table = builder
            .add_table(paradigm, &[(0, positive), (0, comparative)])
            .unwrap();
        builder.add_word("новый", &[entry(table, 0)]).unwrap();
        builder.add_word("поновее", &[entry(table, 1)]).unwrap();
        builder
            .add_guesses(GuessIndex::Common, "ый", &[(entry(table, 0), 3)])
            .unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let analysis = &dict.analyze("Кринжовый")[0];
        let forms: Vec<String> = dict
            .forms(analysis)
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["Кринжовый", "Покринжовее"]);
        let comparative = dict.inflect(analysis, &[("Degree", "Cmp")]).unwrap();
        assert_eq!(comparative.word, "Покринжовее");
        let entry = comparative.entry.unwrap();
        let restored = dict.analysis("Покринжовее", entry, Source::Guess).unwrap();
        assert_eq!(restored.lemma, "кринжовый");
        // Формы формы с приставкой: заглавная первая буква сохраняется
        let forms: Vec<String> = dict
            .forms(&restored)
            .into_iter()
            .map(|form| form.word)
            .collect();
        assert_eq!(forms, ["Кринжовый", "Покринжовее"]);
        let positive = dict.inflect(&restored, &[("Degree", "Pos")]).unwrap();
        assert_eq!(positive.word, "Кринжовый");
    }

    #[test]
    fn broken_guess_records_are_rejected() {
        let data = guessing_bytes();
        assert_eq!(
            Dictionary::from_bytes(&data).unwrap().analyze("зума")[0].lemma,
            "зума"
        );
        // Нулевое число словоформ не дает вероятности NaN, а обрезанный список и несуществующая
        // таблица - ошибка формата при загрузке
        for records in [vec![1, 0, 0], vec![]] {
            let mut sections = format::tests::read(&data);
            sections.guess_records = records;
            let dict = Dictionary::from_bytes(&format::write(&sections)).unwrap();
            assert_eq!(dict.analyze("зума"), dict.fallback("зума"));
        }
        for records in [vec![9, 0], vec![1, 0x0007_0000, 2], vec![300]] {
            let mut sections = format::tests::read(&data);
            sections.guess_records = records;
            assert!(Dictionary::from_bytes(&format::write(&sections)).is_err());
        }
    }

    /// Словарь с одним окончанием для угадывания
    fn guessing_bytes() -> Vec<u8> {
        let mut builder = Builder::new(
            vec!["NOUN".into()],
            vec![String::new()],
            vec![String::new(), "а".into()],
        );
        builder.add_ud_tag("NOUN", "", &[]).unwrap();
        let table = lexeme_table(&mut builder, &[(1, 0, 0)]);
        builder.add_word("кота", &[entry(table, 0)]).unwrap();
        builder
            .add_guesses(GuessIndex::Common, "а", &[(entry(table, 0), 2)])
            .unwrap();
        builder.build().unwrap()
    }

    #[test]
    fn yo_spelled_as_e_and_case() {
        let dict = dictionary();
        assert_eq!(summary(&dict.analyze("еж")), [("ёж", "ёж", 0)]);
        assert_eq!(summary(&dict.analyze("ЁЖ")), [("ёж", "ёж", 0)]);
        let words: Vec<_> = dict.analyze("все").iter().map(|a| a.word.clone()).collect();
        assert_eq!(words, ["все", "всё"]);
        assert_eq!(summary(&dict.analyze("всё")), [("всё", "всё", 0)]);
    }

    #[test]
    fn probabilities_order_analyses() {
        let dict = dictionary();
        let analyses = dict.analyze("Стали");
        assert_eq!(
            summary(&analyses),
            [
                ("стали", "стать", 2),
                ("стали", "сталь", 3),
                ("стали", "сталь", 4)
            ]
        );
        let probs: Vec<f64> = analyses.iter().map(|a| a.prob).collect();
        assert!((probs[0] - 0.97).abs() < 1e-12 && (probs[2] - 0.01).abs() < 1e-12);
        assert!((probs.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn uniform_without_probabilities() {
        let dict = dictionary();
        let probs: Vec<f64> = dict.analyze("все").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [0.5, 0.5]);
    }

    #[test]
    fn same_tag_shares_probability() {
        let mut builder = builder(&["NOUN", "VERB"], &[""], &["", "а", "о"]);
        let first = lexeme_table(&mut builder, &[(0, 0, 0)]);
        let second = lexeme_table(&mut builder, &[(1, 1, 0), (0, 0, 0)]);
        let third = lexeme_table(&mut builder, &[(2, 1, 0), (0, 0, 0)]);
        builder
            .add_word("лук", &[entry(first, 0), entry(second, 1), entry(third, 1)])
            .unwrap();
        builder.add_probability("лук", 0, 600_000).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let probs: Vec<f64> = dict.analyze("лук").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [1.0 / 3.0; 3]);
    }

    #[test]
    fn same_ud_analyses_merge() {
        let mut builder = Builder::new(
            vec!["CONJ".into(), "ADVB".into()],
            vec![String::new()],
            vec![String::new()],
        );
        let plain = builder.add_ud_tag("ADV", "Degree=Pos", &[]).unwrap();
        let labelled = builder.add_ud_tag("ADV", "Degree=Pos", &["Dmns"]).unwrap();
        let conjunction = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
        let conjunction = builder.add_table(conjunction, &[(0, plain)]).unwrap();
        let adverb = builder.add_paradigm(&[(0, 1, 0)]).unwrap();
        let adverb = builder.add_table(adverb, &[(0, labelled)]).unwrap();
        builder
            .add_word("так", &[entry(conjunction, 0), entry(adverb, 0)])
            .unwrap();
        builder.add_probability("так", 1, 750_000).unwrap();
        builder.add_probability("так", 0, 250_000).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let analyses = dict.analyze("так");
        assert_eq!(analyses.len(), 1);
        let labels: Vec<&str> = dict.ud_tag(analyses[0].ud).unwrap().labels().collect();
        assert_eq!(labels, ["Dmns"]);
        assert!((analyses[0].prob - 1.0).abs() < 1e-12);
        let probs: Vec<f64> = dict.opencorpora("так").iter().map(|a| a.3).collect();
        assert_eq!(probs, [0.75, 0.25]);
    }

    #[test]
    fn uniform_probabilities_after_merge() {
        let mut builder = Builder::new(
            vec!["ADJF,gent".into(), "ADJF,datv".into(), "NPRO".into()],
            vec![String::new()],
            vec![String::new()],
        );
        let abbreviation = builder.add_ud_tag("ADJ", "Abbr=Yes", &[]).unwrap();
        let pronoun = builder.add_ud_tag("PRON", "Case=Loc", &[]).unwrap();
        let adjective = builder.add_paradigm(&[(0, 0, 0), (0, 1, 0)]).unwrap();
        let adjective = builder
            .add_table(adjective, &[(0, abbreviation), (0, abbreviation)])
            .unwrap();
        let personal = builder.add_paradigm(&[(0, 2, 0)]).unwrap();
        let personal = builder.add_table(personal, &[(0, pronoun)]).unwrap();
        builder
            .add_word(
                "нем",
                &[entry(adjective, 0), entry(adjective, 1), entry(personal, 0)],
            )
            .unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let probs: Vec<f64> = dict.analyze("нем").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [0.5, 0.5]);
        let probs: Vec<f64> = dict.opencorpora("нем").iter().map(|a| a.3).collect();
        assert_eq!(probs, [1.0 / 3.0; 3]);
    }

    #[test]
    fn ud_tags() {
        let mut builder = Builder::new(
            vec!["NOUN".into()],
            vec![String::new()],
            vec![String::new()],
        );
        builder
            .add_ud_tag("NOUN", "Case=Gen|Number=Sing", &["Infr", "Slng"])
            .unwrap();
        builder.add_ud_tag("PART", "", &[]).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let noun = dict.ud_tag(0).unwrap();
        assert_eq!(noun.pos(), "NOUN");
        assert_eq!(noun.feats_string(), "Case=Gen|Number=Sing");
        assert_eq!(noun.labels().collect::<Vec<_>>(), ["Infr", "Slng"]);
        let particle = dict.ud_tag(1).unwrap();
        assert!(particle.feats().len() == 0 && particle.labels().len() == 0);
    }

    #[test]
    fn tag_probability_shared_within_spelling() {
        let mut builder = builder(&["NOUN,datv", "NOUN,loct"], &[""], &[""]);
        let dative = lexeme_table(&mut builder, &[(0, 0, 0)]);
        let locative = lexeme_table(&mut builder, &[(0, 1, 0)]);
        builder.add_word("алеше", &[entry(dative, 0)]).unwrap();
        builder
            .add_word("алёше", &[entry(dative, 0), entry(locative, 0)])
            .unwrap();
        builder.add_probability("алёше", 0, 666_667).unwrap();
        builder.add_probability("алёше", 1, 333_333).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        let summary: Vec<(String, u16, f64)> = dict
            .analyze("алеше")
            .into_iter()
            .map(|a| (a.word, a.tag.unwrap(), (a.prob * 1000.0).round() / 1000.0))
            .collect();
        // У написания из запроса частот нет: его разбор получает среднюю долю разборов «алёше»
        assert_eq!(
            summary,
            [
                ("алёше".to_string(), 0, 0.444),
                ("алеше".to_string(), 0, 0.333),
                ("алёше".to_string(), 1, 0.222),
            ]
        );
    }

    #[test]
    fn probabilities_of_dictionary_spelling() {
        let mut builder = builder(&["ADVB", "PRCL"], &[""], &[""]);
        let adverb = lexeme_table(&mut builder, &[(0, 0, 0)]);
        let particle = lexeme_table(&mut builder, &[(0, 1, 0)]);
        builder
            .add_word("ещё", &[entry(adverb, 0), entry(particle, 0)])
            .unwrap();
        builder.add_probability("ещё", 1, 750_000).unwrap();
        builder.add_probability("ещё", 0, 250_000).unwrap();
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        for word in ["ещё", "еще"] {
            let analyses = dict.analyze(word);
            let tags: Vec<u16> = analyses.iter().map(|a| a.tag.unwrap()).collect();
            assert_eq!(tags, [1, 0], "{word}");
            assert!((analyses[0].prob - 0.75).abs() < 1e-12, "{word}");
        }
    }

    #[test]
    fn unknown_words() {
        let dict = dictionary();
        assert_eq!(dict.analyze("вайб"), dict.fallback("вайб"));
        assert!(dict.analyze("").is_empty());
        assert_eq!(dict.analyze("ежаа"), dict.fallback("ежаа"));
        assert!(!dict.is_known("вайб"));
        assert!(dict.is_known("Еж"));
    }

    #[test]
    fn open_reads_file() {
        let path = std::env::temp_dir().join(format!("rumorph-{}.dict", std::process::id()));
        std::fs::write(&path, dictionary_bytes()).unwrap();
        let dict = Dictionary::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(dict.word_count(), dictionary().word_count());
        assert_eq!(
            summary(&dict.analyze("ежа")),
            summary(&dictionary().analyze("ежа"))
        );
    }

    #[test]
    fn open_reports_directory_as_io_error() {
        let path = std::env::temp_dir().join(format!("rumorph-dir-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let result = Dictionary::open(&path);
        std::fs::remove_dir(&path).unwrap();
        assert!(matches!(result, Err(Error::Io(_))));
    }

    #[test]
    fn metadata() {
        let dict = dictionary();
        assert_eq!(dict.meta(), "тест");
        assert_eq!(dict.word_count(), 6);
        assert_eq!(dict.tag(5), Some(TAGS[5]));
        assert_eq!(dict.tags().len(), TAGS.len());
        assert_eq!(dict.ud_tag(5).unwrap().feats_string(), "Tag=5");
    }

    fn sections() -> Sections {
        Sections {
            tags: vec!["NOUN".into()],
            ud_strings: vec!["NOUN".into()],
            ud_tags: vec![0, 0, 0],
            prefixes: vec![String::new()],
            suffixes: vec![String::new()],
            paradigm_offsets: vec![0, 3],
            paradigms: vec![0, 0, 0],
            tables: vec![0, 0],
            words: fst::MapBuilder::memory().into_inner().unwrap(),
            guesses: fst::MapBuilder::memory().into_inner().unwrap(),
            ..Sections::default()
        }
    }

    #[test]
    fn rejects_broken_paradigms_and_ud_tags() {
        assert!(Dictionary::from_bytes(&format::write(&sections())).is_ok());
        let broken: [fn(&mut Sections); 14] = [
            |s| s.paradigms = vec![0, 1, 0],
            |s| s.paradigms = vec![0, 0, 1],
            |s| s.paradigm_offsets = vec![0, 2],
            |s| s.paradigm_offsets = vec![1, 3],
            |s| s.paradigm_offsets = vec![0, 9, 3],
            |s| s.ud_tags = vec![0],
            |s| s.ud_tags = vec![1, 0, 0],
            |s| s.ud_strings = vec![String::new()],
            |s| s.tables = vec![1, 0],
            |s| s.tables = vec![0],
            |s| s.tables = vec![0, 0x0001_0000],
            // Тег UD правил («PUNCT»), которого нет в файле, таблица назвать не может
            |s| s.tables = vec![0, 1],
            |s| s.alphabet = "аа".into(),
            |s| s.prefixes = vec![],
        ];
        for (number, breaker) in broken.into_iter().enumerate() {
            let mut sections = sections();
            breaker(&mut sections);
            let result = Dictionary::from_bytes(&format::write(&sections));
            assert!(matches!(result, Err(Error::Format(_))), "{number}");
        }
    }

    #[test]
    fn broken_records_are_rejected() {
        let mut builder = builder(&["NOUN"], &[""], &[""]);
        let table = lexeme_table(&mut builder, &[(0, 0, 0)]);
        builder.add_word("кот", &[entry(table, 0)]).unwrap();
        builder.add_word("еж", &[entry(table, 0)]).unwrap();
        builder.add_word("ёж", &[entry(table, 0)]).unwrap();
        let data = builder.build().unwrap();
        assert_eq!(
            format::tests::read(&data).variants,
            [2, 0, 0, 0, 1, 0, 0, 0]
        );
        // Обрезанный или слишком длинный список и несуществующие таблица и форма - ошибка формата
        for records in [vec![5, 0], vec![1, 7], vec![1, 0x0005_0000], vec![256]] {
            let mut sections = format::tests::read(&data);
            sections.records = records;
            assert!(Dictionary::from_bytes(&format::write(&sections)).is_err());
        }
        // Частота несуществующего тега
        let mut sections = format::tests::read(&data);
        sections.probability_records = vec![1, 99, 5];
        assert!(Dictionary::from_bytes(&format::write(&sections)).is_err());
        // Список написаний: обрезанный, пустой, длиннее `MAX_SPELLINGS`, со ссылкой за пределы
        // списков разборов или частот
        let many = [vec![9], [0, 0, 0].repeat(9), vec![0]].concat();
        for variants in [
            vec![2, 0, 0, 0, 1, 0, 0],
            vec![0, 0],
            many,
            vec![1, 0, 0, 9, 0],
            vec![1, 0, 0, 0, 1, 0, 0, 0],
            vec![256],
        ] {
            let mut sections = format::tests::read(&data);
            sections.variants = variants;
            assert!(Dictionary::from_bytes(&format::write(&sections)).is_err());
        }
    }

    #[test]
    fn rejects_damaged_file() {
        let data = dictionary_bytes();
        let middle = format::layout(&data).unwrap().words.start + 1;
        let mut damaged = data.clone();
        damaged[middle] ^= 1;
        let err = Dictionary::from_bytes(&damaged).err().unwrap();
        assert!(err.to_string().contains("контрольная сумма"), "{err}");
    }

    #[test]
    fn limits_feature_count() {
        // У каждого тега свой признак: коды признаков занимают число признаков на тег
        let with_features = |count: usize| {
            let mut builder =
                Builder::new(vec!["X".into()], vec![String::new()], vec![String::new()]);
            let tags: Vec<u16> = (0..count)
                .map(|number| {
                    builder
                        .add_ud_tag("X", &format!("F{number}=V"), &[])
                        .unwrap()
                })
                .collect();
            let paradigm = builder.add_paradigm(&[(0, 0, 0)]).unwrap();
            let table = builder.add_table(paradigm, &[(0, tags[0])]).unwrap();
            builder.add_word("а", &[entry(table, 0)]).unwrap();
            Dictionary::from_bytes(&builder.build().unwrap())
        };
        // Правила добавляют свои признаки (NumForm, NumType, Foreign, Abbr, NameType, Degree)
        let dict = with_features(MAX_FEATURES - 6).unwrap();
        assert_eq!(dict.features().len(), MAX_FEATURES);
        let err = with_features(MAX_FEATURES - 5).err().unwrap();
        assert!(err.to_string().contains("признаков UD"), "{err}");
    }

    /// Контрольная сумма автомата, как ее считает `fst`: CRC32C с маской
    fn masked_crc32c(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0x82F6_3B78
                } else {
                    crc >> 1
                };
            }
        }
        let sum = !crc;
        (sum.rotate_right(15)).wrapping_add(0xA282_EAD8)
    }

    #[test]
    fn rejects_automaton_with_moved_root() {
        let mut sections = format::tests::read(&dictionary_bytes());
        let words = &mut sections.words;
        let end = words.len() - 4;
        let root = u64::from_le_bytes(words[end - 8..end].try_into().unwrap());
        assert_eq!(masked_crc32c(&words[..end]).to_le_bytes(), words[end..]);
        // Корень раньше настоящего и пересчитанная контрольная сумма: `Fst::new` такое пропускает
        words[end - 8..end].copy_from_slice(&(root - 1).to_le_bytes());
        let crc = masked_crc32c(&words[..end]);
        words[end..].copy_from_slice(&crc.to_le_bytes());
        let err = Dictionary::from_bytes(&format::write(&sections))
            .err()
            .unwrap();
        assert!(err.to_string().contains("адрес корня"), "{err}");
    }

    #[test]
    fn rejects_too_many_ud_tags() {
        let mut sections = sections();
        sections.ud_tags = [0, 0, 0].repeat(usize::from(u16::MAX) + 2);
        let err = Dictionary::from_bytes(&format::write(&sections))
            .err()
            .unwrap();
        assert!(err.to_string().contains("65 536 тегов"), "{err}");
        // Теги правил добавляются при загрузке, и вместе с ними тегов тоже не больше 65 536
        sections.ud_tags = [0, 0, 0].repeat(usize::from(u16::MAX) + 1);
        let err = Dictionary::from_bytes(&format::write(&sections))
            .err()
            .unwrap();
        assert!(err.to_string().contains("65 536 тегов"), "{err}");
    }

    /// Все, что можно спросить у словаря: поврежденный, но загруженный словарь не паникует
    fn exercise(dict: &Dictionary) {
        let words = [
            "ёж",
            "ежа",
            "еж",
            "стали",
            "наибольший",
            "все",
            "всё",
            "кота",
            "зума",
            "Зума",
            "XIX-го",
            "кот-то",
            "Д-д-да",
        ];
        for word in words {
            dict.is_known(word);
            dict.opencorpora(word);
            for analysis in dict.analyze(word) {
                dict.forms(&analysis);
                dict.inflect(&analysis, &[("Case", "Gen")]);
                dict.agree(&analysis, 5);
                dict.agree_fraction(&analysis);
            }
        }
        // Восстановление сохраненного разбора с любой ссылкой на таблицу
        for (table, form) in (0..8).flat_map(|table| (0..4).map(move |form| (table, form))) {
            for source in [Source::Dictionary, Source::Rule, Source::Guess] {
                if let Some(analysis) = dict.analysis("стали", entry(table, form), source) {
                    dict.forms(&analysis);
                }
            }
        }
    }

    #[test]
    fn corrupted_dictionary_never_panics() {
        for data in [dictionary_bytes(), guessing_bytes()] {
            exercise(&Dictionary::from_bytes(&data).unwrap());
            for len in 0..data.len() {
                assert!(Dictionary::from_bytes(&data[..len]).is_err(), "{len}");
            }
            // Испорченный байт ловит контрольная сумма, а с пересчитанной суммой - ошибка формата
            // или словарь, который не паникует. Автоматы `fst` проверяет только по своей сумме:
            // испорченный и затем подписанный автомат паникует при обходе
            let layout = format::layout(&data).unwrap();
            let automata = [layout.words.clone(), layout.guesses.clone()];
            for at in 0..data.len() {
                let mut corrupted = data.clone();
                corrupted[at] ^= 0xFF;
                assert!(Dictionary::from_bytes(&corrupted).is_err(), "{at}");
                if automata.iter().any(|range| range.contains(&at)) {
                    continue;
                }
                if let Ok(dict) = Dictionary::from_bytes(&format::tests::with_checksum(corrupted)) {
                    exercise(&dict);
                }
            }
            // Числа секций: ошибка формата или словарь, который не паникует
            let numbers: [fn(&mut Sections) -> &mut Vec<u32>; 6] = [
                |s| &mut s.paradigm_offsets,
                |s| &mut s.tables,
                |s| &mut s.records,
                |s| &mut s.variants,
                |s| &mut s.probability_records,
                |s| &mut s.guess_records,
            ];
            let mut original = format::tests::read(&data);
            for section in numbers {
                for at in 0..section(&mut original).len() {
                    let number = section(&mut original)[at];
                    for value in [
                        0,
                        1,
                        number.wrapping_add(1),
                        number.wrapping_sub(1),
                        0xFFFF,
                        0x1_0000,
                        u32::MAX,
                    ] {
                        let mut sections = format::tests::read(&data);
                        section(&mut sections)[at] = value;
                        if let Ok(dict) = Dictionary::from_bytes(&format::write(&sections)) {
                            exercise(&dict);
                        }
                    }
                }
            }
            let numbers: [fn(&mut Sections) -> &mut Vec<u16>; 2] =
                [|s| &mut s.paradigms, |s| &mut s.ud_tags];
            for section in numbers {
                for at in 0..section(&mut original).len() {
                    for value in [0, 1, 2, u16::MAX] {
                        let mut sections = format::tests::read(&data);
                        section(&mut sections)[at] = value;
                        if let Ok(dict) = Dictionary::from_bytes(&format::write(&sections)) {
                            exercise(&dict);
                        }
                    }
                }
            }
            // Секции одного типа местами
            let swaps: [fn(&mut Sections); 10] = [
                |s| std::mem::swap(&mut s.words, &mut s.guesses),
                |s| std::mem::swap(&mut s.prefixes, &mut s.suffixes),
                |s| std::mem::swap(&mut s.tags, &mut s.ud_strings),
                |s| std::mem::swap(&mut s.tags, &mut s.suffixes),
                |s| std::mem::swap(&mut s.records, &mut s.guess_records),
                |s| std::mem::swap(&mut s.records, &mut s.probability_records),
                |s| std::mem::swap(&mut s.records, &mut s.variants),
                |s| std::mem::swap(&mut s.tables, &mut s.records),
                |s| std::mem::swap(&mut s.paradigm_offsets, &mut s.tables),
                |s| std::mem::swap(&mut s.paradigms, &mut s.ud_tags),
            ];
            for swap in swaps {
                let mut sections = format::tests::read(&data);
                swap(&mut sections);
                if let Ok(dict) = Dictionary::from_bytes(&format::write(&sections)) {
                    exercise(&dict);
                }
            }
        }
    }

    #[test]
    fn broken_probability_offset_gives_uniform_probabilities() {
        let data = dictionary_bytes();
        let dict = Dictionary::from_bytes(&data).unwrap();
        let key = dict.alphabet.input_key("стали").unwrap();
        let mut sections = format::tests::read(&data);
        let mut words = fst::MapBuilder::memory();
        let map = fst::Map::new(sections.words.clone()).unwrap();
        let mut stream = fst::IntoStreamer::into_stream(&map);
        while let Some((word, value)) = fst::Streamer::next(&mut stream) {
            let value = match WordValue::unpack(value) {
                WordValue::Single { yo, records, .. } if word == key.bytes() => WordValue::Single {
                    yo,
                    records,
                    probabilities: Some(77_777),
                }
                .pack()
                .unwrap(),
                _ => value,
            };
            words.insert(word, value).unwrap();
        }
        sections.words = words.into_inner().unwrap();
        let dict = Dictionary::from_bytes(&format::write(&sections)).unwrap();
        let probs: Vec<f64> = dict.analyze("стали").iter().map(|a| a.prob).collect();
        assert_eq!(probs, [1.0 / 3.0; 3]);
    }

    #[test]
    fn normalizes_stress_and_soft_hyphens() {
        assert_eq!(normalized_input("Москва\u{301}"), Some("Москва".into()));
        assert_eq!(normalized_input("за\u{300}мок"), Some("замок".into()));
        assert_eq!(normalized_input("при\u{ad}вет"), Some("привет".into()));
        assert_eq!(normalized_input("при\u{2060}вет"), Some("привет".into()));
        assert_eq!(normalized_input("\u{feff}привет"), Some("привет".into()));
        // Знак ударения в составленной букве (NFC): «ѐ» и «ѝ»
        assert_eq!(normalized_input("вс\u{450}"), Some("все".into()));
        assert_eq!(normalized_input("\u{40d}ван"), Some("Иван".into()));
        assert_eq!(normalized_input("\u{400}\u{45d}"), Some("Еи".into()));
        // Буквы-двойники - письменностью большинства букв отрезка
        assert_eq!(normalized_input("cтали"), Some("стали".into()));
        assert_eq!(normalized_input("Рoccия"), Some("Россия".into()));
        assert_eq!(normalized_input("сedar"), Some("cedar".into()));
        assert_eq!(normalized_input("ХIX-го"), Some("XIX-го".into()));
        assert_eq!(normalized_input("cтa\u{301}ли"), Some("стали".into()));
        // Письменность задают буквы без двойника, а не большинство
        assert_eq!(normalized_input("eгo"), Some("его".into()));
        assert_eq!(normalized_input("Hoвocти"), Some("Новости".into()));
        assert_eq!(normalized_input("ХХI"), Some("XXI".into()));
        assert_eq!(normalized_input("СаМgВ6O8"), Some("CaMgB6O8".into()));
        // Мягкий перенос и соединитель слов не делят отрезок
        assert_eq!(normalized_input("в\u{2060}ceх"), Some("всех".into()));
        assert_eq!(normalized_input("дoм\u{ad}a"), Some("дома".into()));
        assert_eq!(normalized_input("p\u{450}"), Some("ре".into()));
        for word in ["ТНК-BP", "T-клетки", "Газета.ru", "Юрiю", "iPhone", "стали"]
        {
            assert_eq!(normalized_input(word), None, "{word}");
        }
        // Знак ударения после латинской буквы - не ударение русского слова
        assert_eq!(normalized_input("cafe\u{301}"), None);
        assert_eq!(normalized_input("Москва"), None);
    }

    #[test]
    fn probabilities_of_table_forms() {
        // «стали» - форма «стать» и несклоняемое «стали» с одним тегом: доли по тегу их не
        // различают, доли по формам таблиц - различают
        let mut builder = builder(&["VERB"], &[""], &["", "ть", "ли"]);
        let become_ = lexeme_table(&mut builder, &[(1, 0, 0), (2, 0, 0)]);
        let other = lexeme_table(&mut builder, &[(0, 0, 0)]);
        for (word, entries) in [
            ("стать", vec![entry(become_, 0)]),
            ("стали", vec![entry(become_, 1), entry(other, 0)]),
        ] {
            builder.add_word(word, &entries).unwrap();
        }
        builder.add_probability("стали", 0, 1_000_000).unwrap();
        let lemmas = |builder: &Builder| -> Vec<(String, f64)> {
            let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
            dict.analyze("стали")
                .into_iter()
                .map(|analysis| (analysis.lemma, analysis.prob))
                .collect()
        };
        assert_eq!(
            lemmas(&builder),
            [("стать".to_owned(), 0.5), ("стали".to_owned(), 0.5)]
        );
        builder
            .set_entry_probabilities(
                "стали",
                &[(entry(other, 0), 900_000), (entry(become_, 1), 100_000)],
            )
            .unwrap();
        assert_eq!(
            lemmas(&builder),
            [("стали".to_owned(), 0.9), ("стать".to_owned(), 0.1)]
        );
        // Список по формам заменяет список по тегам, а не дополняется им
        assert!(builder.add_probability("стали", 0, 1).is_err());
        assert!(
            builder
                .set_entry_probabilities("стали", &[(entry(9, 0), 1)])
                .is_err()
        );
        assert!(
            builder
                .set_entry_probabilities("стали", &[(entry(other, 5), 1)])
                .is_err()
        );
    }

    #[test]
    fn yo_spellings_of_one_key() {
        // «трёхзвёздный» - с двумя «ё», «трехзвездный» - без, «трёхзвездный» - с одной
        let mut builder = builder(&["ADJF"], &[""], &[""]);
        let table = lexeme_table(&mut builder, &[(0, 0, 0)]);
        for word in ["трёхзвёздный", "трехзвездный", "трехзвёздный"]
        {
            builder.add_word(word, &[entry(table, 0)]).unwrap();
        }
        let dict = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        assert_eq!(dict.word_count(), 3);
        let words = |word: &str| -> Vec<String> {
            dict.analyze(word).into_iter().map(|a| a.word).collect()
        };
        // Сначала точное написание, затем варианты с «ё» по местам слева направо
        assert_eq!(
            words("трехзвездный"),
            ["трехзвездный", "трёхзвёздный", "трехзвёздный"]
        );
        assert_eq!(words("трёхзвездный"), ["трёхзвёздный"]);
        assert_eq!(words("трехзвёздный"), ["трехзвёздный", "трёхзвёздный"]);
        assert_eq!(words("ТРЁХЗВЁЗДНЫЙ"), ["трёхзвёздный"]);
        assert!(dict.is_known("трёхзвездный") && dict.is_known("Трехзвездный"));
        assert!(!dict.is_known("трехзвездныи"));
        // Восстановление разбора - только по точному написанию
        for word in ["трёхзвёздный", "трехзвездный", "трехзвёздный"]
        {
            assert!(
                dict.analysis(word, entry(table, 0), Source::Dictionary)
                    .is_some()
            );
        }
        assert!(
            dict.analysis("трёхзвездный", entry(table, 0), Source::Dictionary)
                .is_none()
        );
    }

    #[test]
    fn probabilities_of_spelling_without_yo() {
        // Частоты есть у написания «тетки», которого нет в словаре: словоформа - «тётки»
        let mut builder = builder(&["NOUN,gent", "NOUN,nomn"], &[""], &[""]);
        let genitive = lexeme_table(&mut builder, &[(0, 0, 0)]);
        let nominative = lexeme_table(&mut builder, &[(0, 1, 0)]);
        builder
            .add_word("тётки", &[entry(genitive, 0), entry(nominative, 0)])
            .unwrap();
        builder.add_probability("тетки", 1, 800_000).unwrap();
        builder.add_probability("тетки", 0, 200_000).unwrap();
        builder.add_probability("тётки", 0, 900_000).unwrap();
        builder.add_probability("тётки", 1, 100_000).unwrap();
        // Частоты слова, которого нет в словаре, в файл не попадают
        builder.add_probability("вайб", 0, 1_000_000).unwrap();
        let data = builder.build().unwrap();
        assert_eq!(format::tests::read(&data).probability_records.len(), 10);
        let dict = Dictionary::from_bytes(&data).unwrap();
        let top = |word: &str| {
            let analyses = dict.analyze(word);
            (analyses[0].word.clone(), analyses[0].tag, analyses[0].prob)
        };
        assert_eq!(top("тетки"), ("тётки".to_string(), Some(1), 0.8));
        assert_eq!(top("тётки"), ("тётки".to_string(), Some(0), 0.9));
    }

    #[test]
    fn mapped_file_outlives_dictionary_parts() {
        let path = std::env::temp_dir().join(format!("rumorph-map-{}.dict", std::process::id()));
        std::fs::write(&path, dictionary_bytes()).unwrap();
        let dict = Dictionary::open(&path).unwrap();
        assert!(matches!(*dict.storage, Storage::Mapped(_)));
        let analyses = dict.analyze("стали");
        drop(dict);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(analyses.len(), 3);
        // Пустой файл читается, а не отображается, и дает ошибку формата
        std::fs::write(&path, b"").unwrap();
        let result = Dictionary::open(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(result, Err(Error::Format(_))));
    }
}
