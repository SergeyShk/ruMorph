//! Бинарный формат словаря: заголовок (сигнатура, версия формата, CRC32 остальной части файла)
//! и секции «длина u64 + данные + нули до границы 8 байт», числа little-endian. Секции чисел
//! выровнены, поэтому читаются прямо из отображенного в память файла, без копий

use std::io::{ErrorKind, Read};
use std::ops::Range;

use crate::Error;

const MAGIC: &[u8; 8] = b"RUMORPH\0";
/// Наибольшее число написаний одного ключа словоформы: в словаре OpenCorpora их не больше трех
pub(crate) const MAX_SPELLINGS: usize = 8;
/// Наибольшая длина списка разборов, частот, написаний или форм для угадывания: в словаре они
/// не длиннее нескольких десятков, а испорченная длина делает разбор слова квадратичным
pub(crate) const MAX_LIST: usize = 255;
/// Бит длины списка частот: ключи списка - формы таблиц `(таблица << 16) | форма`, а не теги
pub(crate) const ENTRY_KEYS: u32 = 1 << 31;
const VERSION: u32 = 6;
const HEADER: usize = 16;
const ALIGN: usize = 8;
const SECTIONS: usize = 16;

/// Содержимое словаря для записи в файл
#[cfg(any(test, feature = "build"))]
#[derive(Clone, Default)]
pub(crate) struct Sections {
    pub meta: String,
    /// Теги OpenCorpora форм парадигм
    pub tags: Vec<String>,
    /// Части речи, имена и значения признаков и пометы тегов UD
    pub ud_strings: Vec<String>,
    /// Теги UD подряд номерами строк `ud_strings`: часть речи, число признаков, пары «имя, значение»,
    /// число помет, пометы
    pub ud_tags: Vec<u16>,
    /// Буквы словоформ и окончаний: байт буквы в ключах автоматов - ее номер с единицы
    pub alphabet: String,
    /// Префиксы и суффиксы форм; строки в таблице не повторяются
    pub prefixes: Vec<String>,
    pub suffixes: Vec<String>,
    /// Начало каждой парадигмы в `paradigms` и общий конец
    pub paradigm_offsets: Vec<u32>,
    /// Парадигмы подряд: номера суффиксов, тегов и префиксов форм тремя блоками
    pub paradigms: Vec<u16>,
    /// Таблицы словоизменения подряд: номер парадигмы, затем `(форма леммы << 16) | тег UD`
    /// каждой ее формы
    pub tables: Vec<u32>,
    /// Автомат «словоформа с «е» на месте «ё» - значение `WordValue`»
    pub words: Vec<u8>,
    /// Списки разборов: длина, затем `(таблица << 16) | форма` каждого разбора
    pub records: Vec<u32>,
    /// Написания ключей, которые не уместились в значение автомата: число написаний, места «ё»
    /// (два числа) и начало списка разборов каждого, затем число списков частот, места «ё»
    /// и начало списка каждого
    pub variants: Vec<u32>,
    /// Списки частот: длина (с битом `ENTRY_KEYS` - ключи списка формы таблиц), затем пары
    /// «тег или форма таблицы, доля в миллионных»
    pub probability_records: Vec<u32>,
    /// Автомат «индекс и окончание - начало списка в `guess_records`» для угадывания
    pub guesses: Vec<u8>,
    /// Списки форм для угадывания: длина, затем пары `(таблица << 16) | форма`, число словоформ
    pub guess_records: Vec<u32>,
}

/// Значение ключа в автомате словоформ
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WordValue {
    /// Одно написание: места «ё», начало списка разборов и начало списка частот этого написания
    Single {
        yo: u64,
        records: u32,
        probabilities: Option<u32>,
    },
    /// Написания и списки частот - в секции `variants` с этого места
    Many(u32),
}

/// Одно написание укладывается в значение автомата, если начало списка разборов меньше 2²⁴,
/// начало списка частот - меньше 2²⁰ - 1, а «ё» стоят среди первых 19 букв
const RECORDS_BITS: u32 = 24;
const PROBABILITIES_BITS: u32 = 20;
const YO_BITS: u32 = 19;
const MANY: u64 = 1 << 63;

impl WordValue {
    /// Значение автомата; `None`, если написание не укладывается в значение
    #[cfg(any(test, feature = "build"))]
    pub fn pack(self) -> Option<u64> {
        let fits = |value: u64, bits: u32| (value >> bits == 0).then_some(value);
        match self {
            WordValue::Single {
                yo,
                records,
                probabilities,
            } => {
                let probabilities = probabilities.map_or(0, |offset| u64::from(offset) + 1);
                Some(
                    fits(u64::from(records), RECORDS_BITS)?
                        | fits(probabilities, PROBABILITIES_BITS)? << RECORDS_BITS
                        | fits(yo, YO_BITS)? << (RECORDS_BITS + PROBABILITIES_BITS),
                )
            }
            WordValue::Many(offset) => Some(MANY | u64::from(offset)),
        }
    }

    pub fn unpack(value: u64) -> Self {
        if value & MANY != 0 {
            return WordValue::Many(value as u32);
        }
        let field = |shift: u32, bits: u32| (value >> shift) & ((1 << bits) - 1);
        let probabilities = field(RECORDS_BITS, PROBABILITIES_BITS);
        WordValue::Single {
            yo: field(RECORDS_BITS + PROBABILITIES_BITS, YO_BITS),
            records: field(0, RECORDS_BITS) as u32,
            probabilities: probabilities.checked_sub(1).map(|offset| offset as u32),
        }
    }
}

/// Границы секций в байтах файла
#[derive(Clone, Debug, Default)]
pub(crate) struct Layout {
    pub meta: Range<usize>,
    pub tags: Range<usize>,
    pub ud_strings: Range<usize>,
    pub ud_tags: Range<usize>,
    pub alphabet: Range<usize>,
    pub prefixes: Range<usize>,
    pub suffixes: Range<usize>,
    pub paradigm_offsets: Range<usize>,
    pub paradigms: Range<usize>,
    pub tables: Range<usize>,
    pub words: Range<usize>,
    pub records: Range<usize>,
    pub variants: Range<usize>,
    pub probability_records: Range<usize>,
    pub guesses: Range<usize>,
    pub guess_records: Range<usize>,
}

impl Layout {
    /// Секции чисел, которые читаются прямо из файла, и размер их чисел в байтах
    pub fn numbers(&self) -> [(&Range<usize>, usize); 7] {
        [
            (&self.paradigm_offsets, 4),
            (&self.paradigms, 2),
            (&self.tables, 4),
            (&self.records, 4),
            (&self.variants, 4),
            (&self.probability_records, 4),
            (&self.guess_records, 4),
        ]
    }
}

#[cfg(any(test, feature = "build"))]
pub(crate) fn write(sections: &Sections) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    let mut section = |bytes: &[u8]| {
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(bytes);
        out.resize(out.len().next_multiple_of(ALIGN), 0);
    };
    section(sections.meta.as_bytes());
    section(&strings_bytes(&sections.tags));
    section(&strings_bytes(&sections.ud_strings));
    section(&u16_bytes(&sections.ud_tags));
    section(sections.alphabet.as_bytes());
    section(&strings_bytes(&sections.prefixes));
    section(&strings_bytes(&sections.suffixes));
    section(&u32_bytes(&sections.paradigm_offsets));
    section(&u16_bytes(&sections.paradigms));
    section(&u32_bytes(&sections.tables));
    section(&sections.words);
    section(&u32_bytes(&sections.records));
    section(&u32_bytes(&sections.variants));
    section(&u32_bytes(&sections.probability_records));
    section(&sections.guesses);
    section(&u32_bytes(&sections.guess_records));
    let checksum = crc32fast::hash(&out[HEADER..]);
    out[12..HEADER].copy_from_slice(&checksum.to_le_bytes());
    out
}

/// Проверяет сигнатуру и версию формата в начале файла
fn check_header(data: &[u8]) -> Result<(), Error> {
    if data.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
        return Err(Error::Format("нет сигнатуры словаря ruMorph".into()));
    }
    let Some(header) = data.get(..HEADER) else {
        return Err(truncated());
    };
    let version = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if version != VERSION {
        return Err(Error::Format(format!(
            "версия формата {version}, поддерживается {VERSION}"
        )));
    }
    Ok(())
}

/// Границы секций файла: проверяются сигнатура, версия, длины секций и контрольная сумма
pub(crate) fn layout(data: &[u8]) -> Result<Layout, Error> {
    check_header(data)?;
    let mut at = HEADER;
    let mut section = |width: usize| -> Result<Range<usize>, Error> {
        let len = data.get(at..at + 8).ok_or_else(truncated)?;
        let len = u64::from_le_bytes(len.try_into().map_err(|_| truncated())?);
        let start = at + 8;
        let end = usize::try_from(len)
            .ok()
            .and_then(|len| start.checked_add(len))
            .filter(|&end| end <= data.len())
            .ok_or_else(truncated)?;
        if !(end - start).is_multiple_of(width) {
            return Err(Error::Format(format!("длина секции не кратна {width}")));
        }
        at = end.next_multiple_of(ALIGN);
        if at > data.len() {
            return Err(truncated());
        }
        Ok(start..end)
    };
    let layout = Layout {
        meta: section(1)?,
        tags: section(1)?,
        ud_strings: section(1)?,
        ud_tags: section(2)?,
        alphabet: section(1)?,
        prefixes: section(1)?,
        suffixes: section(1)?,
        paradigm_offsets: section(4)?,
        paradigms: section(2)?,
        tables: section(4)?,
        words: section(1)?,
        records: section(4)?,
        variants: section(4)?,
        probability_records: section(4)?,
        guesses: section(1)?,
        guess_records: section(4)?,
    };
    if at != data.len() {
        return Err(extra_data());
    }
    // Сумма читает весь файл, поэтому считается после быстрых проверок
    let checksum = u32::from_le_bytes(data[12..HEADER].try_into().map_err(|_| truncated())?);
    if crc32fast::hash(&data[HEADER..]) != checksum {
        return Err(Error::Format(
            "контрольная сумма не совпадает: файл поврежден".into(),
        ));
    }
    Ok(layout)
}

/// Читает словарь из потока: сигнатура и версия проверяются до чтения остатка, а секции
/// читаются по мере поступления, поэтому поток без конца не читается целиком
pub(crate) fn read(mut input: impl Read) -> Result<Storage, Error> {
    let mut buffer = Buffer::default();
    buffer.fill(&mut input, HEADER as u64)?;
    check_header(buffer.bytes())?;
    for _ in 0..SECTIONS {
        if buffer.fill(&mut input, 8)? < 8 {
            return Err(truncated());
        }
        let bytes = buffer.bytes();
        let len = u64::from_le_bytes(
            bytes[bytes.len() - 8..]
                .try_into()
                .map_err(|_| truncated())?,
        );
        let padded = len
            .checked_next_multiple_of(ALIGN as u64)
            .ok_or_else(truncated)?;
        if buffer.fill(&mut input, padded)? < padded {
            return Err(truncated());
        }
    }
    if buffer.fill(&mut input, 1)? > 0 {
        return Err(extra_data());
    }
    buffer.words.truncate(buffer.len.div_ceil(ALIGN));
    Ok(Storage::Owned(buffer.words, buffer.len))
}

/// Байты потока, выровненные по 8: из них собирается словарь без второй копии
#[derive(Default)]
struct Buffer {
    words: Vec<u64>,
    len: usize,
}

impl Buffer {
    fn bytes(&self) -> &[u8] {
        &bytemuck::cast_slice(&self.words)[..self.len]
    }

    /// Дочитывает `count` байт; меньше - только в конце потока
    fn fill(&mut self, input: &mut impl Read, count: u64) -> Result<u64, Error> {
        const CHUNK: u64 = 1 << 16;
        let mut done = 0;
        while done < count {
            let want = (count - done).min(CHUNK) as usize;
            self.words.resize((self.len + want).div_ceil(ALIGN), 0);
            let space = &mut bytemuck::cast_slice_mut::<u64, u8>(&mut self.words)
                [self.len..self.len + want];
            match input.read(space) {
                Ok(0) => break,
                Ok(read) => {
                    self.len += read;
                    done += read as u64;
                }
                Err(err) if err.kind() == ErrorKind::Interrupted => {}
                Err(err) => return Err(err.into()),
            }
        }
        Ok(done)
    }
}

fn truncated() -> Error {
    Error::Format("файл обрезан".into())
}

fn extra_data() -> Error {
    Error::Format("лишние данные после последней секции".into())
}

/// Байты словаря: отображенный в память файл, данные программы или копия, выровненная по 8 байт
pub(crate) enum Storage {
    Owned(Vec<u64>, usize),
    Mapped(memmap2::Mmap),
    Static(&'static [u8]),
}

impl Storage {
    pub fn copy(data: &[u8]) -> Self {
        let mut words = vec![0u64; data.len().div_ceil(ALIGN)];
        bytemuck::cast_slice_mut::<u64, u8>(&mut words)[..data.len()].copy_from_slice(data);
        Storage::Owned(words, data.len())
    }

    /// Данные программы читаются без копии, если они выровнены по 8 байт
    pub fn of_static(data: &'static [u8]) -> Self {
        if (data.as_ptr() as usize).is_multiple_of(ALIGN) {
            Storage::Static(data)
        } else {
            Storage::copy(data)
        }
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Storage::Owned(words, len) => &bytemuck::cast_slice(words)[..*len],
            Storage::Mapped(map) => map,
            Storage::Static(data) => data,
        }
    }

    /// Словарь с числами секций `layout` в порядке байт платформы: на big-endian - копия
    pub fn native(self, layout: &Layout) -> Self {
        if cfg!(target_endian = "little") {
            self
        } else {
            self.swapped(layout)
        }
    }

    /// Копия с обратным порядком байт в числах секций `layout`
    fn swapped(&self, layout: &Layout) -> Self {
        let mut copy = Storage::copy(self.bytes());
        if let Storage::Owned(words, _) = &mut copy {
            let bytes = bytemuck::cast_slice_mut::<u64, u8>(words);
            for (range, width) in layout.numbers() {
                for number in bytes[range.clone()].chunks_exact_mut(width) {
                    number.reverse();
                }
            }
        }
        copy
    }
}

/// Байты секции словаря для автомата: автомат держит словарь, пока он нужен
#[derive(Clone)]
pub(crate) struct Slice {
    pub storage: std::sync::Arc<Storage>,
    pub range: Range<usize>,
}

impl AsRef<[u8]> for Slice {
    fn as_ref(&self) -> &[u8] {
        &self.storage.bytes()[self.range.clone()]
    }
}

/// Числа секции, выровненной при загрузке; невыровненная секция - пустая
pub(crate) fn numbers<T: bytemuck::Pod>(bytes: &[u8]) -> &[T] {
    bytemuck::try_cast_slice(bytes).unwrap_or_default()
}

/// Таблица строк: число строк, концы строк, затем строки подряд
#[derive(Clone, Debug, Default)]
pub(crate) struct Strings {
    text: String,
    ends: Vec<u32>,
}

impl Strings {
    pub fn read(bytes: &[u8]) -> Result<Self, Error> {
        let invalid = |msg: &str| Error::Format(msg.into());
        let number = |at: usize| -> Option<u32> {
            Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
        };
        let count = number(0).ok_or_else(|| invalid("таблица строк обрезана"))? as usize;
        // Конец каждой строки занимает 4 байта: иначе число строк из файла задало бы любую аллокацию
        let start = count
            .checked_mul(4)
            .and_then(|len| len.checked_add(4))
            .filter(|&start| start <= bytes.len())
            .ok_or_else(|| invalid("число строк больше размера таблицы"))?;
        let text = std::str::from_utf8(&bytes[start..])
            .map_err(|_| invalid("строка не в UTF-8"))?
            .to_owned();
        let ends: Vec<u32> = (0..count)
            .map(|i| number(4 + 4 * i).unwrap_or(u32::MAX))
            .collect();
        let mut previous = 0;
        for &end in &ends {
            let end = end as usize;
            if end < previous || !text.is_char_boundary(end) {
                return Err(invalid("неверные границы строк в таблице"));
            }
            previous = end;
        }
        if previous != text.len() {
            return Err(invalid("лишние данные в таблице строк"));
        }
        Ok(Strings { text, ends })
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn get(&self, index: usize) -> Option<&str> {
        let end = *self.ends.get(index)? as usize;
        let start = index.checked_sub(1).map_or(0, |i| self.ends[i] as usize);
        self.text.get(start..end)
    }

    /// Строка, которая точно есть в таблице: номер проверен при загрузке
    pub fn at(&self, index: usize) -> &str {
        self.get(index).unwrap_or_default()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &str> {
        (0..self.len()).map(|index| self.at(index))
    }
}

#[cfg(any(test, feature = "build"))]
fn strings_bytes(values: &[String]) -> Vec<u8> {
    let mut out = (values.len() as u32).to_le_bytes().to_vec();
    let mut end = 0;
    for value in values {
        end += value.len() as u32;
        out.extend_from_slice(&end.to_le_bytes());
    }
    for value in values {
        out.extend_from_slice(value.as_bytes());
    }
    out
}

#[cfg(any(test, feature = "build"))]
fn u16_bytes(values: &[u16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

#[cfg(any(test, feature = "build"))]
fn u32_bytes(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Секции файла словаря обратно в записываемом виде
    pub(crate) fn read(data: &[u8]) -> Sections {
        let layout = layout(data).unwrap();
        let bytes = |range: &Range<usize>| &data[range.clone()];
        let text = |range: &Range<usize>| String::from_utf8(bytes(range).to_vec()).unwrap();
        let strings = |range: &Range<usize>| -> Vec<String> {
            Strings::read(bytes(range))
                .unwrap()
                .iter()
                .map(str::to_owned)
                .collect()
        };
        let u16s = |range: &Range<usize>| -> Vec<u16> {
            bytes(range)
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&item| u16::from_le_bytes(item))
                .collect()
        };
        let u32s = |range: &Range<usize>| -> Vec<u32> {
            bytes(range)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&item| u32::from_le_bytes(item))
                .collect()
        };
        Sections {
            meta: text(&layout.meta),
            tags: strings(&layout.tags),
            ud_strings: strings(&layout.ud_strings),
            ud_tags: u16s(&layout.ud_tags),
            alphabet: text(&layout.alphabet),
            prefixes: strings(&layout.prefixes),
            suffixes: strings(&layout.suffixes),
            paradigm_offsets: u32s(&layout.paradigm_offsets),
            paradigms: u16s(&layout.paradigms),
            tables: u32s(&layout.tables),
            words: bytes(&layout.words).to_vec(),
            records: u32s(&layout.records),
            variants: u32s(&layout.variants),
            probability_records: u32s(&layout.probability_records),
            guesses: bytes(&layout.guesses).to_vec(),
            guess_records: u32s(&layout.guess_records),
        }
    }

    /// Файл с пересчитанной контрольной суммой: порча, которую сумма не ловит
    pub(crate) fn with_checksum(mut data: Vec<u8>) -> Vec<u8> {
        if data.len() >= HEADER {
            let checksum = crc32fast::hash(&data[HEADER..]);
            data[12..HEADER].copy_from_slice(&checksum.to_le_bytes());
        }
        data
    }

    fn sample() -> Sections {
        Sections {
            meta: "{}".into(),
            tags: vec!["NOUN".into(), "VERB".into()],
            ud_strings: vec!["NOUN".into(), "Case".into(), "Nom".into()],
            ud_tags: vec![0, 1, 1, 2, 0],
            alphabet: "аб".into(),
            prefixes: vec![String::new(), "по".into()],
            suffixes: vec![String::new(), "а".into()],
            paradigm_offsets: vec![0, 3],
            paradigms: vec![0, 0, 0],
            tables: vec![0, 0],
            words: vec![1, 2, 3],
            records: vec![1, 0],
            ..Sections::default()
        }
    }

    #[test]
    fn word_values() {
        let values = [
            WordValue::Single {
                yo: 0,
                records: 0,
                probabilities: None,
            },
            WordValue::Single {
                yo: (1 << YO_BITS) - 1,
                records: (1 << RECORDS_BITS) - 1,
                probabilities: Some((1 << PROBABILITIES_BITS) - 2),
            },
            WordValue::Many(0),
            WordValue::Many(u32::MAX),
        ];
        for value in values {
            assert_eq!(WordValue::unpack(value.pack().unwrap()), value);
        }
        for value in [
            WordValue::Single {
                yo: 1 << YO_BITS,
                records: 0,
                probabilities: None,
            },
            WordValue::Single {
                yo: 0,
                records: 1 << RECORDS_BITS,
                probabilities: None,
            },
            WordValue::Single {
                yo: 0,
                records: 0,
                probabilities: Some((1 << PROBABILITIES_BITS) - 1),
            },
        ] {
            assert_eq!(value.pack(), None);
        }
    }

    #[test]
    fn roundtrip() {
        let data = write(&sample());
        let back = read(&data);
        assert_eq!(write(&back), data);
        assert_eq!(back.ud_strings, sample().ud_strings);
        assert_eq!(back.ud_tags, sample().ud_tags);
        assert_eq!(back.paradigms, sample().paradigms);
        assert_eq!(back.words, sample().words);
    }

    #[test]
    fn sections_are_aligned() {
        let data = write(&sample());
        let layout = layout(&data).unwrap();
        for (range, _) in layout.numbers() {
            assert_eq!(range.start % ALIGN, 0);
        }
        assert_eq!(data.len() % ALIGN, 0);
        let storage = Storage::copy(&data);
        assert_eq!(storage.bytes(), data);
        assert_eq!(storage.bytes().as_ptr() as usize % ALIGN, 0);
        let storage = storage.native(&layout);
        assert_eq!(numbers::<u32>(&storage.bytes()[layout.records]), [1, 0]);
    }

    #[test]
    fn swaps_numbers_for_big_endian() {
        let mut sections = sample();
        sections.paradigms = vec![1, 0x0102, 0];
        let data = write(&sections);
        let layout = layout(&data).unwrap();
        let storage = Storage::copy(&data);
        let swapped = storage.swapped(&layout);
        // Числа файла записаны little-endian: после перестановки байт они читаются как big-endian
        assert_eq!(
            numbers::<u16>(&swapped.bytes()[layout.paradigms.clone()]),
            [1u16, 0x0102, 0].map(|number| u16::from_ne_bytes(number.to_be_bytes()))
        );
        assert_eq!(
            numbers::<u32>(&swapped.bytes()[layout.records.clone()]),
            [1u32, 0].map(|number| u32::from_ne_bytes(number.to_be_bytes()))
        );
        // Строки и автоматы не меняются
        assert_eq!(
            swapped.bytes()[layout.words.clone()],
            storage.bytes()[layout.words.clone()]
        );
        let native = storage.native(&layout);
        assert_eq!(
            numbers::<u16>(&native.bytes()[layout.paradigms.clone()]),
            [1, 0x0102, 0]
        );
    }

    #[test]
    fn rejects_damaged_files() {
        let data = write(&sample());
        // Любой испорченный байт после заголовка ловит контрольная сумма или проверка длин секций
        for at in HEADER..data.len() {
            let mut damaged = data.clone();
            damaged[at] ^= 0x10;
            assert!(layout(&damaged).is_err(), "{at}");
        }
        let mut damaged = data.clone();
        damaged[HEADER + 8] ^= 0x10;
        let err = layout(&damaged).err().unwrap();
        assert!(err.to_string().contains("контрольная сумма"), "{err}");
        for len in 0..data.len() {
            assert!(
                layout(&with_checksum(data[..len].to_vec())).is_err(),
                "{len}"
            );
        }
        let mut longer = data.clone();
        longer.extend_from_slice(&[0; 8]);
        let err = layout(&with_checksum(longer)).err().unwrap();
        assert!(err.to_string().contains("лишние данные"), "{err}");
    }

    #[test]
    fn checks_sections_before_checksum() {
        // Заголовок словаря и нули: длины секций нулевые, и остаток файла лишний
        let mut data = write(&sample())[..HEADER].to_vec();
        data.resize(1 << 20, 0);
        let err = layout(&data).err().unwrap();
        assert!(err.to_string().contains("лишние данные"), "{err}");
    }

    /// Поток, который отдает начало `head`, а затем нули без конца
    struct Endless {
        head: Vec<u8>,
        at: usize,
    }

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let rest = self.head.get(self.at..).unwrap_or_default();
            let len = rest.len().min(buf.len());
            buf[..len].copy_from_slice(&rest[..len]);
            buf[len..].fill(0);
            self.at += buf.len();
            Ok(buf.len())
        }
    }

    #[test]
    fn reads_stream_by_sections() {
        let data = write(&sample());
        let storage = super::read(data.as_slice()).unwrap();
        assert_eq!(storage.bytes(), data);
        assert_eq!(storage.bytes().as_ptr() as usize % ALIGN, 0);
        for len in 0..data.len() {
            assert!(
                matches!(super::read(&data[..len]), Err(Error::Format(_))),
                "{len}"
            );
        }
        let mut longer = data.clone();
        longer.push(0);
        let err = super::read(longer.as_slice()).err().unwrap();
        assert!(err.to_string().contains("лишние данные"), "{err}");
        // Длина секции больше потока: читается только то, что есть
        let mut huge = data[..HEADER].to_vec();
        huge.extend_from_slice(&(u64::MAX - 7).to_le_bytes());
        huge.extend_from_slice(b"meta");
        let err = super::read(huge.as_slice()).err().unwrap();
        assert!(err.to_string().contains("файл обрезан"), "{err}");
        let err = super::read(&huge[..HEADER + 4]).err().unwrap();
        assert!(err.to_string().contains("файл обрезан"), "{err}");
    }

    #[test]
    fn stops_reading_endless_stream() {
        // Нули без сигнатуры (`/dev/zero`) и заголовок словаря с нулями после него
        let err = super::read(Endless {
            head: Vec::new(),
            at: 0,
        })
        .err()
        .unwrap();
        assert!(err.to_string().contains("нет сигнатуры"), "{err}");
        let head = write(&sample())[..HEADER].to_vec();
        let err = super::read(Endless { head, at: 0 }).err().unwrap();
        assert!(err.to_string().contains("лишние данные"), "{err}");
    }

    #[test]
    fn reports_stream_errors() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("диск"))
            }
        }
        assert!(matches!(super::read(Broken), Err(Error::Io(_))));
        // Прерванное чтение повторяется
        struct Interrupted(Vec<u8>, bool);
        impl Read for Interrupted {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.1 = !self.1;
                if self.1 {
                    return Err(ErrorKind::Interrupted.into());
                }
                let len = self.0.len().min(buf.len()).min(3);
                buf[..len].copy_from_slice(&self.0[..len]);
                self.0.drain(..len);
                Ok(len)
            }
        }
        let data = write(&sample());
        let storage = super::read(Interrupted(data.clone(), false)).unwrap();
        assert_eq!(storage.bytes(), data);
    }

    #[test]
    fn rejects_section_lengths() {
        let data = write(&sample());
        // Длина секции meta больше остатка файла или не кратна размеру чисел секции
        for huge in [u64::MAX, u64::MAX - 1, data.len() as u64] {
            let mut broken = data.clone();
            broken[HEADER..HEADER + 8].copy_from_slice(&huge.to_le_bytes());
            let err = layout(&with_checksum(broken)).err().unwrap();
            assert!(err.to_string().contains("файл обрезан"), "{huge}: {err}");
        }
        // Длина секции ud_tags не кратна 2: 3 байта вместо 10
        let mut broken = data.clone();
        let at = layout(&data).unwrap().ud_tags.start - 8;
        let len = u64::from_le_bytes(broken[at..at + 8].try_into().unwrap());
        assert_eq!(len, 10);
        broken[at..at + 8].copy_from_slice(&3u64.to_le_bytes());
        let err = layout(&with_checksum(broken)).err().unwrap();
        assert!(err.to_string().contains("не кратна 2"), "{err}");
    }

    #[test]
    fn rejects_foreign_data_and_other_version() {
        assert!(matches!(layout(b"not a dictionary"), Err(Error::Format(_))));
        assert!(matches!(layout(b""), Err(Error::Format(_))));
        assert!(matches!(layout(b"RUMORPH\0"), Err(Error::Format(_))));
        let mut data = write(&sample());
        data[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&4u32.to_le_bytes());
        let err = layout(&data).err().unwrap();
        assert!(err.to_string().contains("версия формата 4"), "{err}");
    }

    #[test]
    fn string_tables() {
        let table = Strings::read(&strings_bytes(&["".into(), "по".into(), "а".into()])).unwrap();
        assert_eq!(table.iter().collect::<Vec<_>>(), ["", "по", "а"]);
        assert_eq!(table.get(3), None);
        let mut huge = u32::MAX.to_le_bytes().to_vec();
        huge.extend_from_slice(&[0; 8]);
        assert!(Strings::read(&huge).is_err());
        // Конец строки внутри буквы, убывающие концы и лишний текст
        for (ends, text) in [(&[1u32][..], "п"), (&[2, 1][..], "по"), (&[1][..], "ab")] {
            let mut bytes = (ends.len() as u32).to_le_bytes().to_vec();
            for end in ends {
                bytes.extend_from_slice(&end.to_le_bytes());
            }
            bytes.extend_from_slice(text.as_bytes());
            assert!(Strings::read(&bytes).is_err(), "{ends:?} {text}");
        }
        assert!(Strings::read(&[0, 0, 0]).is_err());
        assert!(Strings::read(&[0, 0, 0, 0, 0xFF]).is_err());
    }
}
