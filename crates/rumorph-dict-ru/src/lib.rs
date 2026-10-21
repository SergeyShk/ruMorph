//! Словарь русского языка для морфологического анализатора ruMorph: словарь OpenCorpora
//! в формате ruMorph с частотами разборов по корпусу OpenCorpora и трибанкам UD Taiga и GSD,
//! лицензия CC BY-SA 4.0
//!
//! Словарь загружается крейтом `rumorph`: `rumorph::Dictionary::bundled()`

/// Байты, выровненные по 8: так словарь читается прямо из памяти программы, без копии
#[repr(C, align(8))]
struct Aligned<T: ?Sized>(T);

static ALIGNED: &Aligned<[u8]> = &Aligned(*include_bytes!(concat!(env!("OUT_DIR"), "/ru.dict")));

/// Файл словаря; пустой, если крейт собран без словаря
pub static DICTIONARY: &[u8] = &ALIGNED.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned() {
        assert_eq!(DICTIONARY.as_ptr() as usize % 8, 0);
        assert!(DICTIONARY.is_empty() || DICTIONARY.starts_with(b"RUMORPH\0"));
    }
}
