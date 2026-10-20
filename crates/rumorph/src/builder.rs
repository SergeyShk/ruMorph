use std::collections::{BTreeMap, HashMap};

use fst::MapBuilder;

use crate::Error;
use crate::format::{self, Sections};

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

    /// Ключ окончания в автомате угадывания
    pub(crate) fn key(self, ending: &str) -> String {
        let index = match self {
            GuessIndex::Common => '0',
            GuessIndex::Proper => '1',
        };
        format!("{index}{ending}")
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

/// Сборщик словаря из таблиц тегов, префиксов, суффиксов, парадигм, словоформ и частот
///
/// Парадигма - список форм лексемы; форма задается номерами суффикса, тега OpenCorpora
/// и префикса, форма 0 - начальная. Таблица словоизменения дополняет парадигму для
/// лексем одного типа: у каждой формы - форма леммы и тег UD. Разбор словоформы
/// ссылается на таблицу и форму.
pub struct Builder {
    meta: String,
    tags: Vec<String>,
    ud_tags: Vec<String>,
    ud_numbers: HashMap<String, u16>,
    prefixes: Vec<String>,
    suffixes: Vec<String>,
    suffix_numbers: HashMap<String, u16>,
    paradigms: Vec<Vec<(u16, u16, u16)>>,
    tables: Vec<(u16, Vec<u32>)>,
    table_numbers: HashMap<(u16, Vec<u32>), u16>,
    words: BTreeMap<String, Vec<u32>>,
    probabilities: BTreeMap<String, Vec<(u16, u32)>>,
    guesses: BTreeMap<String, Vec<(u32, u32)>>,
}

impl Builder {
    pub fn new(tags: Vec<String>, prefixes: Vec<String>, suffixes: Vec<String>) -> Self {
        let mut suffix_numbers = HashMap::new();
        for (number, suffix) in suffixes.iter().enumerate() {
            suffix_numbers
                .entry(suffix.clone())
                .or_insert(number as u16);
        }
        Builder {
            suffix_numbers,
            meta: String::new(),
            tags,
            ud_tags: Vec::new(),
            ud_numbers: HashMap::new(),
            prefixes,
            suffixes,
            paradigms: Vec::new(),
            tables: Vec::new(),
            table_numbers: HashMap::new(),
            words: BTreeMap::new(),
            probabilities: BTreeMap::new(),
            guesses: BTreeMap::new(),
        }
    }

    /// Описание источника словаря, которое хранится в файле как есть
    pub fn meta(&mut self, meta: impl Into<String>) -> &mut Self {
        self.meta = meta.into();
        self
    }

    /// Добавляет тег UD и возвращает его номер; одинаковые теги получают один номер
    ///
    /// `feats` - признаки в записи CoNLL-U («Case=Gen|Number=Sing», пустая строка - без признаков)
    pub fn add_ud_tag(&mut self, pos: &str, feats: &str, labels: &[&str]) -> Result<u16, Error> {
        let fields = [pos, feats, &labels.join(",")];
        if pos.is_empty() || fields.iter().any(|field| field.contains(['\t', '\n'])) {
            return Err(Error::Build(format!("неверный тег UD {fields:?}")));
        }
        if !feats.is_empty() && feats.split('|').any(|feat| feat.split_once('=').is_none()) {
            return Err(Error::Build(format!("неверные признаки UD {feats:?}")));
        }
        let tag = fields.join("\t");
        if let Some(&number) = self.ud_numbers.get(&tag) {
            return Ok(number);
        }
        let number = u16::try_from(self.ud_tags.len())
            .map_err(|_| Error::Build("больше 65 536 тегов UD".into()))?;
        self.ud_tags.push(tag.clone());
        self.ud_numbers.insert(tag, number);
        Ok(number)
    }

    /// Добавляет суффикс и возвращает его номер; известный суффикс сохраняет свой номер
    pub fn add_suffix(&mut self, suffix: &str) -> Result<u16, Error> {
        if let Some(&number) = self.suffix_numbers.get(suffix) {
            return Ok(number);
        }
        let number = u16::try_from(self.suffixes.len())
            .map_err(|_| Error::Build("больше 65 536 суффиксов".into()))?;
        self.suffixes.push(suffix.to_owned());
        self.suffix_numbers.insert(suffix.to_owned(), number);
        Ok(number)
    }

    /// Добавляет парадигму из форм `(суффикс, тег, префикс)` и возвращает ее номер
    pub fn add_paradigm(&mut self, forms: &[(u16, u16, u16)]) -> Result<u16, Error> {
        if forms.is_empty() {
            return Err(Error::Build("парадигма без форм".into()));
        }
        for &(suffix, tag, prefix) in forms {
            check_id("суффикс", suffix, self.suffixes.len())?;
            check_id("тег", tag, self.tags.len())?;
            check_id("префикс", prefix, self.prefixes.len())?;
        }
        let id = u16::try_from(self.paradigms.len())
            .map_err(|_| Error::Build("больше 65 536 парадигм".into()))?;
        self.paradigms.push(forms.to_vec());
        Ok(id)
    }

    /// Добавляет таблицу словоизменения - пары «форма леммы, тег UD» всех форм парадигмы -
    /// и возвращает ее номер; одинаковые таблицы получают один номер
    pub fn add_table(&mut self, paradigm: u16, forms: &[(u16, u16)]) -> Result<u16, Error> {
        let count = self
            .paradigms
            .get(paradigm as usize)
            .ok_or_else(|| Error::Build(format!("нет парадигмы {paradigm}")))?
            .len();
        if forms.len() != count {
            return Err(Error::Build(format!(
                "в таблице {} форм, в парадигме {paradigm} - {count}",
                forms.len()
            )));
        }
        for &(lemma_form, ud) in forms {
            check_id("форма леммы", lemma_form, count)?;
            check_id("тег UD", ud, self.ud_tags.len())?;
        }
        let cells: Vec<u32> = forms
            .iter()
            .map(|&(lemma_form, ud)| (u32::from(lemma_form) << 16) | u32::from(ud))
            .collect();
        let key = (paradigm, cells);
        if let Some(&number) = self.table_numbers.get(&key) {
            return Ok(number);
        }
        let number = u16::try_from(self.tables.len())
            .map_err(|_| Error::Build("больше 65 536 таблиц словоизменения".into()))?;
        self.tables.push(key.clone());
        self.table_numbers.insert(key, number);
        Ok(number)
    }

    /// Добавляет словоформу с разборами; повторные разборы дописываются
    pub fn add_word(&mut self, word: &str, entries: &[Entry]) -> Result<(), Error> {
        let mut packed = Vec::with_capacity(entries.len());
        for entry in entries {
            let &(paradigm, _) = self.tables.get(entry.table as usize).ok_or_else(|| {
                Error::Build(format!("словоформа {word:?}: нет таблицы {}", entry.table))
            })?;
            let forms = &self.paradigms[paradigm as usize];
            let &(suffix, _, prefix) = forms.get(entry.form as usize).ok_or_else(|| {
                Error::Build(format!(
                    "словоформа {word:?}: в парадигме {paradigm} нет формы {}",
                    entry.form
                ))
            })?;
            let (prefix, suffix) = (
                &self.prefixes[prefix as usize],
                &self.suffixes[suffix as usize],
            );
            if word.len() < prefix.len() + suffix.len()
                || !word.starts_with(prefix.as_str())
                || !word.ends_with(suffix.as_str())
            {
                return Err(Error::Build(format!(
                    "словоформа {word:?} не совпадает с формой {} парадигмы {paradigm}",
                    entry.form
                )));
            }
            packed.push((u32::from(entry.table) << 16) | u32::from(entry.form));
        }
        self.words
            .entry(word.to_owned())
            .or_default()
            .extend(packed);
        Ok(())
    }

    /// Добавляет P(тег | словоформа) в миллионных долях
    pub fn add_probability(&mut self, word: &str, tag: u16, millionths: u32) -> Result<(), Error> {
        check_id("тег", tag, self.tags.len())?;
        let pairs = self.probabilities.entry(word.to_owned()).or_default();
        match pairs.iter_mut().find(|(known, _)| *known == tag) {
            Some(pair) => pair.1 = millionths,
            None => pairs.push((tag, millionths)),
        }
        Ok(())
    }

    /// Добавляет формы для угадывания незнакомых слов с окончанием `ending`: форма таблицы
    /// и число словоформ словаря с этим окончанием и этой формой
    pub fn add_guesses(
        &mut self,
        index: GuessIndex,
        ending: &str,
        forms: &[(Entry, u32)],
    ) -> Result<(), Error> {
        let mut packed = Vec::with_capacity(forms.len());
        for &(entry, count) in forms {
            let &(paradigm, _) = self.tables.get(entry.table as usize).ok_or_else(|| {
                Error::Build(format!("окончание {ending:?}: нет таблицы {}", entry.table))
            })?;
            check_id("форма", entry.form, self.paradigms[paradigm as usize].len())?;
            packed.push((
                (u32::from(entry.table) << 16) | u32::from(entry.form),
                count,
            ));
        }
        self.guesses.insert(index.key(ending), packed);
        Ok(())
    }

    /// Собирает словарь в бинарном формате
    pub fn build(&self) -> Result<Vec<u8>, Error> {
        let mut paradigm_offsets = vec![0u32];
        let mut paradigms = Vec::new();
        for forms in &self.paradigms {
            paradigms.extend(forms.iter().map(|form| form.0));
            paradigms.extend(forms.iter().map(|form| form.1));
            paradigms.extend(forms.iter().map(|form| form.2));
            paradigm_offsets.push(paradigms.len() as u32);
        }
        let tables = self
            .tables
            .iter()
            .flat_map(|(paradigm, cells)| {
                std::iter::once(u32::from(*paradigm)).chain(cells.iter().copied())
            })
            .collect();

        // Одинаковые списки разборов у разных словоформ хранятся один раз
        let mut records = Vec::new();
        let mut offsets: HashMap<&[u32], u64> = HashMap::new();
        let mut words = MapBuilder::memory();
        for (word, packed) in &self.words {
            let offset = *offsets.entry(packed).or_insert_with(|| {
                let offset = records.len() as u64;
                records.push(packed.len() as u32);
                records.extend_from_slice(packed);
                offset
            });
            words.insert(word, offset).map_err(build_error)?;
        }

        let mut probability_records = Vec::new();
        let mut probabilities = MapBuilder::memory();
        for (word, pairs) in &self.probabilities {
            probabilities
                .insert(word, probability_records.len() as u64)
                .map_err(build_error)?;
            probability_records.push(pairs.len() as u32);
            for &(tag, millionths) in pairs {
                probability_records.extend([tag as u32, millionths]);
            }
        }

        let mut guess_records = Vec::new();
        let mut guess_offsets: HashMap<&[(u32, u32)], u64> = HashMap::new();
        let mut guesses = MapBuilder::memory();
        for (key, forms) in &self.guesses {
            let offset = *guess_offsets.entry(forms).or_insert_with(|| {
                let offset = guess_records.len() as u64;
                guess_records.push(forms.len() as u32);
                for &(entry, count) in forms {
                    guess_records.extend([entry, count]);
                }
                offset
            });
            guesses.insert(key, offset).map_err(build_error)?;
        }

        Ok(format::write(&Sections {
            meta: self.meta.clone(),
            tags: self.tags.clone(),
            ud_tags: self.ud_tags.clone(),
            prefixes: self.prefixes.clone(),
            suffixes: self.suffixes.clone(),
            paradigm_offsets,
            paradigms,
            tables,
            words: words.into_inner().map_err(build_error)?,
            records,
            probabilities: probabilities.into_inner().map_err(build_error)?,
            probability_records,
            guesses: guesses.into_inner().map_err(build_error)?,
            guess_records,
        }))
    }
}

fn check_id(what: &str, id: u16, len: usize) -> Result<(), Error> {
    if (id as usize) < len {
        Ok(())
    } else {
        Err(Error::Build(format!("{what} {id} вне таблицы из {len}")))
    }
}

fn build_error(err: fst::Error) -> Error {
    Error::Build(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builder() -> Builder {
        let mut builder = Builder::new(
            vec!["NOUN".into()],
            vec![String::new()],
            vec![String::new(), "а".into()],
        );
        builder.add_ud_tag("NOUN", "", &[]).unwrap();
        builder
    }

    fn entry(table: u16, form: u16) -> Entry {
        Entry { table, form }
    }

    #[test]
    fn checks_paradigm_ids() {
        let mut builder = builder();
        assert!(builder.add_paradigm(&[]).is_err());
        assert!(builder.add_paradigm(&[(2, 0, 0)]).is_err());
        assert!(builder.add_paradigm(&[(0, 1, 0)]).is_err());
        assert!(builder.add_paradigm(&[(0, 0, 1)]).is_err());
        assert_eq!(builder.add_paradigm(&[(0, 0, 0), (1, 0, 0)]).unwrap(), 0);
    }

    #[test]
    fn adds_suffixes() {
        let mut builder = builder();
        assert_eq!(builder.add_suffix("а").unwrap(), 1);
        assert_eq!(builder.add_suffix("ой").unwrap(), 2);
        assert_eq!(builder.add_suffix("ой").unwrap(), 2);
        assert_eq!(builder.add_paradigm(&[(2, 0, 0)]).unwrap(), 0);
    }

    #[test]
    fn checks_tables() {
        let mut builder = builder();
        builder.add_paradigm(&[(0, 0, 0), (1, 0, 0)]).unwrap();
        assert!(builder.add_table(1, &[(0, 0), (0, 0)]).is_err());
        assert!(builder.add_table(0, &[(0, 0)]).is_err());
        assert!(builder.add_table(0, &[(0, 0), (2, 0)]).is_err());
        assert!(builder.add_table(0, &[(0, 0), (0, 1)]).is_err());
        assert_eq!(builder.add_table(0, &[(0, 0), (0, 0)]).unwrap(), 0);
        assert_eq!(builder.add_table(0, &[(0, 0), (1, 0)]).unwrap(), 1);
        assert_eq!(builder.add_table(0, &[(0, 0), (0, 0)]).unwrap(), 0);
    }

    #[test]
    fn checks_guesses() {
        let mut builder = builder();
        builder.add_paradigm(&[(0, 0, 0), (1, 0, 0)]).unwrap();
        builder.add_table(0, &[(0, 0), (0, 0)]).unwrap();
        assert!(
            builder
                .add_guesses(GuessIndex::Common, "а", &[(entry(1, 0), 1)])
                .is_err()
        );
        assert!(
            builder
                .add_guesses(GuessIndex::Common, "а", &[(entry(0, 2), 1)])
                .is_err()
        );
        builder
            .add_guesses(GuessIndex::Proper, "а", &[(entry(0, 1), 1)])
            .unwrap();
    }

    #[test]
    fn checks_word_forms() {
        let mut builder = builder();
        builder.add_paradigm(&[(0, 0, 0), (1, 0, 0)]).unwrap();
        builder.add_table(0, &[(0, 0), (0, 0)]).unwrap();
        assert!(builder.add_word("ежа", &[entry(1, 0)]).is_err());
        assert!(builder.add_word("ежа", &[entry(0, 2)]).is_err());
        assert!(builder.add_word("еж", &[entry(0, 1)]).is_err());
        builder.add_word("ежа", &[entry(0, 1)]).unwrap();
    }

    #[test]
    fn ud_tags_are_shared() {
        let mut builder = builder();
        assert_eq!(
            builder
                .add_ud_tag("VERB", "Aspect=Perf", &["intr"])
                .unwrap(),
            1
        );
        assert_eq!(
            builder
                .add_ud_tag("VERB", "Aspect=Perf", &["intr"])
                .unwrap(),
            1
        );
        assert_eq!(builder.add_ud_tag("VERB", "Aspect=Perf", &[]).unwrap(), 2);
        assert!(builder.add_ud_tag("", "", &[]).is_err());
        assert!(builder.add_ud_tag("VERB", "Aspect", &[]).is_err());
        assert!(builder.add_ud_tag("VERB", "Aspect=Perf\t", &[]).is_err());
    }

    #[test]
    fn checks_probability_tags() {
        let mut builder = builder();
        assert!(builder.add_probability("еж", 1, 1).is_err());
        builder.add_probability("еж", 0, 1).unwrap();
    }
}
