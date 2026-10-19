//! Бинарный формат словаря: сигнатура, версия формата и секции «длина u64 + данные», числа little-endian

use crate::Error;

const MAGIC: &[u8; 8] = b"RUMORPH\0";
const VERSION: u32 = 1;

/// Содержимое словаря в том виде, в каком оно лежит в файле
pub(crate) struct Sections {
    pub meta: String,
    pub tags: Vec<String>,
    pub prefixes: Vec<String>,
    pub suffixes: Vec<String>,
    /// Начало каждой парадигмы в `paradigms` и общий конец
    pub paradigm_offsets: Vec<u32>,
    /// Парадигмы подряд: номера суффиксов, тегов и префиксов форм тремя блоками
    pub paradigms: Vec<u16>,
    /// Автомат «словоформа - начало списка разборов в `records`»
    pub words: Vec<u8>,
    /// Списки разборов: длина, затем `(парадигма << 16) | форма`
    pub records: Vec<u32>,
    /// Автомат «словоформа - начало списка частот в `probability_records`»
    pub probabilities: Vec<u8>,
    /// Списки частот: длина, затем пары «тег, P(тег | словоформа) в миллионных долях»
    pub probability_records: Vec<u32>,
}

pub(crate) fn write(sections: &Sections) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&VERSION.to_le_bytes());
    let mut section = |bytes: &[u8]| {
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(bytes);
    };
    section(sections.meta.as_bytes());
    section(&strings_bytes(&sections.tags));
    section(&strings_bytes(&sections.prefixes));
    section(&strings_bytes(&sections.suffixes));
    section(&u32_bytes(&sections.paradigm_offsets));
    section(&u16_bytes(&sections.paradigms));
    section(&sections.words);
    section(&u32_bytes(&sections.records));
    section(&sections.probabilities);
    section(&u32_bytes(&sections.probability_records));
    out
}

pub(crate) fn read(data: &[u8]) -> Result<Sections, Error> {
    if data.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
        return Err(Error::Format("нет сигнатуры словаря ruMorph".into()));
    }
    let mut reader = Reader {
        data,
        pos: MAGIC.len(),
    };
    let version = u32::from_le_bytes(reader.take(4)?.try_into().unwrap());
    if version != VERSION {
        return Err(Error::Format(format!(
            "версия формата {version}, поддерживается {VERSION}"
        )));
    }
    let sections = Sections {
        meta: utf8(reader.section()?)?,
        tags: strings(reader.section()?)?,
        prefixes: strings(reader.section()?)?,
        suffixes: strings(reader.section()?)?,
        paradigm_offsets: u32_vec(reader.section()?)?,
        paradigms: u16_vec(reader.section()?)?,
        words: reader.section()?.to_vec(),
        records: u32_vec(reader.section()?)?,
        probabilities: reader.section()?.to_vec(),
        probability_records: u32_vec(reader.section()?)?,
    };
    if reader.pos != data.len() {
        return Err(Error::Format("лишние данные после последней секции".into()));
    }
    Ok(sections)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|&end| end <= self.data.len());
        let end = end.ok_or_else(|| Error::Format("файл обрезан".into()))?;
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn section(&mut self) -> Result<&'a [u8], Error> {
        let len = u64::from_le_bytes(self.take(8)?.try_into().unwrap());
        let len = usize::try_from(len).map_err(|_| Error::Format("файл обрезан".into()))?;
        self.take(len)
    }
}

fn utf8(bytes: &[u8]) -> Result<String, Error> {
    String::from_utf8(bytes.to_vec()).map_err(|_| Error::Format("строка не в UTF-8".into()))
}

fn strings_bytes(values: &[String]) -> Vec<u8> {
    let mut out = (values.len() as u32).to_le_bytes().to_vec();
    for value in values {
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    out
}

fn strings(bytes: &[u8]) -> Result<Vec<String>, Error> {
    let mut reader = Reader {
        data: bytes,
        pos: 0,
    };
    let count = u32::from_le_bytes(reader.take(4)?.try_into().unwrap());
    let mut values = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let len = u32::from_le_bytes(reader.take(4)?.try_into().unwrap());
        values.push(utf8(reader.take(len as usize)?)?);
    }
    if reader.pos != bytes.len() {
        return Err(Error::Format("лишние данные в таблице строк".into()));
    }
    Ok(values)
}

fn u16_bytes(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn u32_bytes(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn u16_vec(bytes: &[u8]) -> Result<Vec<u16>, Error> {
    let (chunks, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        return Err(Error::Format("длина секции не кратна 2".into()));
    }
    Ok(chunks
        .iter()
        .map(|chunk| u16::from_le_bytes(*chunk))
        .collect())
}

fn u32_vec(bytes: &[u8]) -> Result<Vec<u32>, Error> {
    let (chunks, rest) = bytes.as_chunks::<4>();
    if !rest.is_empty() {
        return Err(Error::Format("длина секции не кратна 4".into()));
    }
    Ok(chunks
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sections {
        Sections {
            meta: "{}".into(),
            tags: vec!["NOUN".into(), "VERB".into()],
            prefixes: vec![String::new(), "по".into()],
            suffixes: vec![String::new(), "а".into()],
            paradigm_offsets: vec![0, 3],
            paradigms: vec![0, 0, 0],
            words: vec![1, 2, 3],
            records: vec![1, 0],
            probabilities: vec![],
            probability_records: vec![],
        }
    }

    #[test]
    fn roundtrip() {
        let read_back = read(&write(&sample())).unwrap();
        assert_eq!(read_back.tags, sample().tags);
        assert_eq!(read_back.prefixes, sample().prefixes);
        assert_eq!(read_back.paradigms, sample().paradigms);
        assert_eq!(read_back.words, sample().words);
        assert_eq!(read_back.records, sample().records);
    }

    #[test]
    fn rejects_foreign_data() {
        assert!(matches!(read(b"not a dictionary"), Err(Error::Format(_))));
        assert!(matches!(read(b""), Err(Error::Format(_))));
    }

    #[test]
    fn rejects_other_version() {
        let mut data = write(&sample());
        data[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&2u32.to_le_bytes());
        let err = read(&data).err().unwrap();
        assert!(err.to_string().contains("версия формата 2"));
    }

    #[test]
    fn rejects_truncated_and_trailing_data() {
        let data = write(&sample());
        for len in [MAGIC.len() + 4, MAGIC.len() + 10, data.len() - 1] {
            assert!(matches!(read(&data[..len]), Err(Error::Format(_))), "{len}");
        }
        let mut longer = data.clone();
        longer.push(0);
        assert!(matches!(read(&longer), Err(Error::Format(_))));
    }
}
