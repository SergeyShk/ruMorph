//! Контекстная модель: выбор разбора словоформы по словам предложения. Признаки токена - запись,
//! слово, окончания и теги его разборов, BiLSTM над предложением, оценка разбора - скалярное
//! произведение с вектором токена. Веса - секция словаря, файл `scripts/train_context.py`:
//! сигнатура, версия, записи «имя, вид, данные», числа little-endian

use std::collections::HashMap;

use super::Analysis;
use super::Source;
use super::ud::UdTags;
use super::weights::{self, Entries, Lstm, MAX_DIM, Matrix, Tiled, Value, add, bilstm, dot};
use crate::Error;

const MAGIC: &[u8; 8] = b"RUMCTX\0\0";
const FORMAT: u32 = 1;
/// Строк в таблице записей токена (`shape`)
const SHAPES: usize = 64;
/// Источники разбора по порядку строк таблицы `source`
const SOURCES: usize = 3;
/// Веса контекстной модели
#[derive(Debug)]
pub(crate) struct Model {
    kmax: usize,
    ranks: usize,
    suffix_max: usize,
    edges: Vec<f64>,
    words: HashMap<String, u32>,
    suffixes: HashMap<String, u32>,
    /// Номер тега разбора с 1: «P=часть речи», «F=признак=значение», «C=часть речи|признаки»
    tags: HashMap<String, u32>,
    /// Теги частей речи и признаков словаря, на котором обучена модель
    #[cfg_attr(not(feature = "build"), allow(dead_code))]
    pub inventory: Vec<String>,
    word: Matrix,
    suffix: Matrix,
    shape: Matrix,
    tag_in: Matrix,
    tag_out: Matrix,
    tag_bias: Matrix,
    source: Matrix,
    rank: Matrix,
    bucket: Matrix,
    rank_bias: Matrix,
    bucket_bias: Matrix,
    input: Tiled,
    input_bias: Vec<f32>,
    lstm: Vec<[Lstm; 2]>,
    norm_weight: Vec<f32>,
    norm_bias: Vec<f32>,
    query: Matrix,
    query_bias: Vec<f32>,
}

/// Модель - в сообщениях об ошибках файла весов
const WHAT: &str = "контекстная модель";

fn invalid(message: &str) -> Error {
    weights::invalid(WHAT, message)
}

/// Записи файла весов `scripts/train_context.py`
fn read(data: &[u8]) -> Result<Entries<'_>, Error> {
    weights::read(data, MAGIC, FORMAT, WHAT)
}

/// Источники обучения из записи `sources` файла весов - строка JSON; в весах до 0.5.0 ее нет
#[cfg(any(test, feature = "build"))]
pub(crate) fn sources(data: &[u8]) -> Result<Option<String>, Error> {
    match read(data)?.map.remove("sources") {
        None => Ok(None),
        Some(Value::Strings(strings)) if strings.len() == 1 => Ok(Some(strings[0].to_owned())),
        Some(_) => Err(invalid("sources - не одна строка")),
    }
}

impl Model {
    /// Веса из файла `scripts/train_context.py`: размеры всех матриц проверяются
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let mut entries = read(data)?;
        let Value::Numbers(config) = entries.take("config")? else {
            return Err(invalid("config - не числа"));
        };
        let &[d, layers, dw, ds, dc, dshape, kmax, ranks, suffix_max] = config.as_slice() else {
            return Err(invalid("в config не 9 чисел"));
        };
        let [d, layers, dw, ds, dc, dshape, kmax, ranks, suffix_max] =
            [d, layers, dw, ds, dc, dshape, kmax, ranks, suffix_max].map(|value| value as usize);
        if [d, dw, ds, dc, dshape, kmax, ranks, suffix_max]
            .iter()
            .any(|&value| value == 0 || value > MAX_DIM)
            || layers == 0
            || layers > 8
            || d % 2 != 0
        {
            return Err(invalid("неверные размеры в config"));
        }
        let Value::Floats(edges) = entries.take("edges")? else {
            return Err(invalid("edges - не числа"));
        };
        // Строки 0 и 1 таблиц слов и окончаний - пустая и неизвестная, строка 0 тегов - пустая
        let words = entries.index("words", 2)?;
        let suffixes = entries.index("suffixes", 2)?;
        let tags = entries.index("tags", 1)?;
        let inventory = (entries.strings("inventory")?.iter())
            .map(|text| (*text).to_owned())
            .collect();
        let hidden = d / 2;
        let din = dshape + 2 * dc + dw + suffix_max * ds;
        let mut lstm = Vec::with_capacity(layers);
        for layer in 0..layers {
            let mut cell = |direction: &str| -> Result<Lstm, Error> {
                let prefix = format!("lstm.{layer}.{direction}");
                Ok(Lstm {
                    input: Tiled::new(&entries.matrix(&format!("{prefix}.ih"), 4 * hidden, d)?),
                    hidden: Tiled::new(&entries.matrix(
                        &format!("{prefix}.hh"),
                        4 * hidden,
                        hidden,
                    )?),
                    bias: entries.vector(&format!("{prefix}.bias"), 4 * hidden)?,
                })
            };
            lstm.push([cell("forward")?, cell("backward")?]);
        }
        Ok(Model {
            kmax,
            ranks,
            suffix_max,
            word: entries.matrix("word", words.len() + 2, dw)?,
            suffix: entries.matrix("suffix", suffixes.len() + 2, ds)?,
            shape: entries.matrix("shape", SHAPES, dshape)?,
            tag_in: entries.matrix("tag_in", tags.len() + 1, dc)?,
            tag_out: entries.matrix("tag_out", tags.len() + 1, dc)?,
            tag_bias: entries.matrix("tag_bias", tags.len() + 1, 1)?,
            source: entries.matrix("source", SOURCES, dc)?,
            rank: entries.matrix("rank", ranks, dc)?,
            bucket: entries.matrix("bucket", edges.len() + 1, dc)?,
            rank_bias: entries.matrix("rank_bias", ranks, 1)?,
            bucket_bias: entries.matrix("bucket_bias", edges.len() + 1, 1)?,
            input: Tiled::new(&entries.matrix("input.weight", d, din)?),
            input_bias: entries.vector("input.bias", d)?,
            lstm,
            norm_weight: entries.vector("norm.weight", d)?,
            norm_bias: entries.vector("norm.bias", d)?,
            query: entries.matrix("query.weight", dc, d)?,
            query_bias: entries.vector("query.bias", dc)?,
            edges,
            words,
            suffixes,
            tags,
            inventory,
        })
    }

    /// Номера тегов разбора с частью речи `pos` и признаками `feats` в `numbers`; неизвестных
    /// модели нет. Имена тегов собираются в `name` без новых строк: тегов UD в словаре тысячи
    fn tag_numbers(
        &self,
        pos: &str,
        feats: &mut [(&str, &str)],
        name: &mut String,
        numbers: &mut Vec<u32>,
    ) {
        use std::fmt::Write;
        feats.sort_unstable();
        numbers.clear();
        let mut find = |name: &str| numbers.extend(self.tags.get(name).copied());
        name.clear();
        let _ = write!(name, "P={pos}");
        find(name);
        for (feat, value) in feats.iter() {
            name.clear();
            let _ = write!(name, "F={feat}={value}");
            find(name);
        }
        name.clear();
        let _ = write!(name, "C={pos}");
        for (feat, value) in feats.iter() {
            let _ = write!(name, "|{feat}={value}");
        }
        find(name);
    }
}

/// Модель и суммы строк тегов каждого тега UD словаря: вход, выход и смещение разбора
#[derive(Debug)]
pub(crate) struct Context {
    model: Model,
    tag_in: Vec<f32>,
    tag_out: Vec<f32>,
    tag_bias: Vec<f32>,
}

impl Context {
    pub fn new(model: Model, ud: &UdTags) -> Self {
        let dc = model.tag_in.cols;
        let mut context = Context {
            tag_in: vec![0.0; ud.len() * dc],
            tag_out: vec![0.0; ud.len() * dc],
            tag_bias: vec![0.0; ud.len()],
            model,
        };
        let (mut feats, mut name, mut numbers) = (Vec::new(), String::new(), Vec::new());
        for (number, tag) in ud.iter().enumerate() {
            feats.clear();
            feats.extend(tag.feats());
            context
                .model
                .tag_numbers(tag.pos(), &mut feats, &mut name, &mut numbers);
            for &tag_number in &numbers {
                let row = tag_number as usize;
                let at = number * dc..(number + 1) * dc;
                add(
                    &mut context.tag_in[at.clone()],
                    context.model.tag_in.row(row),
                );
                add(&mut context.tag_out[at], context.model.tag_out.row(row));
                context.tag_bias[number] += context.model.tag_bias.row(row)[0];
            }
        }
        context
    }

    /// Теги частей речи и признаков словаря `ud`, которых нет в словаре обучения модели
    #[cfg(feature = "build")]
    pub fn gaps(&self, ud: &UdTags) -> Vec<String> {
        let known: std::collections::HashSet<&str> =
            self.model.inventory.iter().map(String::as_str).collect();
        let mut gaps: Vec<String> = ud
            .iter()
            .flat_map(|tag| {
                std::iter::once(format!("P={}", tag.pos()))
                    .chain(tag.feats().map(|(name, value)| format!("F={name}={value}")))
            })
            .filter(|name| !known.contains(name.as_str()))
            .collect();
        gaps.sort_unstable();
        gaps.dedup();
        gaps
    }

    fn tag<'a>(&self, table: &'a [f32], ud: u16) -> &'a [f32] {
        let dc = self.model.tag_in.cols;
        let at = usize::from(ud) * dc;
        table.get(at..at + dc).unwrap_or(&[])
    }

    /// Разборы слов предложения по убыванию оценки модели; вероятность - доля в контексте.
    /// Модель видит первые `kmax` разборов слова, остальные идут после них с вероятностью 0.
    /// Слово с `starts` - начало строки: его запись - как у первого слова предложения
    pub fn rank(
        &self,
        words: &[&str],
        starts: &[bool],
        analyses: Vec<Vec<Analysis>>,
    ) -> Vec<Vec<Analysis>> {
        let model = &self.model;
        let n = words.len();
        if n == 0 {
            return analyses;
        }
        let (d, dc) = (model.input.rows, model.tag_in.cols);
        let din = model.input.cols;
        let mut x = vec![0.0f32; n * din];
        // Ранг и корзина вероятности каждого разбора, который видит модель
        let mut features: Vec<Vec<(usize, usize)>> = Vec::with_capacity(n);
        for (t, (word, list)) in words.iter().zip(&analyses).enumerate() {
            // Вход токена: запись, среднее и взвешенная по вероятностям сумма тегов разборов,
            // слово, окончания
            let row = &mut x[t * din..(t + 1) * din];
            let (shape_row, rest) = row.split_at_mut(model.shape.cols);
            let first = t == 0 || starts.get(t).copied().unwrap_or(false);
            shape_row.copy_from_slice(model.shape.row(shape(word, first)));
            let (mean, rest) = rest.split_at_mut(dc);
            let (weighted, rest) = rest.split_at_mut(dc);
            let seen = &list[..list.len().min(model.kmax)];
            let total: f64 = seen.iter().map(|analysis| analysis.prob).sum();
            for analysis in seen {
                let tag_in = self.tag(&self.tag_in, analysis.ud);
                let weight = (analysis.prob / total.max(1e-9)) as f32;
                for ((mean, weighted), value) in
                    mean.iter_mut().zip(weighted.iter_mut()).zip(tag_in)
                {
                    *mean += value;
                    *weighted += value * weight;
                }
            }
            for value in mean.iter_mut() {
                *value /= seen.len().max(1) as f32;
            }
            let normal = word.to_lowercase().replace('ё', "е");
            let (word_row, rest) = rest.split_at_mut(model.word.cols);
            let number = model.words.get(&normal).copied().unwrap_or(1);
            word_row.copy_from_slice(model.word.row(number as usize));
            let suffix_rows = rest.chunks_exact_mut(model.suffix.cols);
            for (key, out) in suffixes(&normal, model.suffix_max).zip(suffix_rows) {
                let number = model.suffixes.get(&key).copied().unwrap_or(1);
                out.copy_from_slice(model.suffix.row(number as usize));
            }
            let mut order: Vec<usize> = (0..seen.len()).collect();
            order.sort_by(|&a, &b| seen[b].prob.total_cmp(&seen[a].prob));
            let mut ranks = vec![0; seen.len()];
            for (place, &index) in order.iter().enumerate() {
                ranks[index] = place.min(model.ranks - 1);
            }
            features.push(
                seen.iter()
                    .zip(ranks)
                    .map(|(analysis, rank)| {
                        let bucket = model.edges.iter().filter(|&&edge| edge < analysis.prob);
                        (rank, bucket.count())
                    })
                    .collect(),
            );
        }

        let mut hidden = vec![0.0f32; n * d];
        model.input.apply_all(&x, &model.input_bias, &mut hidden);
        for value in &mut hidden {
            *value = value.max(0.0);
        }
        for layer in &model.lstm {
            hidden = bilstm(layer, &hidden, n, d);
        }

        let mut query = vec![0.0f32; dc];
        let mut normed = vec![0.0f32; d];
        let mut result = Vec::with_capacity(n);
        for (t, (list, features)) in analyses.into_iter().zip(&features).enumerate() {
            let h = &hidden[t * d..(t + 1) * d];
            let mean = h.iter().sum::<f32>() / d as f32;
            let variance = h.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / d as f32;
            let scale = 1.0 / (variance + 1e-5).sqrt();
            for (i, value) in normed.iter_mut().enumerate() {
                *value = (h[i] - mean) * scale * model.norm_weight[i] + model.norm_bias[i];
            }
            model.query.apply(&normed, &mut query);
            add(&mut query, &model.query_bias);
            let scores: Vec<f32> = list
                .iter()
                .zip(features)
                .map(|(analysis, &(rank, bucket))| {
                    let source = match analysis.source {
                        Source::Dictionary => 0,
                        Source::Rule => 1,
                        Source::Guess => 2,
                    };
                    dot(&query, self.tag(&self.tag_out, analysis.ud))
                        + dot(&query, model.source.row(source))
                        + dot(&query, model.rank.row(rank))
                        + dot(&query, model.bucket.row(bucket))
                        + self
                            .tag_bias
                            .get(usize::from(analysis.ud))
                            .copied()
                            .unwrap_or(0.0)
                        + model.rank_bias.row(rank)[0]
                        + model.bucket_bias.row(bucket)[0]
                })
                .collect();
            result.push(reorder(list, &scores));
        }
        result
    }
}

/// Разборы по убыванию оценок `scores` первых разборов с вероятностями softmax, остальные - после
/// них с вероятностью 0; при равных оценках порядок словаря
fn reorder(list: Vec<Analysis>, scores: &[f32]) -> Vec<Analysis> {
    let top = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f64> = scores.iter().map(|&s| f64::from(s - top).exp()).collect();
    let total: f64 = exps.iter().sum();
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    let mut slots: Vec<Option<Analysis>> = list.into_iter().map(Some).collect();
    let mut result = Vec::with_capacity(slots.len());
    for index in order {
        if let Some(mut analysis) = slots[index].take() {
            analysis.prob = exps[index] / total;
            result.push(analysis);
        }
    }
    for mut analysis in slots.into_iter().flatten() {
        analysis.prob = 0.0;
        result.push(analysis);
    }
    result
}

/// Окончания слова длиной 1..=`max` с началом «^»: ключ - длина и окончание
fn suffixes(word: &str, max: usize) -> impl Iterator<Item = String> {
    let marked: Vec<char> = std::iter::once('^').chain(word.chars()).collect();
    (1..=max).map(move |n| {
        let tail: String = marked[marked.len().saturating_sub(n)..].iter().collect();
        format!("{n}{tail}")
    })
}

/// Запись токена: знаки, число, буквы в нижнем, верхнем регистре, с заглавной или смешанно,
/// латиница, начало предложения и дефис (`shape` в `scripts/train_context.py`)
fn shape(token: &str, first: bool) -> usize {
    let digits = token.chars().filter(char::is_ascii_digit).count();
    let letters: Vec<char> = token.chars().filter(|ch| ch.is_alphabetic()).collect();
    let kind = if digits == 0 && letters.is_empty() {
        0
    } else if digits > 0 {
        if digits == token.chars().count() {
            1
        } else {
            2
        }
    } else {
        let kind = if letters.iter().all(|ch| !ch.is_uppercase()) {
            3
        } else if letters.iter().all(|ch| !ch.is_lowercase()) && letters.len() > 1 {
            4
        } else if letters[0].is_uppercase() && letters[1..].iter().all(|ch| !ch.is_uppercase()) {
            5
        } else {
            6
        };
        let latin = letters.iter().any(|ch| {
            ch.to_lowercase()
                .next()
                .is_some_and(|ch| ch.is_ascii_lowercase())
        });
        kind + 4 * usize::from(latin)
    };
    kind * 4 + 2 * usize::from(first) + usize::from(token.contains('-'))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::Dictionary;
    use crate::dictionary::tests::{Forms, dictionary_bytes_of, dictionary_bytes_with};
    use crate::dictionary::weights::tests::{entry, matrix, strings};

    /// Веса маленькой модели (d = 2): все веса нули, кроме смещений тегов `bias`, поэтому
    /// порядок разборов задают теги
    pub(crate) fn weights(bias: &[(&str, f32)]) -> Vec<u8> {
        let tags: Vec<&str> = bias.iter().map(|&(tag, _)| tag).collect();
        let n = tags.len() + 1;
        let mut entries = Vec::new();
        let mut count = 0u32;
        let mut add = |name: &str, kind: u8, body: Vec<u8>| {
            entry(&mut entries, name, kind, &body);
            count += 1;
        };
        let config: Vec<u8> = [2u32, 1, 1, 1, 1, 1, 32, 8, 4].iter().fold(
            9u32.to_le_bytes().to_vec(),
            |mut body, value| {
                body.extend_from_slice(&value.to_le_bytes());
                body
            },
        );
        add("config", 3, config);
        let edges = [
            1e-4, 1e-3, 0.01, 0.03, 0.1, 0.2, 0.35, 0.5, 0.65, 0.8, 0.9, 0.97, 0.995f64,
        ];
        let mut body = (edges.len() as u32).to_le_bytes().to_vec();
        edges
            .iter()
            .for_each(|edge| body.extend_from_slice(&edge.to_le_bytes()));
        add("edges", 4, body);
        add("words", 0, strings(&["мама"]));
        add("suffixes", 0, strings(&[]));
        add("tags", 0, strings(&tags));
        add("inventory", 0, strings(&["P=NOUN"]));
        let biases: Vec<f32> = std::iter::once(0.0)
            .chain(bias.iter().map(|&(_, b)| b))
            .collect();
        for (name, rows, cols, values) in [
            ("word", 3, 1, &[][..]),
            ("suffix", 2, 1, &[]),
            ("shape", SHAPES, 1, &[]),
            ("tag_in", n, 1, &[]),
            ("tag_out", n, 1, &[]),
            ("tag_bias", n, 1, &biases[..]),
            ("source", SOURCES, 1, &[]),
            ("rank", 8, 1, &[]),
            ("bucket", 14, 1, &[]),
            ("rank_bias", 8, 1, &[]),
            ("bucket_bias", 14, 1, &[]),
            ("input.weight", 2, 8, &[]),
            ("input.bias", 1, 2, &[]),
            ("lstm.0.forward.ih", 4, 2, &[]),
            ("lstm.0.forward.hh", 4, 1, &[]),
            ("lstm.0.forward.bias", 1, 4, &[]),
            ("lstm.0.backward.ih", 4, 2, &[]),
            ("lstm.0.backward.hh", 4, 1, &[]),
            ("lstm.0.backward.bias", 1, 4, &[]),
            ("norm.weight", 1, 2, &[]),
            ("norm.bias", 1, 2, &[]),
            ("query.weight", 1, 2, &[]),
            ("query.bias", 1, 1, &[]),
        ] {
            add(name, 1, matrix(rows, cols, values));
        }
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&FORMAT.to_le_bytes());
        data.extend_from_slice(&count.to_le_bytes());
        data.extend_from_slice(&entries);
        data
    }

    fn table() -> Vec<(&'static str, &'static str, Forms)> {
        let noun = |case: &str| format!("Case={case}|Gender=Masc|Number=Sing");
        vec![
            ("NOUN", "стол", vec![("", noun("Nom")), ("", noun("Acc"))]),
            ("VERB", "виж", vec![("у", "Person=1".into())]),
        ]
    }

    #[test]
    fn training_sources() {
        let mut data = weights(&[("P=NOUN", 0.0)]);
        assert_eq!(sources(&data).unwrap(), None);
        let count = u32::from_le_bytes(data[12..16].try_into().unwrap());
        data[12..16].copy_from_slice(&(count + 1).to_le_bytes());
        entry(
            &mut data,
            "sources",
            0,
            &strings(&[r#"{"release":"r2.18"}"#]),
        );
        assert_eq!(
            sources(&data).unwrap().as_deref(),
            Some(r#"{"release":"r2.18"}"#)
        );
        assert!(Model::parse(&data).is_ok());
    }

    #[test]
    fn ranks_by_context_model() {
        let data = dictionary_bytes_with(&table(), &[], Some(weights(&[("F=Case=Acc", 2.0)])));
        let dictionary = Dictionary::from_bytes(&data).unwrap();
        let sentence = dictionary.analyze_sentence(&["вижу", "стол"]).unwrap();
        let case = |analysis: &Analysis| dictionary.ud_tag(analysis.ud).unwrap().feat("Case");
        let table = &sentence[1];
        assert_eq!(table.len(), 2);
        assert_eq!(case(&table[0]), Some("Acc"));
        let expected = 1.0 / (1.0 + (-2.0f64).exp());
        assert!((table[0].prob - expected).abs() < 1e-6, "{}", table[0].prob);
        assert!((table[0].prob + table[1].prob - 1.0).abs() < 1e-9);
        assert_eq!(sentence[0][0].prob, 1.0);
        // Пустое предложение, слово без соседей - как `analyze`, копия словаря - та же модель
        assert!(dictionary.analyze_sentence::<&str>(&[]).unwrap().is_empty());
        let alone = dictionary.analyze_sentence(&["стол"]).unwrap();
        assert_eq!(alone, vec![dictionary.analyze("стол")]);
        let copy = dictionary.clone();
        assert_eq!(
            copy.analyze_sentence(&["вижу", "стол"]).unwrap()[1][0].ud,
            table[0].ud
        );
    }

    #[test]
    fn long_sentence_in_chunks() {
        let data = dictionary_bytes_with(&table(), &[], Some(weights(&[("F=Case=Acc", 2.0)])));
        let dictionary = Dictionary::from_bytes(&data).unwrap();
        let words: Vec<&str> = ["вижу", "стол"]
            .into_iter()
            .cycle()
            .take(crate::SENTENCE_CHUNK + 3)
            .collect();
        let (head, tail) = words.split_at(crate::SENTENCE_CHUNK);
        let mut chunks = dictionary.analyze_sentence(head).unwrap();
        chunks.extend(dictionary.analyze_sentence(tail).unwrap());
        assert_eq!(dictionary.analyze_sentence(&words).unwrap(), chunks);
    }

    #[test]
    fn verify_reads_the_model() {
        // Модель, испорченная под верной суммой: загрузка ее принимает, `verify` - нет
        let data = dictionary_bytes_with(&table(), &[], Some(weights(&[("F=Case=Acc", 2.0)])));
        assert!(Dictionary::from_bytes(&data).unwrap().verify().is_ok());
        let mut damaged = data.clone();
        let at = (damaged.windows(MAGIC.len()))
            .position(|window| window == MAGIC)
            .unwrap();
        damaged[at] = b'X';
        let dictionary =
            Dictionary::from_bytes(&crate::format::tests::with_checksum(damaged)).unwrap();
        assert!(matches!(dictionary.verify(), Err(Error::Context(_))));
        // Словарь без модели проверяется без нее
        let plain = Dictionary::from_bytes(&dictionary_bytes_of(&table(), &[])).unwrap();
        assert!(plain.verify().is_ok());
    }

    #[test]
    fn dictionary_without_model() {
        let dictionary = Dictionary::from_bytes(&dictionary_bytes_of(&table(), &[])).unwrap();
        let err = dictionary.analyze_sentence(&["стол"]).unwrap_err();
        assert!(matches!(err, Error::Context(_)), "{err}");
        assert!(err.to_string().contains("нет контекстной модели"));
    }

    #[test]
    fn token_shapes() {
        assert_eq!(shape(",", false), 0);
        assert_eq!(shape("1990", false), 4);
        assert_eq!(shape("1990-х", false), 9);
        assert_eq!(shape("мама", false), 12);
        assert_eq!(shape("МГУ", false), 16);
        assert_eq!(shape("Мама", true), 22);
        assert_eq!(shape("РусГидро", false), 24);
        assert_eq!(shape("iPhone", false), 40);
        assert_eq!(shape("Москва-река", false), 21);
    }

    #[test]
    fn word_suffixes() {
        let keys: Vec<String> = suffixes("кот", 4).collect();
        assert_eq!(keys, ["1т", "2от", "3кот", "4^кот"]);
        let keys: Vec<String> = suffixes("я", 4).collect();
        assert_eq!(keys, ["1я", "2^я", "3^я", "4^я"]);
    }

    #[test]
    fn repeated_strings() {
        // Повтор тега: номер строки второго вышел бы за таблицу тегов
        let data = weights(&[("P=NOUN", 1.0), ("P=NOUN", 2.0)]);
        let err = Model::parse(&data).unwrap_err().to_string();
        assert!(err.contains("в tags повторяются строки"), "{err}");
    }

    #[test]
    fn broken_weights() {
        assert!(Model::parse(b"").is_err());
        assert!(Model::parse(b"RUMCTX\0\0\x02\0\0\0\0\0\0\0").is_err());
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&FORMAT.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        let err = Model::parse(&data).unwrap_err().to_string();
        assert!(err.contains("нет записи config"), "{err}");
    }
}
