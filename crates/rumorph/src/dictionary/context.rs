//! Контекстная модель: выбор разбора словоформы по словам предложения. Признаки токена - запись,
//! слово, окончания и теги его разборов, BiLSTM над предложением, оценка разбора - скалярное
//! произведение с вектором токена. Веса - секция словаря, файл `scripts/train_context.py`:
//! сигнатура, версия, записи «имя, вид, данные», числа little-endian

use std::collections::HashMap;

use super::Analysis;
use super::Source;
use super::ud::UdTags;
use crate::Error;

const MAGIC: &[u8; 8] = b"RUMCTX\0\0";
const FORMAT: u32 = 1;
/// Строк в таблице записей токена (`shape`)
const SHAPES: usize = 64;
/// Источники разбора по порядку строк таблицы `source`
const SOURCES: usize = 3;
/// Наибольшие размеры из файла: испорченный файл не задает гигабайты
const MAX_DIM: usize = 4096;
const MAX_ROWS: usize = 1 << 22;

/// Матрица f32 по строкам
#[derive(Debug)]
struct Matrix {
    rows: usize,
    cols: usize,
    data: Vec<f32>,
}

impl Matrix {
    fn row(&self, row: usize) -> &[f32] {
        &self.data[row * self.cols..(row + 1) * self.cols]
    }

    /// `out = self · input` (`out` - строки, `input` - столбцы)
    fn apply(&self, input: &[f32], out: &mut [f32]) {
        for (value, row) in out.iter_mut().zip(self.data.chunks_exact(self.cols)) {
            *value = dot(row, input);
        }
    }
}

/// Число строк плитки `Tiled`
const TILE: usize = 16;
/// Число векторов, которые плитка умножает за раз
const BATCH: usize = 4;

/// Матрица по плиткам из `TILE` строк для умножения на много векторов: в плитке столбцы подряд,
/// в столбце - числа `TILE` строк. Суммы плитки на `BATCH` векторов держатся в регистрах,
/// а каждое число матрицы читается один раз на `BATCH` векторов
#[derive(Debug)]
struct Tiled {
    rows: usize,
    cols: usize,
    data: Vec<f32>,
}

impl Tiled {
    fn new(matrix: &Matrix) -> Self {
        let tiles = matrix.rows.div_ceil(TILE);
        let mut data = vec![0.0; tiles * matrix.cols * TILE];
        for row in 0..matrix.rows {
            let (tile, offset) = (row / TILE, row % TILE);
            for (col, &value) in matrix.row(row).iter().enumerate() {
                data[(tile * matrix.cols + col) * TILE + offset] = value;
            }
        }
        Tiled {
            rows: matrix.rows,
            cols: matrix.cols,
            data,
        }
    }

    /// `out += self · input` для одного вектора
    fn apply_add(&self, input: &[f32], out: &mut [f32]) {
        for (tile, weights) in self.data.chunks_exact(self.cols * TILE).enumerate() {
            let mut sums = [0.0f32; TILE];
            for (column, &value) in weights.as_chunks::<TILE>().0.iter().zip(input) {
                for j in 0..TILE {
                    sums[j] += value * column[j];
                }
            }
            let start = tile * TILE;
            let end = (start + TILE).min(self.rows);
            for (out, sum) in out[start..end].iter_mut().zip(sums) {
                *out += sum;
            }
        }
    }

    /// `out[t] = self · inputs[t] + bias` для всех векторов `inputs` подряд
    fn apply_all(&self, inputs: &[f32], bias: &[f32], out: &mut [f32]) {
        let count = inputs.len() / self.cols;
        let mut padded = vec![[0.0f32; BATCH]; self.cols];
        for first in (0..count).step_by(BATCH) {
            let batch = (count - first).min(BATCH);
            // Векторы пакета по столбцам: столбец - числа `BATCH` векторов
            for (col, values) in padded.iter_mut().enumerate() {
                for (b, value) in values.iter_mut().enumerate() {
                    *value = if b < batch {
                        inputs[(first + b) * self.cols + col]
                    } else {
                        0.0
                    };
                }
            }
            for (tile, weights) in self.data.chunks_exact(self.cols * TILE).enumerate() {
                let mut sums = [[0.0f32; TILE]; BATCH];
                for (column, values) in weights.as_chunks::<TILE>().0.iter().zip(&padded) {
                    for b in 0..BATCH {
                        for j in 0..TILE {
                            sums[b][j] += values[b] * column[j];
                        }
                    }
                }
                for (b, sums) in sums.iter().enumerate().take(batch) {
                    for (j, sum) in sums.iter().enumerate() {
                        let row = tile * TILE + j;
                        if row < self.rows {
                            out[(first + b) * self.rows + row] = sum + bias[row];
                        }
                    }
                }
            }
        }
    }
}

/// Скалярное произведение блоками по 8: так цикл векторизуется
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut sums = [0.0f32; 8];
    let (left, right) = (a.as_chunks::<8>(), b.as_chunks::<8>());
    for (x, y) in left.0.iter().zip(right.0) {
        for i in 0..8 {
            sums[i] += x[i] * y[i];
        }
    }
    let tail: f32 = left.1.iter().zip(right.1).map(|(x, y)| x * y).sum();
    sums.iter().sum::<f32>() + tail
}

fn add(out: &mut [f32], values: &[f32]) {
    for (out, value) in out.iter_mut().zip(values) {
        *out += value;
    }
}

/// e^x с относительной ошибкой около 2e-7: без libm и ветвлений, поэтому цикл по массиву
/// векторизуется. Округление - сложением 1,5·2²³ (SSE2 не округляет векторно)
#[inline(always)]
fn exp(x: f32) -> f32 {
    const ROUND: f32 = 12_582_912.0;
    let x = x.clamp(-87.0, 88.0);
    let n = (x * std::f32::consts::LOG2_E + ROUND) - ROUND;
    // ln 2 двумя частями: остаток точен и при больших n
    let r = x - n * 0.693_145_75 - n * 1.428_606_8e-6;
    let p = 1.0
        + r * (1.0
            + r * (0.5 + r * (1.0 / 6.0 + r * (1.0 / 24.0 + r * (1.0 / 120.0 + r / 720.0)))));
    p * f32::from_bits(((n as i32 + 127) as u32) << 23)
}

#[inline(always)]
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + exp(-x))
}

#[inline(always)]
fn tanh(x: f32) -> f32 {
    2.0 / (1.0 + exp(-2.0 * x)) - 1.0
}

/// Ячейка LSTM одного направления: матрицы ворот в порядке PyTorch (i, f, g, o), смещения сложены
#[derive(Debug)]
struct Lstm {
    input: Tiled,
    hidden: Tiled,
    bias: Vec<f32>,
}

/// Значение записи файла весов
enum Value<'a> {
    Strings(Vec<&'a str>),
    Matrix(Matrix),
    Numbers(Vec<u32>),
    Floats(Vec<f64>),
}

/// Чтение файла весов с проверкой границ
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let end = self
            .at
            .checked_add(count)
            .filter(|&end| end <= self.data.len());
        let end = end.ok_or_else(|| invalid("файл весов обрезан"))?;
        let bytes = &self.data[self.at..end];
        self.at = end;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn count(&mut self, limit: usize) -> Result<usize, Error> {
        let count = self.u32()? as usize;
        if count > limit {
            return Err(invalid("размер записи больше допустимого"));
        }
        Ok(count)
    }

    fn f32s(&mut self, count: usize) -> Result<Vec<f32>, Error> {
        let bytes = self.take(count.checked_mul(4).ok_or_else(|| invalid("размер"))?)?;
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&chunk| f32::from_le_bytes(chunk))
            .collect())
    }

    fn value(&mut self) -> Result<Value<'a>, Error> {
        let kind = self.take(1)?[0];
        match kind {
            0 => {
                let count = self.count(MAX_ROWS)?;
                let mut strings = Vec::with_capacity(count.min(1 << 16));
                for _ in 0..count {
                    let len = self.count(1 << 16)?;
                    let text = std::str::from_utf8(self.take(len)?)
                        .map_err(|_| invalid("строка не в UTF-8"))?;
                    strings.push(text);
                }
                Ok(Value::Strings(strings))
            }
            1 | 2 => {
                let (rows, cols) = (self.count(MAX_ROWS)?, self.count(MAX_DIM)?);
                let size = rows * cols;
                let data = if kind == 1 {
                    self.f32s(size)?
                } else {
                    let scales = self.f32s(rows)?;
                    let values = self.take(size)?;
                    values
                        .chunks_exact(cols.max(1))
                        .zip(&scales)
                        .flat_map(|(row, &scale)| {
                            row.iter().map(move |&q| f32::from(q as i8) * scale)
                        })
                        .collect()
                };
                Ok(Value::Matrix(Matrix { rows, cols, data }))
            }
            3 => {
                let count = self.count(64)?;
                (0..count)
                    .map(|_| self.u32())
                    .collect::<Result<_, _>>()
                    .map(Value::Numbers)
            }
            4 => {
                let count = self.count(64)?;
                let bytes = self.take(count * 8)?;
                Ok(Value::Floats(
                    bytes
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|&chunk| f64::from_le_bytes(chunk))
                        .collect(),
                ))
            }
            _ => Err(invalid(&format!("неизвестный вид записи {kind}"))),
        }
    }
}

fn invalid(message: &str) -> Error {
    Error::Format(format!("контекстная модель: {message}"))
}

/// Записи файла весов по именам
struct Entries<'a>(HashMap<&'a str, Value<'a>>);

impl<'a> Entries<'a> {
    fn take(&mut self, name: &str) -> Result<Value<'a>, Error> {
        self.0
            .remove(name)
            .ok_or_else(|| invalid(&format!("нет записи {name}")))
    }

    fn matrix(&mut self, name: &str, rows: usize, cols: usize) -> Result<Matrix, Error> {
        match self.take(name)? {
            Value::Matrix(matrix) if matrix.rows == rows && matrix.cols == cols => Ok(matrix),
            Value::Matrix(matrix) => Err(invalid(&format!(
                "{name}: размер {}×{}, нужен {rows}×{cols}",
                matrix.rows, matrix.cols
            ))),
            _ => Err(invalid(&format!("{name} - не матрица"))),
        }
    }

    fn vector(&mut self, name: &str, size: usize) -> Result<Vec<f32>, Error> {
        Ok(self.matrix(name, 1, size)?.data)
    }

    fn strings(&mut self, name: &str) -> Result<Vec<&'a str>, Error> {
        match self.take(name)? {
            Value::Strings(strings) => Ok(strings),
            _ => Err(invalid(&format!("{name} - не строки"))),
        }
    }

    /// Номера строк таблицы по строкам записи `name` с номера `first`; повтор строки - ошибка:
    /// иначе номер строки вышел бы за таблицу
    fn index(&mut self, name: &str, first: u32) -> Result<HashMap<String, u32>, Error> {
        let strings = self.strings(name)?;
        let index: HashMap<String, u32> = (strings.iter().enumerate())
            .map(|(number, text)| ((*text).to_owned(), number as u32 + first))
            .collect();
        if index.len() != strings.len() {
            return Err(invalid(&format!("в {name} повторяются строки")));
        }
        Ok(index)
    }
}

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

impl Model {
    /// Веса из файла `scripts/train_context.py`: размеры всех матриц проверяются
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        if data.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
            return Err(invalid("нет сигнатуры файла весов"));
        }
        let mut reader = Reader {
            data,
            at: MAGIC.len(),
        };
        let format = reader.u32()?;
        if format != FORMAT {
            return Err(invalid(&format!(
                "версия файла весов {format}, поддерживается {FORMAT}"
            )));
        }
        let count = reader.count(256)?;
        let mut entries = HashMap::new();
        for _ in 0..count {
            let len = usize::from(u16::from_le_bytes(
                reader.take(2)?.try_into().map_err(|_| invalid("имя"))?,
            ));
            let name = std::str::from_utf8(reader.take(len)?)
                .map_err(|_| invalid("имя записи не в UTF-8"))?;
            entries.insert(name, reader.value()?);
        }
        if reader.at != data.len() {
            return Err(invalid("лишние данные после записей"));
        }
        let mut entries = Entries(entries);
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

    /// Номера тегов разбора с частью речи `pos` и признаками `feats`; неизвестных модели нет
    fn tag_numbers(&self, pos: &str, feats: &[(&str, &str)]) -> Vec<u32> {
        let mut feats = feats.to_vec();
        feats.sort_unstable();
        let mut names = vec![format!("P={pos}")];
        names.extend(
            feats
                .iter()
                .map(|(name, value)| format!("F={name}={value}")),
        );
        let full: Vec<String> = std::iter::once(pos.to_owned())
            .chain(feats.iter().map(|(name, value)| format!("{name}={value}")))
            .collect();
        names.push(format!("C={}", full.join("|")));
        names
            .iter()
            .filter_map(|name| self.tags.get(name).copied())
            .collect()
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
        for (number, tag) in ud.iter().enumerate() {
            let feats: Vec<(&str, &str)> = tag.feats().collect();
            for tag_number in context.model.tag_numbers(tag.pos(), &feats) {
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
    /// Модель видит первые `kmax` разборов слова, остальные идут после них с вероятностью 0
    pub fn rank(&self, words: &[&str], analyses: Vec<Vec<Analysis>>) -> Vec<Vec<Analysis>> {
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
            shape_row.copy_from_slice(model.shape.row(shape(word, t == 0)));
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

/// Слой BiLSTM: выход токена - состояния прямого и обратного направления подряд. Направления
/// идут в одном цикле: их цепочки независимы, и процессор считает их одновременно
fn bilstm(layer: &[Lstm; 2], input: &[f32], n: usize, d: usize) -> Vec<f32> {
    let hidden = d / 2;
    let mut output = vec![0.0f32; n * d];
    // Вклад входа во все ворота считается сразу для всех токенов
    let gates_input = layer.each_ref().map(|cell| {
        let mut gates = vec![0.0f32; n * 4 * hidden];
        cell.input.apply_all(input, &cell.bias, &mut gates);
        gates
    });
    let mut states = [(); 2].map(|()| (vec![0.0f32; hidden], vec![0.0f32; hidden]));
    let mut gates = [(); 2].map(|()| vec![0.0f32; 4 * hidden]);
    for step in 0..n {
        for direction in 0..2 {
            let t = if direction == 0 { step } else { n - 1 - step };
            let (h, c) = &mut states[direction];
            let gates = &mut gates[direction];
            gates.copy_from_slice(&gates_input[direction][t * 4 * hidden..(t + 1) * 4 * hidden]);
            layer[direction].hidden.apply_add(h, gates);
            // Ворота i, f, g, o; каждый цикл - по массиву, без ветвлений
            let (sigmoids, rest) = gates.split_at_mut(2 * hidden);
            let (candidates, outputs) = rest.split_at_mut(hidden);
            sigmoids
                .iter_mut()
                .for_each(|value| *value = sigmoid(*value));
            candidates
                .iter_mut()
                .for_each(|value| *value = tanh(*value));
            outputs
                .iter_mut()
                .for_each(|value| *value = sigmoid(*value));
            let (inputs, forgets) = sigmoids.split_at(hidden);
            for i in 0..hidden {
                c[i] = forgets[i] * c[i] + inputs[i] * candidates[i];
            }
            for i in 0..hidden {
                h[i] = outputs[i] * tanh(c[i]);
            }
            output[t * d + direction * hidden..t * d + (direction + 1) * hidden].copy_from_slice(h);
        }
    }
    output
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

    fn entry(out: &mut Vec<u8>, name: &str, kind: u8, body: &[u8]) {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(kind);
        out.extend_from_slice(body);
    }

    fn matrix(rows: usize, cols: usize, values: &[f32]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(rows as u32).to_le_bytes());
        body.extend_from_slice(&(cols as u32).to_le_bytes());
        for row in 0..rows {
            for col in 0..cols {
                let value = values.get(row * cols + col).copied().unwrap_or(0.0);
                body.extend_from_slice(&value.to_le_bytes());
            }
        }
        body
    }

    fn strings(items: &[&str]) -> Vec<u8> {
        let mut body = (items.len() as u32).to_le_bytes().to_vec();
        for item in items {
            body.extend_from_slice(&(item.len() as u32).to_le_bytes());
            body.extend_from_slice(item.as_bytes());
        }
        body
    }

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
    fn dictionary_without_model() {
        let dictionary = Dictionary::from_bytes(&dictionary_bytes_of(&table(), &[])).unwrap();
        let err = dictionary.analyze_sentence(&["стол"]).unwrap_err();
        assert!(matches!(err, Error::Context(_)), "{err}");
        assert!(err.to_string().contains("нет контекстной модели"));
    }

    #[test]
    fn lstm_recurrence() {
        // Скрытое состояние 1: ворота постоянны (только смещения), состояние растет по шагам
        let bias = vec![0.5f32, -0.3, 0.8, 0.2];
        let zeros = |cols: usize| {
            Tiled::new(&Matrix {
                rows: 4,
                cols,
                data: vec![0.0; 4 * cols],
            })
        };
        let cell = || Lstm {
            input: zeros(2),
            hidden: zeros(1),
            bias: bias.clone(),
        };
        let output = bilstm(&[cell(), cell()], &[0.0; 6], 3, 2);
        let (i, f, g, o) = (sigmoid(0.5), sigmoid(-0.3), 0.8f32.tanh(), sigmoid(0.2));
        let mut states = Vec::new();
        let mut c = 0.0;
        for _ in 0..3 {
            c = f * c + i * g;
            states.push(o * c.tanh());
        }
        for t in 0..3 {
            assert!((output[t * 2] - states[t]).abs() < 1e-6);
            assert!((output[t * 2 + 1] - states[2 - t]).abs() < 1e-6);
        }
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
    fn dot_products() {
        let a: Vec<f32> = (0..19).map(|i| i as f32).collect();
        let b = vec![2.0f32; 19];
        assert_eq!(dot(&a, &b), 342.0);
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
