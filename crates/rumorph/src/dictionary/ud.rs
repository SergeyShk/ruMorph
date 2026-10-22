//! Теги UD словаря: строки хранятся один раз, теги - номерами строк

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::Error;
use crate::format::Strings;

/// Часть речи, признаки и пометы Universal Dependencies
#[derive(Clone, Copy)]
pub struct UdTag<'d> {
    strings: &'d [Box<str>],
    /// Часть речи, число признаков, пары «имя, значение», число помет, пометы - номерами строк
    data: &'d [u16],
}

impl<'d> UdTag<'d> {
    fn string(self, number: u16) -> &'d str {
        &self.strings[usize::from(number)]
    }

    fn feat_numbers(self) -> &'d [u16] {
        &self.data[2..2 + 2 * usize::from(self.data[1])]
    }

    fn label_numbers(self) -> &'d [u16] {
        let start = 3 + 2 * usize::from(self.data[1]);
        &self.data[start..]
    }

    /// Часть речи
    pub fn pos(self) -> &'d str {
        self.string(self.data[0])
    }

    /// Признаки по алфавиту без учета регистра, как в CoNLL-U: («Case», «Gen»)
    pub fn feats(self) -> impl ExactSizeIterator<Item = (&'d str, &'d str)> {
        self.feat_numbers()
            .as_chunks::<2>()
            .0
            .iter()
            .map(move |&[name, value]| (self.string(name), self.string(value)))
    }

    /// Значение признака `name`
    pub fn feat(self, name: &str) -> Option<&'d str> {
        self.feats()
            .find(|&(known, _)| known == name)
            .map(|(_, value)| value)
    }

    /// Пометы словаря, у которых нет признака UD
    pub fn labels(self) -> impl ExactSizeIterator<Item = &'d str> {
        self.label_numbers()
            .iter()
            .map(move |&number| self.string(number))
    }

    /// Есть ли помета `label`
    pub fn has_label(self, label: &str) -> bool {
        self.labels().any(|known| known == label)
    }

    /// Признаки в записи CoNLL-U: «Case=Gen|Number=Sing»
    pub fn feats_string(self) -> String {
        self.feats()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("|")
    }
}

impl PartialEq for UdTag<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.pos() == other.pos()
            && self.feats().eq(other.feats())
            && self.labels().eq(other.labels())
    }
}

impl Eq for UdTag<'_> {}

impl Hash for UdTag<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pos().hash(state);
        self.feats().len().hash(state);
        self.feats().for_each(|feat| feat.hash(state));
        self.labels().for_each(|label| label.hash(state));
    }
}

impl fmt::Debug for UdTag<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UdTag")
            .field("pos", &self.pos())
            .field("feats", &self.feats().collect::<Vec<_>>())
            .field("labels", &self.labels().collect::<Vec<_>>())
            .finish()
    }
}

/// Теги UD словаря
#[derive(Clone)]
pub(crate) struct UdTags {
    /// Разные строки тегов: одинаковые строки файла получают один номер
    strings: Vec<Box<str>>,
    /// Начало каждого тега в `data` и общий конец
    bounds: Vec<u32>,
    data: Vec<u16>,
}

impl UdTags {
    /// Теги из таблицы строк и тегов номерами строк подряд
    pub fn read(table: &Strings, data: &[u16]) -> Result<Self, Error> {
        let invalid = |msg: &str| Err(Error::Format(msg.into()));
        let mut tags = UdTags {
            strings: Vec::new(),
            bounds: vec![0],
            data: Vec::with_capacity(data.len()),
        };
        let mut numbers: HashMap<&str, u16> = HashMap::new();
        let mut renumbered = Vec::with_capacity(table.len());
        for text in table.iter() {
            let next = numbers.len() as u16;
            let number = *numbers.entry(text).or_insert(next);
            if number == next {
                tags.strings.push(text.into());
            }
            renumbered.push(number);
        }
        let mut at = 0;
        while at < data.len() {
            let Some(&count) = data.get(at + 1) else {
                return invalid("тег UD обрезан");
            };
            let labels = at + 2 + 2 * usize::from(count);
            let Some(&label_count) = data.get(labels) else {
                return invalid("тег UD обрезан");
            };
            let end = labels + 1 + usize::from(label_count);
            let Some(tag) = data.get(at..end) else {
                return invalid("тег UD обрезан");
            };
            for (index, &number) in tag.iter().enumerate() {
                let is_count = index == 1 || index == labels - at;
                tags.data.push(if is_count {
                    number
                } else {
                    match renumbered.get(usize::from(number)) {
                        Some(&number) => number,
                        None => return invalid("тег UD ссылается за пределы таблицы строк"),
                    }
                });
            }
            if tags.strings[usize::from(tags.data[at])].is_empty() {
                return invalid("тег UD без части речи");
            }
            tags.bounds.push(end as u32);
            at = end;
        }
        if tags.len() > usize::from(u16::MAX) + 1 {
            return invalid("больше 65 536 тегов UD");
        }
        Ok(tags)
    }

    pub fn len(&self) -> usize {
        self.bounds.len() - 1
    }

    pub fn get(&self, number: u16) -> Option<UdTag<'_>> {
        let number = usize::from(number);
        (number < self.len()).then(|| self.tag(number))
    }

    fn tag(&self, number: usize) -> UdTag<'_> {
        let (start, end) = (self.bounds[number], self.bounds[number + 1]);
        UdTag {
            strings: &self.strings,
            data: &self.data[start as usize..end as usize],
        }
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = UdTag<'_>> {
        (0..self.len()).map(|number| self.tag(number))
    }

    /// Строка тегов с номером `number`
    pub fn string(&self, number: u16) -> &str {
        self.strings
            .get(usize::from(number))
            .map_or("", |text| text)
    }

    /// Номер строки тегов
    pub fn string_number(&self, text: &str) -> Option<u16> {
        self.strings
            .iter()
            .position(|known| &**known == text)
            .map(|number| number as u16)
    }

    /// Номер тега с частью речи `pos`, признаками `feats` и без помет; если такого тега нет,
    /// он добавляется
    pub fn find_or_add(&mut self, pos: &str, feats: &[(&str, &str)]) -> u16 {
        let found = self.iter().position(|tag| {
            tag.pos() == pos && tag.feats().eq(feats.iter().copied()) && tag.labels().len() == 0
        });
        if let Some(number) = found {
            return number as u16;
        }
        let mut number = |text: &str| match self.string_number(text) {
            Some(number) => number,
            None => {
                self.strings.push(text.into());
                (self.strings.len() - 1) as u16
            }
        };
        let mut tag = vec![number(pos), feats.len() as u16];
        for &(name, value) in feats {
            tag.extend([number(name), number(value)]);
        }
        tag.push(0);
        self.data.extend(tag);
        self.bounds.push(self.data.len() as u32);
        (self.len() - 1) as u16
    }

    /// Номера строк части речи, пар «имя, значение» признаков и помет тега
    pub fn numbers(&self, number: u16) -> Option<(u16, &[u16], &[u16])> {
        let tag = self.get(number)?;
        Some((tag.data[0], tag.feat_numbers(), tag.label_numbers()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(strings: &[&str]) -> Strings {
        let mut bytes = (strings.len() as u32).to_le_bytes().to_vec();
        let mut end = 0;
        for text in strings {
            end += text.len() as u32;
            bytes.extend_from_slice(&end.to_le_bytes());
        }
        for text in strings {
            bytes.extend_from_slice(text.as_bytes());
        }
        Strings::read(&bytes).unwrap()
    }

    #[test]
    fn reads_tags() {
        let strings = table(&["NOUN", "Case", "Gen", "Infr", "NOUN", "VERB"]);
        let tags = UdTags::read(&strings, &[0, 1, 1, 2, 1, 3, 5, 0, 0, 4, 0, 0]).unwrap();
        assert_eq!(tags.len(), 3);
        let noun = tags.get(0).unwrap();
        assert_eq!(noun.pos(), "NOUN");
        assert_eq!(noun.feats().collect::<Vec<_>>(), [("Case", "Gen")]);
        assert_eq!(noun.feat("Case"), Some("Gen"));
        assert_eq!(noun.feats_string(), "Case=Gen");
        assert!(noun.has_label("Infr") && !noun.has_label("Slng"));
        assert_eq!(tags.get(1).unwrap().pos(), "VERB");
        assert_eq!(tags.get(1).unwrap().feats().len(), 0);
        // Одинаковые строки файла - одна строка
        assert_eq!(tags.numbers(2).unwrap().0, tags.numbers(0).unwrap().0);
        let unique: std::collections::HashSet<UdTag> = tags.iter().chain(tags.iter()).collect();
        assert_eq!(unique.len(), 3);
        assert!(tags.get(3).is_none());
        assert_eq!(
            format!("{:?}", tags.get(1).unwrap()),
            r#"UdTag { pos: "VERB", feats: [], labels: [] }"#
        );
    }

    #[test]
    fn adds_missing_tags() {
        let strings = table(&["NOUN", "Case", "Gen"]);
        let mut tags = UdTags::read(&strings, &[0, 1, 1, 2, 0]).unwrap();
        assert_eq!(tags.find_or_add("NOUN", &[("Case", "Gen")]), 0);
        assert_eq!(tags.find_or_add("NUM", &[("Case", "Gen")]), 1);
        assert_eq!(tags.find_or_add("NUM", &[("Case", "Gen")]), 1);
        assert_eq!(tags.get(1).unwrap().feats_string(), "Case=Gen");
        assert_eq!(tags.string_number("NUM"), Some(3));
    }

    #[test]
    fn rejects_broken_tags() {
        let strings = table(&["NOUN", "", "Gen"]);
        for data in [
            &[0][..],
            &[0, 1, 1],
            &[0, 1, 1, 2],
            &[0, 0, 1],
            &[0, 0, 1, 7],
            &[3, 0, 0],
            &[1, 0, 0],
        ] {
            assert!(UdTags::read(&strings, data).is_err(), "{data:?}");
        }
        assert!(UdTags::read(&strings, &[0, 0, 1, 2]).is_ok());
    }
}
