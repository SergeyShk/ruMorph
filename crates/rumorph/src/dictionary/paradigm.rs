//! Модель класса парадигмы: выбор формы таблицы словоизменения незнакомого слова среди кандидатов
//! из списков окончаний. Посимвольный LSTM над словом с конца, оценка кандидата - скалярное
//! произведение вектора слова с суммой строк признаков кандидата: тега UD, суффиксов формы леммы
//! и формы, длины окончания. Модель выбирает среди нарицательных. Веса - секция словаря, файл
//! `scripts/train_paradigms.py`

use std::collections::HashMap;

use super::guess::GuessIndex;
use super::ud::UdTags;
use super::weights::{self, Entries, Lstm, MAX_DIM, Matrix, Tiled, add, dot, lstm};
use crate::Error;
use crate::format::Strings;

const MAGIC: &[u8; 8] = b"RUMPAR\0\0";
const FORMAT: u32 = 1;
const WHAT: &str = "модель класса парадигмы";
/// Число признаков кандидата из списка окончания: log(1 + число словоформ), доля, самое длинное
const NUMERIC: usize = 3;

fn invalid(message: &str) -> Error {
    weights::invalid(WHAT, message)
}

/// Записи файла весов `scripts/train_paradigms.py`
fn read(data: &[u8]) -> Result<Entries<'_>, Error> {
    weights::read(data, MAGIC, FORMAT, WHAT)
}

/// Веса модели класса парадигмы
#[derive(Debug)]
pub(crate) struct Model {
    max_chars: usize,
    /// Строка таблицы букв по букве: 1 - незнакомая
    chars: HashMap<char, u32>,
    keys: HashMap<String, u32>,
    suffixes: HashMap<String, u32>,
    char: Matrix,
    lstm: Lstm,
    hidden: Matrix,
    hidden_bias: Vec<f32>,
    /// Строки признаков: вектор и смещение в последнем столбце
    key: Matrix,
    lemma_suffix: Matrix,
    form_suffix: Matrix,
    length: Matrix,
    numeric: Vec<f32>,
}

impl Model {
    /// Веса из файла `scripts/train_paradigms.py`: размеры всех матриц проверяются
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let mut entries = read(data)?;
        let weights::Value::Numbers(config) = entries.take("config")? else {
            return Err(invalid("config - не числа"));
        };
        let &[dim, hidden, dchar, max_chars] = config.as_slice() else {
            return Err(invalid("в config не 4 числа"));
        };
        let [dim, hidden, dchar, max_chars] = [dim, hidden, dchar, max_chars].map(|v| v as usize);
        if [dim, hidden, dchar, max_chars]
            .iter()
            .any(|&value| value == 0 || value > MAX_DIM)
        {
            return Err(invalid("неверные размеры в config"));
        }
        // Строки 0 и 1 таблицы букв - дополнение и незнакомая, строка 0 признаков - пустая
        let mut chars = HashMap::new();
        for (number, text) in entries.strings("chars")?.iter().enumerate() {
            let mut letters = text.chars();
            let (Some(letter), None) = (letters.next(), letters.next()) else {
                return Err(invalid("в chars не одна буква"));
            };
            if chars.insert(letter, number as u32 + 2).is_some() {
                return Err(invalid("в chars повторяются строки"));
            }
        }
        let keys = entries.index("keys", 1)?;
        let suffixes = entries.index("suffixes", 1)?;
        let width = dim + 1;
        let lstm = Lstm {
            input: Tiled::new(&entries.matrix("lstm.ih", 4 * hidden, dchar)?),
            hidden: Tiled::new(&entries.matrix("lstm.hh", 4 * hidden, hidden)?),
            bias: entries.vector("lstm.bias", 4 * hidden)?,
        };
        Ok(Model {
            max_chars,
            char: entries.matrix("char", chars.len() + 2, dchar)?,
            lstm,
            hidden: entries.matrix("hidden.weight", dim, hidden)?,
            hidden_bias: entries.vector("hidden.bias", dim)?,
            key: entries.matrix("key", keys.len() + 1, width)?,
            lemma_suffix: entries.matrix("lemma_suffix", suffixes.len() + 1, width)?,
            form_suffix: entries.matrix("form_suffix", suffixes.len() + 1, width)?,
            length: entries.matrix("length", GuessIndex::MAX_ENDING, width)?,
            numeric: entries.vector("numeric", NUMERIC)?,
            chars,
            keys,
            suffixes,
        })
    }
}

/// Источники обучения из записи `sources` файла весов - строка JSON
#[cfg(any(test, feature = "build"))]
pub(crate) fn sources(data: &[u8]) -> Result<Option<String>, Error> {
    match read(data)?.map.remove("sources") {
        None => Ok(None),
        Some(weights::Value::Strings(strings)) if strings.len() == 1 => {
            Ok(Some(strings[0].to_owned()))
        }
        Some(_) => Err(invalid("sources - не одна строка")),
    }
}

/// Признаки кандидата для оценки
#[derive(Clone, Copy, Debug)]
pub(crate) struct Features {
    pub ud: u16,
    /// Номера суффиксов формы леммы и формы в таблице суффиксов словаря
    pub lemma_suffix: usize,
    pub form_suffix: usize,
    pub length: u8,
    pub count: u32,
    pub share: f32,
    pub longest: bool,
}

/// Слово для оценки кандидатов: вектор с единицей для смещений и его произведения со строками
/// длины окончания - они одни у всех кандидатов слова
#[derive(Debug)]
pub(crate) struct Word {
    vector: Vec<f32>,
    lengths: [f32; GuessIndex::MAX_ENDING],
}

/// Модель и строки признаков словаря: суммы строк ключей каждого тега UD и строки суффиксов
#[derive(Debug)]
pub(crate) struct Paradigms {
    model: Model,
    tags: Vec<f32>,
    /// Строка таблиц суффиксов модели по номеру суффикса словаря: 0 - суффикса не было при обучении
    suffix_rows: Vec<u32>,
}

impl Paradigms {
    pub fn new(model: Model, ud: &UdTags, suffixes: &Strings) -> Self {
        let width = model.key.cols;
        let mut tags = vec![0.0; ud.len() * width];
        for (number, tag) in ud.iter().enumerate() {
            let row = &mut tags[number * width..(number + 1) * width];
            let keys = std::iter::once(format!("P={}", tag.pos()))
                .chain(tag.feats().map(|(name, value)| format!("F={name}={value}")));
            for key in keys {
                if let Some(&key) = model.keys.get(&key) {
                    add(row, model.key.row(key as usize));
                }
            }
        }
        let suffix_rows = (0..suffixes.len())
            .map(|number| {
                suffixes
                    .get(number)
                    .and_then(|suffix| model.suffixes.get(suffix))
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        Paradigms {
            model,
            tags,
            suffix_rows,
        }
    }

    /// Слово в нижнем регистре для оценки кандидатов: LSTM читает последние буквы
    /// с конца, «ё» - как «е»
    pub fn word(&self, lower: &str) -> Word {
        let model = &self.model;
        let dchar = model.char.cols;
        let mut input = Vec::with_capacity(model.max_chars * dchar);
        for letter in lower.chars().rev().take(model.max_chars) {
            let letter = if letter == 'ё' { 'е' } else { letter };
            let row = model.chars.get(&letter).copied().unwrap_or(1);
            input.extend_from_slice(model.char.row(row as usize));
        }
        let state = lstm(&model.lstm, &input, input.len() / dchar);
        let mut vector = vec![0.0; model.hidden.rows + 1];
        model.hidden.apply(&state, &mut vector[..model.hidden.rows]);
        for (value, bias) in vector.iter_mut().zip(&model.hidden_bias) {
            *value = (*value + bias).tanh();
        }
        vector[model.hidden.rows] = 1.0;
        Word {
            lengths: std::array::from_fn(|length| dot(&vector, model.length.row(length))),
            vector,
        }
    }

    /// Оценка кандидата слова `word`
    pub fn score(&self, word: &Word, features: &Features) -> f32 {
        let model = &self.model;
        let width = model.key.cols;
        let ud = usize::from(features.ud);
        let suffix = |number: usize| self.suffix_rows.get(number).copied().unwrap_or(0) as usize;
        let tag = self.tags.get(ud * width..(ud + 1) * width).unwrap_or(&[]);
        let numeric = [
            (features.count as f32).ln_1p(),
            features.share,
            f32::from(u8::from(features.longest)),
        ];
        let length = usize::from(features.length.clamp(1, GuessIndex::MAX_ENDING as u8)) - 1;
        let vector = &word.vector;
        dot(vector, tag)
            + dot(
                vector,
                model.lemma_suffix.row(suffix(features.lemma_suffix)),
            )
            + dot(vector, model.form_suffix.row(suffix(features.form_suffix)))
            + word.lengths[length]
            + dot(&numeric, &model.numeric)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::weights::tests::{entry, matrix, strings};
    use crate::{Builder, Dictionary, Entry, GuessIndex as Index};

    /// Веса маленькой модели (dim = 1): все веса нули, кроме смещений суффиксов формы леммы
    /// `lemma_bias`, поэтому порядок кандидатов задают они
    fn weights(lemma_bias: &[(&str, f32)]) -> Vec<u8> {
        let suffixes: Vec<&str> = lemma_bias.iter().map(|&(suffix, _)| suffix).collect();
        let n = suffixes.len() + 1;
        let mut entries = Vec::new();
        let mut count = 0u32;
        let mut add = |name: &str, kind: u8, body: Vec<u8>| {
            entry(&mut entries, name, kind, &body);
            count += 1;
        };
        let config = [1u32, 1, 1, 4]
            .iter()
            .fold(4u32.to_le_bytes().to_vec(), |mut body, value| {
                body.extend_from_slice(&value.to_le_bytes());
                body
            });
        add("config", 3, config);
        add("chars", 0, strings(&["а", "о"]));
        add("keys", 0, strings(&["P=NOUN"]));
        add("suffixes", 0, strings(&suffixes));
        // Смещение - второй столбец строки суффикса
        let biases: Vec<f32> = std::iter::once([0.0, 0.0])
            .chain(lemma_bias.iter().map(|&(_, bias)| [0.0, bias]))
            .flatten()
            .collect();
        for (name, rows, cols, values) in [
            ("char", 4, 1, &[][..]),
            ("lstm.ih", 4, 1, &[]),
            ("lstm.hh", 4, 1, &[]),
            ("lstm.bias", 1, 4, &[]),
            ("hidden.weight", 1, 1, &[]),
            ("hidden.bias", 1, 1, &[]),
            ("key", 2, 2, &[]),
            ("lemma_suffix", n, 2, &biases[..]),
            ("form_suffix", n, 2, &[]),
            ("length", Index::MAX_ENDING, 2, &[]),
            ("numeric", 1, NUMERIC, &[]),
        ] {
            add(name, 1, matrix(rows, cols, values));
        }
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&FORMAT.to_le_bytes());
        data.extend_from_slice(&count.to_le_bytes());
        data.extend_from_slice(&entries);
        data
    }

    /// Словарь с двумя таблицами существительных: «кот» (лемма без суффикса, родительный на «а»)
    /// и «мама» (лемма на «а»); у окончания «а» в списке обе формы, у первой больше словоформ
    fn builder() -> Builder {
        let mut builder = Builder::new(
            vec!["NOUN,nomn".into(), "NOUN,gent".into()],
            vec![String::new()],
            vec![String::new(), "а".into(), "ы".into()],
        );
        let nominative = builder.add_ud_tag("NOUN", "Case=Nom", &[]).unwrap();
        let genitive = builder.add_ud_tag("NOUN", "Case=Gen", &[]).unwrap();
        let masculine = builder.add_paradigm(&[(0, 0, 0), (1, 1, 0)]).unwrap();
        let feminine = builder.add_paradigm(&[(1, 0, 0), (2, 1, 0)]).unwrap();
        let cells = [(0, nominative), (0, genitive)];
        let kot = builder.add_table(masculine, &cells).unwrap();
        let mama = builder.add_table(feminine, &cells).unwrap();
        let at = |table, form| Entry { table, form };
        builder.add_word("кота", &[at(kot, 1)]).unwrap();
        builder.add_word("мама", &[at(mama, 0)]).unwrap();
        builder
            .add_guesses(Index::Common, "а", &[(at(kot, 1), 5), (at(mama, 0), 2)])
            .unwrap();
        builder
    }

    fn lemmas(dictionary: &Dictionary, word: &str) -> Vec<String> {
        dictionary
            .analyze(word)
            .into_iter()
            .map(|analysis| analysis.lemma)
            .collect()
    }

    #[test]
    fn chooses_by_model() {
        // Без модели - по числу словоформ самого длинного окончания
        let dictionary = Dictionary::from_bytes(&builder().build().unwrap()).unwrap();
        assert_eq!(lemmas(&dictionary, "зума"), ["зум", "зума"]);
        // Модель предпочитает лемму на «а»; вероятности - softmax оценок
        let mut builder = builder();
        builder
            .paradigm_model(weights(&[("а", 3.0), ("", 0.0)]))
            .unwrap();
        let dictionary = Dictionary::from_bytes(&builder.build().unwrap()).unwrap();
        dictionary.verify().unwrap();
        let analyses = dictionary.analyze("зума");
        assert_eq!(lemmas(&dictionary, "зума"), ["зума", "зум"]);
        let expected = 3.0f64.exp() / (3.0f64.exp() + 1.0);
        assert!((analyses[0].prob - expected).abs() < 1e-6, "{analyses:?}");
        // Словарные слова модель не трогает
        assert_eq!(lemmas(&dictionary, "кота"), ["кот"]);
    }

    #[test]
    fn examples_without_own_word() {
        let dictionary = Dictionary::from_bytes(&builder().build().unwrap()).unwrap();
        let candidates = dictionary.guess_examples("кота");
        let summary: Vec<(&str, &str, &str, u32, bool)> = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.analysis.lemma.as_str(),
                    candidate.lemma_suffix.as_str(),
                    candidate.form_suffix.as_str(),
                    candidate.count,
                    candidate.gold,
                )
            })
            .collect();
        // Свое слово вычтено из списка: у его формы 4 словоформы, а не 5
        assert_eq!(
            summary,
            [("кот", "", "а", 4, true), ("кота", "а", "а", 2, false)]
        );
        assert!((candidates[0].share - 4.0 / 6.0).abs() < 1e-6);
    }

    #[test]
    fn broken_weights() {
        assert!(Model::parse(b"").is_err());
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&FORMAT.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        let err = Model::parse(&data).unwrap_err().to_string();
        assert!(
            err.contains("модель класса парадигмы: нет записи config"),
            "{err}"
        );
        // Веса контекстной модели - не веса модели класса парадигмы
        let err = Model::parse(&crate::dictionary::context::tests::weights(&[]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("нет сигнатуры"), "{err}");
        assert!(builder().paradigm_model(b"RUMPAR".to_vec()).is_err());
        assert!(Model::parse(&weights(&[("а", 1.0)])).is_ok());
    }
}
