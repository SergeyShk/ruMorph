//! Однобайтовая запись слов в ключах автоматов: байт буквы - ее номер в алфавите словаря.
//! В ключе словоформы «ё» записана как «е», а места «ё» хранятся отдельно маской

use std::cmp::Ordering;

use crate::Error;

/// Наибольшая длина ключа в буквах: длиннее словоформ в словаре нет
pub(crate) const MAX_KEY: usize = 64;

/// Буквы словаря и их байты
#[derive(Clone)]
pub(crate) struct Alphabet {
    /// Буква каждого байта; у байта без буквы - '\0'
    letters: [char; 256],
    /// Байты строчных букв от «а» до «я»
    cyrillic: [u8; 32],
    ascii: [u8; 128],
    /// Байты остальных букв по порядку букв
    other: Vec<(char, u8)>,
}

impl Alphabet {
    /// Алфавит из букв по порядку: байт буквы - ее номер с единицы
    pub fn new(letters: &str) -> Result<Self, Error> {
        let mut alphabet = Alphabet {
            letters: ['\0'; 256],
            cyrillic: [0; 32],
            ascii: [0; 128],
            other: Vec::new(),
        };
        for (index, letter) in letters.chars().enumerate() {
            let code = u8::try_from(index + 1)
                .map_err(|_| Error::Format("больше 255 букв в алфавите".into()))?;
            if letter == '\0' {
                return Err(Error::Format("символ U+0000 в алфавите".into()));
            }
            if alphabet.code(letter).is_some() {
                return Err(Error::Format("буква алфавита повторяется".into()));
            }
            alphabet.letters[usize::from(code)] = letter;
            match letter {
                'а'..='я' => alphabet.cyrillic[letter as usize - 'а' as usize] = code,
                _ if letter.is_ascii() => alphabet.ascii[letter as usize] = code,
                _ => alphabet.other.push((letter, code)),
            }
        }
        // Повтор буквы вне «а»-«я» и ASCII виден только после сортировки
        alphabet.other.sort_unstable();
        if alphabet.other.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(Error::Format("буква алфавита повторяется".into()));
        }
        Ok(alphabet)
    }

    /// Добавляет буквы слова `word` в нижнем регистре, которых нет в алфавите, на свободные байты:
    /// байты прежних букв и ключи автомата не меняются. `false`, если свободных байтов не хватило
    pub fn extend(&mut self, word: &str) -> bool {
        for letter in word.chars().flat_map(char::to_lowercase) {
            let letter = if letter == 'ё' { 'е' } else { letter };
            if letter == '\0' || self.code(letter).is_some() {
                continue;
            }
            let Some(code) = (1..=u8::MAX).find(|&code| self.letters[usize::from(code)] == '\0')
            else {
                return false;
            };
            self.letters[usize::from(code)] = letter;
            match letter {
                'а'..='я' => self.cyrillic[letter as usize - 'а' as usize] = code,
                _ if letter.is_ascii() => self.ascii[letter as usize] = code,
                _ => {
                    let at = self.other.partition_point(|&(known, _)| known < letter);
                    self.other.insert(at, (letter, code));
                }
            }
        }
        true
    }

    fn code(&self, letter: char) -> Option<u8> {
        let code = match letter {
            'а'..='я' => self.cyrillic[letter as usize - 'а' as usize],
            _ if letter.is_ascii() => self.ascii[letter as usize],
            _ => match self
                .other
                .binary_search_by(|&(known, _)| known.cmp(&letter))
            {
                Ok(at) => self.other[at].1,
                Err(_) => 0,
            },
        };
        (code != 0).then_some(code)
    }

    /// Ключ словоформы из текста: буквы в нижнем регистре, «ё» - как «е»; `None`, если такой
    /// словоформы в словаре быть не может
    pub fn input_key(&self, word: &str) -> Option<Key> {
        let mut key = Key::new();
        for letter in word.chars() {
            // Заглавные кириллицы и латиницы - без таблиц Unicode
            let lower = match letter {
                'А'..='Я' => char::from_u32(letter as u32 + 0x20)?,
                'Ё' => 'ё',
                _ if letter.is_ascii() => letter.to_ascii_lowercase(),
                'а'..='я' | 'ё' => letter,
                _ => {
                    for lower in letter.to_lowercase() {
                        self.push(&mut key, lower)?;
                    }
                    continue;
                }
            };
            self.push(&mut key, lower)?;
        }
        Some(key)
    }

    /// Ключ словоформы в написании словаря: регистр не меняется
    pub fn spelling_key(&self, spelling: &str) -> Option<Key> {
        let mut key = Key::new();
        for letter in spelling.chars() {
            self.push(&mut key, letter)?;
        }
        Some(key)
    }

    fn push(&self, key: &mut Key, letter: char) -> Option<()> {
        if letter == 'ё' {
            let at = key.len;
            key.push(self.code('е')?)?;
            key.yo |= 1 << at;
            Some(())
        } else {
            key.push(self.code(letter)?)
        }
    }

    /// Ключ из байта `first` и букв `text` как есть: окончания для угадывания
    pub fn key(&self, first: u8, text: &str) -> Option<Key> {
        let mut key = Key::new();
        key.push(first)?;
        for letter in text.chars() {
            key.push(self.code(letter)?)?;
        }
        Some(key)
    }

    /// Написание ключа словоформы с «ё» на местах `yo`
    pub fn spelling(&self, key: &Key, yo: u64) -> String {
        self.decode(key.bytes(), yo)
    }

    /// Написание байтов ключа словоформы с «ё» на местах `yo`
    pub fn decode(&self, bytes: &[u8], yo: u64) -> String {
        let mut spelling = String::with_capacity(2 * bytes.len());
        for (at, &code) in bytes.iter().enumerate() {
            match self.letters[usize::from(code)] {
                'е' if yo >> at & 1 == 1 => spelling.push('ё'),
                letter => spelling.push(letter),
            }
        }
        spelling
    }
}

/// Ключ автомата: байты букв и места «ё»
pub(crate) struct Key {
    bytes: [u8; MAX_KEY],
    len: usize,
    /// Места «ё», записанной как «е»: бит номера буквы
    pub yo: u64,
}

impl Key {
    fn new() -> Self {
        Key {
            bytes: [0; MAX_KEY],
            len: 0,
            yo: 0,
        }
    }

    fn push(&mut self, byte: u8) -> Option<()> {
        *self.bytes.get_mut(self.len)? = byte;
        self.len += 1;
        Some(())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Порядок написаний одного ключа по местам «ё» слева направо: сначала без «ё», затем с первой
/// «ё» на самом левом месте и т. д. («все», «всё»; «е», «ё» в первом слоге раньше второго)
pub(crate) fn yo_order(a: u64, b: u64) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        match (a, b) {
            (0, 0) => return Ordering::Equal,
            (0, _) => return Ordering::Less,
            (_, 0) => return Ordering::Greater,
            _ => {}
        }
        let (left, right) = (a.trailing_zeros(), b.trailing_zeros());
        if left != right {
            return left.cmp(&right);
        }
        a &= a - 1;
        b &= b - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_of_words() {
        let alphabet = Alphabet::new("-абвеёжья’").unwrap();
        let key = alphabet.input_key("ЁЖ").unwrap();
        assert_eq!(key.bytes(), [5, 7]);
        assert_eq!(key.yo, 1);
        assert_eq!(alphabet.spelling(&key, key.yo), "ёж");
        assert_eq!(alphabet.spelling(&key, 0), "еж");
        assert_eq!(alphabet.input_key("еж").unwrap().bytes(), key.bytes());
        assert_eq!(alphabet.input_key("вабья’-").unwrap().bytes().len(), 7);
        assert!(alphabet.input_key("ежик").is_none());
        assert!(alphabet.spelling_key("Еж").is_none());
        assert!(alphabet.input_key(&"а".repeat(MAX_KEY + 1)).is_none());
        assert_eq!(
            alphabet.input_key(&"а".repeat(MAX_KEY)).unwrap().len,
            MAX_KEY
        );
        assert_eq!(alphabet.key(1, "ёж").unwrap().bytes(), [1, 6, 7]);
        assert!(alphabet.key(1, "к").is_none());
    }

    #[test]
    fn extends_alphabet() {
        let mut alphabet = Alphabet::new("абвеёж").unwrap();
        let before = alphabet.input_key("ёж").unwrap().bytes().to_vec();
        assert!(alphabet.input_key("Ўёж").is_none());
        assert!(alphabet.extend("ўёжz"));
        assert_eq!(alphabet.input_key("ёж").unwrap().bytes(), before);
        let key = alphabet.input_key("ЎЁЖZ").unwrap();
        assert_eq!(alphabet.spelling(&key, key.yo), "ўёжz");
        let letters: String = ('\u{100}'..).take(255).collect();
        let mut full = Alphabet::new(&letters).unwrap();
        assert!(!full.extend("я"));
    }

    #[test]
    fn rejects_broken_alphabets() {
        assert!(Alphabet::new("абa").is_ok());
        assert!(Alphabet::new("аба").is_err());
        assert!(Alphabet::new("’ё’").is_err());
        assert!(Alphabet::new("’ёі").is_ok());
        assert!(Alphabet::new("а\0").is_err());
        let letters: String = ('\u{100}'..).take(256).collect();
        assert!(Alphabet::new(&letters).is_err());
        assert!(Alphabet::new(&letters[..letters.len() - 2]).is_ok());
    }

    #[test]
    fn yo_positions_order() {
        let mut masks = vec![0b110, 0b100, 0b010, 0b000, 0b011];
        masks.sort_by(|&a, &b| yo_order(a, b));
        assert_eq!(masks, [0b000, 0b011, 0b010, 0b110, 0b100]);
    }
}
