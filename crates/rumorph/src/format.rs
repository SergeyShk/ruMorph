//! Бинарный формат словаря: сигнатура, версия формата и секции «длина u64 + данные», числа little-endian

use std::io::{ErrorKind, Read};

use crate::Error;

const MAGIC: &[u8; 8] = b"RUMORPH\0";
const VERSION: u32 = 4;

/// Содержимое словаря в том виде, в каком оно лежит в файле
pub(crate) struct Sections {
    pub meta: String,
    /// Теги OpenCorpora форм парадигм
    pub tags: Vec<String>,
    /// Теги UD разборов: «часть речи\tпризнаки\tпометы через запятую»
    pub ud_tags: Vec<String>,
    pub prefixes: Vec<String>,
    pub suffixes: Vec<String>,
    /// Начало каждой парадигмы в `paradigms` и общий конец
    pub paradigm_offsets: Vec<u32>,
    /// Парадигмы подряд: номера суффиксов, тегов и префиксов форм тремя блоками
    pub paradigms: Vec<u16>,
    /// Таблицы словоизменения подряд: номер парадигмы, затем `(форма леммы << 16) | тег UD`
    /// каждой ее формы
    pub tables: Vec<u32>,
    /// Автомат «словоформа - начало списка разборов в `records`»
    pub words: Vec<u8>,
    /// Списки разборов: длина, затем `(таблица << 16) | форма` каждого разбора
    pub records: Vec<u32>,
    /// Автомат «словоформа - начало списка частот в `probability_records`»
    pub probabilities: Vec<u8>,
    /// Списки частот: длина, затем пары «тег, P(тег | словоформа) в миллионных долях»
    pub probability_records: Vec<u32>,
    /// Автомат «индекс и окончание - начало списка в `guess_records`» для угадывания
    pub guesses: Vec<u8>,
    /// Списки форм для угадывания: длина, затем пары `(таблица << 16) | форма`, число словоформ
    pub guess_records: Vec<u32>,
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
    section(&strings_bytes(&sections.ud_tags));
    section(&strings_bytes(&sections.prefixes));
    section(&strings_bytes(&sections.suffixes));
    section(&u32_bytes(&sections.paradigm_offsets));
    section(&u16_bytes(&sections.paradigms));
    section(&u32_bytes(&sections.tables));
    section(&sections.words);
    section(&u32_bytes(&sections.records));
    section(&sections.probabilities);
    section(&u32_bytes(&sections.probability_records));
    section(&sections.guesses);
    section(&u32_bytes(&sections.guess_records));
    out
}

/// Словарь из байтов в памяти
pub(crate) fn read(data: &[u8]) -> Result<Sections, Error> {
    read_from(data, data.len() as u64)
}

/// Словарь из потока длиной `size` байт: секции читаются по одной, без копии всего файла
/// в памяти; длина секции больше остатка потока - ошибка формата, а не огромная аллокация
pub(crate) fn read_from(input: impl Read, size: u64) -> Result<Sections, Error> {
    let mut stream = Stream { input, left: size };
    let signature = match stream.take(MAGIC.len() as u64) {
        Err(Error::Io(err)) => return Err(Error::Io(err)),
        signature => signature.ok(),
    };
    if signature.as_deref() != Some(MAGIC.as_slice()) {
        return Err(Error::Format("нет сигнатуры словаря ruMorph".into()));
    }
    let version = u32::from_le_bytes(stream.take(4)?.try_into().unwrap());
    if version != VERSION {
        return Err(Error::Format(format!(
            "версия формата {version}, поддерживается {VERSION}"
        )));
    }
    let sections = Sections {
        meta: utf8(&stream.section()?)?,
        tags: strings(&stream.section()?)?,
        ud_tags: strings(&stream.section()?)?,
        prefixes: strings(&stream.section()?)?,
        suffixes: strings(&stream.section()?)?,
        paradigm_offsets: stream.numbers::<4, _>(u32::from_le_bytes)?,
        paradigms: stream.numbers::<2, _>(u16::from_le_bytes)?,
        tables: stream.numbers::<4, _>(u32::from_le_bytes)?,
        words: stream.section()?,
        records: stream.numbers::<4, _>(u32::from_le_bytes)?,
        probabilities: stream.section()?,
        probability_records: stream.numbers::<4, _>(u32::from_le_bytes)?,
        guesses: stream.section()?,
        guess_records: stream.numbers::<4, _>(u32::from_le_bytes)?,
    };
    if stream.left != 0 {
        return Err(Error::Format("лишние данные после последней секции".into()));
    }
    Ok(sections)
}

/// Поток с известным остатком длины
struct Stream<R> {
    input: R,
    left: u64,
}

impl<R: Read> Stream<R> {
    fn take(&mut self, len: u64) -> Result<Vec<u8>, Error> {
        let truncated = || Error::Format("файл обрезан".into());
        if len > self.left {
            return Err(truncated());
        }
        let mut bytes = vec![0; usize::try_from(len).map_err(|_| truncated())?];
        self.input
            .read_exact(&mut bytes)
            .map_err(|err| match err.kind() {
                ErrorKind::UnexpectedEof => truncated(),
                _ => Error::Io(err),
            })?;
        self.left -= len;
        Ok(bytes)
    }

    fn section(&mut self) -> Result<Vec<u8>, Error> {
        let len = u64::from_le_bytes(self.take(8)?.try_into().unwrap());
        self.take(len)
    }

    /// Секция чисел по `N` байт: читается кусками сразу в массив, без байтовой копии секции
    fn numbers<const N: usize, T>(&mut self, decode: fn([u8; N]) -> T) -> Result<Vec<T>, Error> {
        const CHUNK: usize = 1 << 16;
        let len = u64::from_le_bytes(self.take(8)?.try_into().unwrap());
        if len % N as u64 != 0 {
            return Err(Error::Format(format!("длина секции не кратна {N}")));
        }
        if len > self.left {
            return Err(Error::Format("файл обрезан".into()));
        }
        let mut values = Vec::with_capacity(len as usize / N);
        let mut left = len;
        while left > 0 {
            let chunk = self.take(left.min(CHUNK as u64))?;
            let (items, _) = chunk.as_chunks::<N>();
            values.extend(items.iter().map(|&item| decode(item)));
            left -= chunk.len() as u64;
        }
        Ok(values)
    }
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
    let count = u32::from_le_bytes(reader.take(4)?.try_into().unwrap()) as usize;
    // Каждая строка занимает хотя бы 4 байта длины: иначе число строк из файла задало бы любую аллокацию
    if count > (bytes.len() - 4) / 4 {
        return Err(Error::Format("число строк больше размера таблицы".into()));
    }
    let mut values = Vec::with_capacity(count);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sections {
        Sections {
            meta: "{}".into(),
            tags: vec!["NOUN".into(), "VERB".into()],
            ud_tags: vec!["NOUN\tCase=Nom\t".into()],
            prefixes: vec![String::new(), "по".into()],
            suffixes: vec![String::new(), "а".into()],
            paradigm_offsets: vec![0, 3],
            paradigms: vec![0, 0, 0],
            tables: vec![0, 0],
            words: vec![1, 2, 3],
            records: vec![1, 0],
            probabilities: vec![],
            probability_records: vec![],
            guesses: vec![],
            guess_records: vec![],
        }
    }

    #[test]
    fn roundtrip() {
        let read_back = read(&write(&sample())).unwrap();
        assert_eq!(read_back.tags, sample().tags);
        assert_eq!(read_back.ud_tags, sample().ud_tags);
        assert_eq!(read_back.prefixes, sample().prefixes);
        assert_eq!(read_back.paradigms, sample().paradigms);
        assert_eq!(read_back.tables, sample().tables);
        assert_eq!(read_back.words, sample().words);
        assert_eq!(read_back.records, sample().records);
    }

    #[test]
    fn reads_stream_by_sections() {
        let data = write(&sample());
        let read_back = read_from(data.as_slice(), data.len() as u64).unwrap();
        assert_eq!(read_back.records, sample().records);
        // Длина секции больше остатка - ошибка формата, а не аллокация на эту длину
        let mut huge = data[..MAGIC.len() + 4].to_vec();
        huge.extend_from_slice(&u64::MAX.to_le_bytes());
        let err = read_from(huge.as_slice(), huge.len() as u64).err().unwrap();
        assert!(err.to_string().contains("файл обрезан"));
    }

    #[test]
    fn rejects_numbers_of_wrong_length() {
        let mut sections = sample();
        sections.paradigms = vec![0, 0, 0];
        let mut data = write(&sections);
        // Секция парадигм идет седьмой: укорачиваем ее длину на байт и убираем этот байт
        let mut at = MAGIC.len() + 4;
        for _ in 0..6 {
            let len = u64::from_le_bytes(data[at..at + 8].try_into().unwrap()) as usize;
            at += 8 + len;
        }
        let len = u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
        data[at..at + 8].copy_from_slice(&(len - 1).to_le_bytes());
        data.remove(at + 8);
        let err = read(&data).err().unwrap();
        assert!(err.to_string().contains("не кратна 2"));
    }

    #[test]
    fn passes_io_errors() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("диск"))
            }
        }
        assert!(matches!(read_from(Broken, 100), Err(Error::Io(_))));
    }

    #[test]
    fn rejects_foreign_data() {
        assert!(matches!(read(b"not a dictionary"), Err(Error::Format(_))));
        assert!(matches!(read(b""), Err(Error::Format(_))));
    }

    #[test]
    fn rejects_other_version() {
        let mut data = write(&sample());
        data[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&3u32.to_le_bytes());
        let err = read(&data).err().unwrap();
        assert!(err.to_string().contains("версия формата 3"));
    }

    #[test]
    fn rejects_string_count_beyond_table() {
        let mut table = u32::MAX.to_le_bytes().to_vec();
        table.extend_from_slice(&[0; 8]);
        assert!(matches!(strings(&table), Err(Error::Format(_))));
        assert_eq!(strings(&strings_bytes(&["а".into()])).unwrap(), ["а"]);
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
