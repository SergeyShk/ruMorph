use std::collections::{BTreeMap, HashMap};

use fst::MapBuilder;

use crate::Error;
use crate::format::{self, Sections};

/// Сборщик словаря из таблиц тегов, префиксов, суффиксов, парадигм, словоформ и частот
///
/// Парадигма - список форм лексемы; форма задается номерами суффикса, тега и префикса,
/// форма 0 - начальная (лемма). Словоформа ссылается на пары «парадигма, форма».
pub struct Builder {
    meta: String,
    tags: Vec<String>,
    prefixes: Vec<String>,
    suffixes: Vec<String>,
    paradigms: Vec<Vec<(u16, u16, u16)>>,
    words: BTreeMap<String, Vec<u32>>,
    probabilities: BTreeMap<String, Vec<(u16, u32)>>,
}

impl Builder {
    pub fn new(tags: Vec<String>, prefixes: Vec<String>, suffixes: Vec<String>) -> Self {
        Builder {
            meta: String::new(),
            tags,
            prefixes,
            suffixes,
            paradigms: Vec::new(),
            words: BTreeMap::new(),
            probabilities: BTreeMap::new(),
        }
    }

    /// Описание источника словаря, которое хранится в файле как есть
    pub fn meta(&mut self, meta: impl Into<String>) -> &mut Self {
        self.meta = meta.into();
        self
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

    /// Добавляет словоформу с разборами `(парадигма, форма)`; повторные разборы дописываются
    pub fn add_word(&mut self, word: &str, analyses: &[(u16, u16)]) -> Result<(), Error> {
        let mut packed = Vec::with_capacity(analyses.len());
        for &(paradigm, form) in analyses {
            let forms = self.paradigms.get(paradigm as usize).ok_or_else(|| {
                Error::Build(format!("словоформа {word:?}: нет парадигмы {paradigm}"))
            })?;
            let &(suffix, _, prefix) = forms.get(form as usize).ok_or_else(|| {
                Error::Build(format!(
                    "словоформа {word:?}: в парадигме {paradigm} нет формы {form}"
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
                    "словоформа {word:?} не совпадает с формой {form} парадигмы {paradigm}"
                )));
            }
            packed.push(((paradigm as u32) << 16) | form as u32);
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

        Ok(format::write(&Sections {
            meta: self.meta.clone(),
            tags: self.tags.clone(),
            prefixes: self.prefixes.clone(),
            suffixes: self.suffixes.clone(),
            paradigm_offsets,
            paradigms,
            words: words.into_inner().map_err(build_error)?,
            records,
            probabilities: probabilities.into_inner().map_err(build_error)?,
            probability_records,
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
        Builder::new(
            vec!["NOUN".into()],
            vec![String::new()],
            vec![String::new(), "а".into()],
        )
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
    fn checks_word_forms() {
        let mut builder = builder();
        builder.add_paradigm(&[(0, 0, 0), (1, 0, 0)]).unwrap();
        assert!(builder.add_word("ежа", &[(1, 0)]).is_err());
        assert!(builder.add_word("ежа", &[(0, 2)]).is_err());
        assert!(builder.add_word("еж", &[(0, 1)]).is_err());
        builder.add_word("ежа", &[(0, 1)]).unwrap();
    }

    #[test]
    fn checks_probability_tags() {
        let mut builder = builder();
        assert!(builder.add_probability("еж", 1, 1).is_err());
        builder.add_probability("еж", 0, 1).unwrap();
    }
}
