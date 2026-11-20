//! Веса моделей словаря: файл записей «имя, вид, данные» с сигнатурой модели и версией, числа
//! little-endian; матрицы и ядра BiLSTM для прямого прохода

use std::collections::HashMap;

use crate::Error;

/// Наибольшие размеры из файла: испорченный файл не задает гигабайты
pub(super) const MAX_DIM: usize = 4096;
const MAX_ROWS: usize = 1 << 22;

/// Матрица f32 по строкам
#[derive(Debug)]
pub(super) struct Matrix {
    pub(super) rows: usize,
    pub(super) cols: usize,
    pub(super) data: Vec<f32>,
}

impl Matrix {
    pub(super) fn row(&self, row: usize) -> &[f32] {
        &self.data[row * self.cols..(row + 1) * self.cols]
    }

    /// `out = self · input` (`out` - строки, `input` - столбцы)
    pub(super) fn apply(&self, input: &[f32], out: &mut [f32]) {
        for (value, row) in out.iter_mut().zip(self.data.chunks_exact(self.cols)) {
            *value = dot(row, input);
        }
    }
}

/// Число строк плитки `Tiled`
pub(super) const TILE: usize = 16;
/// Число векторов, которые плитка умножает за раз: `Tiled::apply_all` держит столько сумм
pub(super) const BATCH: usize = 4;
const _: () = assert!(BATCH == 4);

/// Матрица по плиткам из `TILE` строк для умножения на много векторов: в плитке столбцы подряд,
/// в столбце - числа `TILE` строк. Суммы плитки на `BATCH` векторов держатся в регистрах,
/// а каждое число матрицы читается один раз на `BATCH` векторов
#[derive(Debug)]
pub(super) struct Tiled {
    pub(super) rows: usize,
    pub(super) cols: usize,
    pub(super) columns: Vec<[f32; TILE]>,
}

impl Tiled {
    pub(super) fn new(matrix: &Matrix) -> Self {
        let tiles = matrix.rows.div_ceil(TILE);
        let mut columns = vec![[0.0; TILE]; tiles * matrix.cols];
        for row in 0..matrix.rows {
            let (tile, offset) = (row / TILE, row % TILE);
            for (col, &value) in matrix.row(row).iter().enumerate() {
                columns[tile * matrix.cols + col][offset] = value;
            }
        }
        Tiled {
            rows: matrix.rows,
            cols: matrix.cols,
            columns,
        }
    }

    /// `out += self · input` для одного вектора
    pub(super) fn apply_add(&self, input: &[f32], out: &mut [f32]) {
        #[cfg(target_arch = "x86_64")]
        if fused() {
            // SAFETY: AVX2 и FMA у процессора есть - проверено `fused`
            return unsafe { self.apply_add_fused(input, out) };
        }
        self.apply_add_with::<false>(input, out);
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2,fma")]
    fn apply_add_fused(&self, input: &[f32], out: &mut [f32]) {
        self.apply_add_with::<true>(input, out);
    }

    #[inline(always)]
    pub(super) fn apply_add_with<const FUSED: bool>(&self, input: &[f32], out: &mut [f32]) {
        for (tile, columns) in self.columns.chunks_exact(self.cols).enumerate() {
            let mut sums = [0.0f32; TILE];
            for (column, &value) in columns.iter().zip(input) {
                for j in 0..TILE {
                    sums[j] = madd::<FUSED>(value, column[j], sums[j]);
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
    pub(super) fn apply_all(&self, inputs: &[f32], bias: &[f32], out: &mut [f32]) {
        #[cfg(target_arch = "x86_64")]
        if fused() {
            // SAFETY: AVX2 и FMA у процессора есть - проверено `fused`
            return unsafe { self.apply_all_fused(inputs, bias, out) };
        }
        self.apply_all_with::<false>(inputs, bias, out);
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2,fma")]
    fn apply_all_fused(&self, inputs: &[f32], bias: &[f32], out: &mut [f32]) {
        self.apply_all_with::<true>(inputs, bias, out);
    }

    #[inline(always)]
    pub(super) fn apply_all_with<const FUSED: bool>(
        &self,
        inputs: &[f32],
        bias: &[f32],
        out: &mut [f32],
    ) {
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
            for (tile, columns) in self.columns.chunks_exact(self.cols).enumerate() {
                // Четыре отдельные суммы, а не массив: так они остаются в регистрах при любом
                // профиле сборки (с несколькими единицами кодогенерации массив уходил в память)
                let (mut s0, mut s1, mut s2, mut s3) =
                    ([0.0f32; TILE], [0.0; TILE], [0.0; TILE], [0.0; TILE]);
                for (column, values) in columns.iter().zip(&padded) {
                    axpy::<FUSED>(&mut s0, column, values[0]);
                    axpy::<FUSED>(&mut s1, column, values[1]);
                    axpy::<FUSED>(&mut s2, column, values[2]);
                    axpy::<FUSED>(&mut s3, column, values[3]);
                }
                let sums = [s0, s1, s2, s3];
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

/// `sums += value · column`
#[inline(always)]
pub(super) fn axpy<const FUSED: bool>(sums: &mut [f32; TILE], column: &[f32; TILE], value: f32) {
    for (sum, &weight) in sums.iter_mut().zip(column) {
        *sum = madd::<FUSED>(value, weight, *sum);
    }
}

/// `a · b + c` одной операцией FMA, где она есть: на aarch64 всегда, на x86_64 - в ядрах,
/// собранных с FMA (`FUSED`). Это на четверть быстрее, а последние биты суммы - другие, чем без FMA;
/// без нее `mul_add` - вызов библиотеки
#[inline(always)]
pub(super) fn madd<const FUSED: bool>(a: f32, b: f32, c: f32) -> f32 {
    if FUSED || cfg!(target_arch = "aarch64") {
        a.mul_add(b, c)
    } else {
        a * b + c
    }
}

/// Есть ли у процессора AVX2 и FMA: ядра умножения матриц собраны еще и с ними, а колеса x86_64 -
/// без них, чтобы работать на любом процессоре. Ответ макрос держит в памяти
#[cfg(target_arch = "x86_64")]
pub(super) fn fused() -> bool {
    std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma")
}

/// Скалярное произведение блоками по 8: так цикл векторизуется
pub(super) fn dot(a: &[f32], b: &[f32]) -> f32 {
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

pub(super) fn add(out: &mut [f32], values: &[f32]) {
    for (out, value) in out.iter_mut().zip(values) {
        *out += value;
    }
}

/// e^x с относительной ошибкой около 2e-7: без libm и ветвлений, поэтому цикл по массиву
/// векторизуется. Округление - сложением 1,5·2²³ (SSE2 не округляет векторно)
#[inline(always)]
pub(super) fn exp(x: f32) -> f32 {
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
pub(super) fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + exp(-x))
}

#[inline(always)]
pub(super) fn tanh(x: f32) -> f32 {
    2.0 / (1.0 + exp(-2.0 * x)) - 1.0
}

/// Ячейка LSTM одного направления: матрицы ворот в порядке PyTorch (i, f, g, o), смещения сложены
#[derive(Debug)]
pub(super) struct Lstm {
    pub(super) input: Tiled,
    pub(super) hidden: Tiled,
    pub(super) bias: Vec<f32>,
}

/// Значение записи файла весов
pub(super) enum Value<'a> {
    Strings(Vec<&'a str>),
    Matrix(Matrix),
    Numbers(Vec<u32>),
    Floats(Vec<f64>),
}

/// Чтение файла весов с проверкой границ
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    /// Модель весов - для сообщений об ошибках
    what: &'static str,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let end = self
            .at
            .checked_add(count)
            .filter(|&end| end <= self.data.len());
        let end = end.ok_or_else(|| invalid(self.what, "файл весов обрезан"))?;
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
            return Err(invalid(self.what, "размер записи больше допустимого"));
        }
        Ok(count)
    }

    fn f32s(&mut self, count: usize) -> Result<Vec<f32>, Error> {
        let bytes = self.take(
            count
                .checked_mul(4)
                .ok_or_else(|| invalid(self.what, "размер"))?,
        )?;
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
                        .map_err(|_| invalid(self.what, "строка не в UTF-8"))?;
                    strings.push(text);
                }
                Ok(Value::Strings(strings))
            }
            1 | 2 => {
                let (rows, cols) = (self.count(MAX_ROWS)?, self.count(MAX_DIM)?);
                let size = rows
                    .checked_mul(cols)
                    .ok_or_else(|| invalid(self.what, "размер"))?;
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
            _ => Err(invalid(
                self.what,
                &format!("неизвестный вид записи {kind}"),
            )),
        }
    }
}

/// Ошибка формата файла весов модели `what`
pub(super) fn invalid(what: &str, message: &str) -> Error {
    Error::Format(format!("{what}: {message}"))
}

/// Записи файла весов по именам
pub(super) struct Entries<'a> {
    pub(super) map: HashMap<&'a str, Value<'a>>,
    what: &'static str,
}

impl<'a> Entries<'a> {
    pub(super) fn take(&mut self, name: &str) -> Result<Value<'a>, Error> {
        self.map
            .remove(name)
            .ok_or_else(|| invalid(self.what, &format!("нет записи {name}")))
    }

    pub(super) fn matrix(&mut self, name: &str, rows: usize, cols: usize) -> Result<Matrix, Error> {
        match self.take(name)? {
            Value::Matrix(matrix) if matrix.rows == rows && matrix.cols == cols => Ok(matrix),
            Value::Matrix(matrix) => Err(invalid(
                self.what,
                &format!(
                    "{name}: размер {}×{}, нужен {rows}×{cols}",
                    matrix.rows, matrix.cols
                ),
            )),
            _ => Err(invalid(self.what, &format!("{name} - не матрица"))),
        }
    }

    pub(super) fn vector(&mut self, name: &str, size: usize) -> Result<Vec<f32>, Error> {
        Ok(self.matrix(name, 1, size)?.data)
    }

    pub(super) fn strings(&mut self, name: &str) -> Result<Vec<&'a str>, Error> {
        match self.take(name)? {
            Value::Strings(strings) => Ok(strings),
            _ => Err(invalid(self.what, &format!("{name} - не строки"))),
        }
    }

    /// Номера строк таблицы по строкам записи `name` с номера `first`; повтор строки - ошибка:
    /// иначе номер строки вышел бы за таблицу
    pub(super) fn index(&mut self, name: &str, first: u32) -> Result<HashMap<String, u32>, Error> {
        let strings = self.strings(name)?;
        let index: HashMap<String, u32> = (strings.iter().enumerate())
            .map(|(number, text)| ((*text).to_owned(), number as u32 + first))
            .collect();
        if index.len() != strings.len() {
            return Err(invalid(self.what, &format!("в {name} повторяются строки")));
        }
        Ok(index)
    }
}

/// Записи файла весов модели `what`: сигнатура `magic`, версия `format`, число записей и записи
pub(super) fn read<'a>(
    data: &'a [u8],
    magic: &[u8; 8],
    format: u32,
    what: &'static str,
) -> Result<Entries<'a>, Error> {
    if data.get(..magic.len()) != Some(magic.as_slice()) {
        return Err(invalid(what, "нет сигнатуры файла весов"));
    }
    let mut reader = Reader {
        data,
        at: magic.len(),
        what,
    };
    let version = reader.u32()?;
    if version != format {
        return Err(invalid(
            what,
            &format!("версия файла весов {version}, поддерживается {format}"),
        ));
    }
    let count = reader.count(256)?;
    let mut entries = HashMap::new();
    for _ in 0..count {
        let len = usize::from(u16::from_le_bytes(
            reader
                .take(2)?
                .try_into()
                .map_err(|_| invalid(what, "имя"))?,
        ));
        let name = std::str::from_utf8(reader.take(len)?)
            .map_err(|_| invalid(what, "имя записи не в UTF-8"))?;
        entries.insert(name, reader.value()?);
    }
    if reader.at != data.len() {
        return Err(invalid(what, "лишние данные после записей"));
    }
    Ok(Entries { map: entries, what })
}

/// Слой BiLSTM: выход токена - состояния прямого и обратного направления подряд. Направления
/// идут в одном цикле: их цепочки независимы, и процессор считает их одновременно
pub(super) fn bilstm(layer: &[Lstm; 2], input: &[f32], n: usize, d: usize) -> Vec<f32> {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn entry(out: &mut Vec<u8>, name: &str, kind: u8, body: &[u8]) {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(kind);
        out.extend_from_slice(body);
    }

    pub(crate) fn matrix(rows: usize, cols: usize, values: &[f32]) -> Vec<u8> {
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

    pub(crate) fn strings(items: &[&str]) -> Vec<u8> {
        let mut body = (items.len() as u32).to_le_bytes().to_vec();
        for item in items {
            body.extend_from_slice(&(item.len() as u32).to_le_bytes());
            body.extend_from_slice(item.as_bytes());
        }
        body
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
    fn fused_kernels_agree() {
        // Ядра с FMA и без нее различаются только последними битами сумм
        let (rows, cols) = (37, 23);
        let value = |i: usize| ((i * 7919) % 101) as f32 / 50.0 - 1.0;
        let tiled = Tiled::new(&Matrix {
            rows,
            cols,
            data: (0..rows * cols).map(value).collect(),
        });
        let inputs: Vec<f32> = (0..5 * cols).map(|i| value(i + 13)).collect();
        let bias: Vec<f32> = (0..rows).map(|i| value(i + 29)).collect();
        let (mut plain, mut fused) = (vec![0.0; 5 * rows], vec![0.0; 5 * rows]);
        tiled.apply_all_with::<false>(&inputs, &bias, &mut plain);
        tiled.apply_all_with::<true>(&inputs, &bias, &mut fused);
        let (mut plain_one, mut fused_one) = (bias.clone(), bias.clone());
        tiled.apply_add_with::<false>(&inputs[..cols], &mut plain_one);
        tiled.apply_add_with::<true>(&inputs[..cols], &mut fused_one);
        assert_eq!(plain_one, plain[..rows]);
        for (a, b) in plain
            .iter()
            .zip(&fused)
            .chain(plain_one.iter().zip(&fused_one))
        {
            assert!((a - b).abs() <= 1e-5 * a.abs().max(1.0), "{a} {b}");
        }
        // Выбор ядра по процессору: ядро с FMA на aarch64 и на x86-64 с AVX2 и FMA, иначе прежнее
        #[cfg(target_arch = "x86_64")]
        let with_fma = super::fused();
        #[cfg(not(target_arch = "x86_64"))]
        let with_fma = cfg!(target_arch = "aarch64");
        let (all, one) = match with_fma {
            true => (&fused, &fused_one),
            false => (&plain, &plain_one),
        };
        let mut chosen = vec![0.0; 5 * rows];
        tiled.apply_all(&inputs, &bias, &mut chosen);
        assert_eq!(&chosen, all);
        let mut chosen_one = bias.clone();
        tiled.apply_add(&inputs[..cols], &mut chosen_one);
        assert_eq!(&chosen_one, one);
    }

    #[test]
    fn dot_products() {
        let a: Vec<f32> = (0..19).map(|i| i as f32).collect();
        let b = vec![2.0f32; 19];
        assert_eq!(dot(&a, &b), 342.0);
    }
}
