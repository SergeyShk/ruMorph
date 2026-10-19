//! Модуль Python `rumorph._rumorph`

use std::path::PathBuf;
use std::sync::Arc;

use pyo3::exceptions::{PyFileNotFoundError, PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyString;

/// Разбор словоформы
///
/// Атрибуты:
///     word (str): Словоформа в написании словаря: с буквой «ё», если она есть в словаре
///     lemma (str): Начальная форма
///     prob (float): Вероятность разбора; сумма по всем разборам словоформы равна 1
///     source (str): Откуда взят разбор: "dictionary" - словоформа есть в словаре
///
/// Пример использования:
///     >>> from rumorph import Analyzer
///     >>> analysis = Analyzer().analyze("еж")[0]
///     >>> analysis.word, analysis.lemma, analysis.source
///     ('ёж', 'ёж', 'dictionary')
#[pyclass(frozen, eq, module = "rumorph")]
#[derive(PartialEq)]
struct Analysis {
    #[pyo3(get)]
    word: String,
    #[pyo3(get)]
    lemma: String,
    #[pyo3(get)]
    prob: f64,
    #[pyo3(get)]
    source: &'static str,
    tag: u16,
}

#[pymethods]
impl Analysis {
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Analysis(word={}, lemma={}, prob={}, source={})",
            py_repr(py, &self.word)?,
            py_repr(py, &self.lemma)?,
            self.prob,
            py_repr(py, self.source)?
        ))
    }
}

impl From<rumorph::Analysis> for Analysis {
    fn from(analysis: rumorph::Analysis) -> Self {
        Analysis {
            word: analysis.word,
            lemma: analysis.lemma,
            prob: analysis.prob,
            source: analysis.source.as_str(),
            tag: analysis.tag,
        }
    }
}

/// Морфологический анализатор
///
/// Описание:
///     Разбирает словоформы по словарю OpenCorpora. Регистр не важен, буква «е» в слове
///     может обозначать «ё». Словарь загружается один раз при создании анализатора;
///     анализатор можно использовать из нескольких потоков
///
/// Аргументы:
///     path (str | os.PathLike | None): Путь к файлу словаря; по умолчанию - словарь из пакета
///
/// Исключения:
///     FileNotFoundError: Если файла словаря нет
///     ValueError: Если файл не является словарем ruMorph или поврежден
///
/// Пример использования:
///     >>> from rumorph import Analyzer
///     >>> morph = Analyzer()
///     >>> [(analysis.lemma, round(analysis.prob, 3)) for analysis in morph.analyze("стали")][:2]
///     [('стать', 0.975), ('сталь', 0.011)]
#[pyclass(frozen, module = "rumorph")]
struct Analyzer {
    dictionary: Arc<rumorph::Dictionary>,
    path: PathBuf,
}

#[pymethods]
impl Analyzer {
    #[new]
    #[pyo3(signature = (path=None))]
    fn new(py: Python<'_>, path: Option<PathBuf>) -> PyResult<Self> {
        let path = match path {
            Some(path) => path,
            None => default_path(py)?,
        };
        let dictionary =
            py.detach(|| rumorph::Dictionary::open(&path))
                .map_err(|err| match err {
                    rumorph::Error::Io(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        PyFileNotFoundError::new_err(format!(
                            "Нет файла словаря: {}",
                            path.display()
                        ))
                    }
                    rumorph::Error::Io(err) => {
                        PyOSError::new_err(format!("{}: {err}", path.display()))
                    }
                    err => PyValueError::new_err(format!("{}: {err}", path.display())),
                })?;
        Ok(Analyzer {
            dictionary: Arc::new(dictionary),
            path,
        })
    }

    /// Путь к файлу словаря
    #[getter]
    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    /// Разборы словоформы
    ///
    /// Описание:
    ///     Разборы идут по убыванию вероятности. Для слова, которого нет в словаре, список пуст
    ///
    /// Аргументы:
    ///     word (str): Словоформа
    ///
    /// Вывод:
    ///     list[Analysis]: Разборы словоформы
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> [analysis.lemma for analysis in Analyzer().analyze("Стали")]
    ///     ['стать', 'сталь', 'сталь', 'сталь', 'сталь', 'сталь']
    fn analyze(&self, word: &str) -> Vec<Analysis> {
        self.dictionary
            .analyze(word)
            .into_iter()
            .map(Analysis::from)
            .collect()
    }

    /// Разборы списка словоформ
    ///
    /// Описание:
    ///     То же, что analyze для каждой словоформы; разбор идет без GIL, поэтому несколько
    ///     потоков разбирают свои списки параллельно
    ///
    /// Аргументы:
    ///     words (Iterable[str]): Словоформы
    ///
    /// Вывод:
    ///     list[list[Analysis]]: Разборы каждой словоформы
    ///
    /// Исключения:
    ///     TypeError: Если передана строка, а не набор слов, или слово не строка
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> [len(analyses) for analyses in Analyzer().analyze_many(["мама", "мыла", "раму", "вайб"])]
    ///     [1, 4, 2, 0]
    fn analyze_many(
        &self,
        py: Python<'_>,
        words: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<Vec<Analysis>>> {
        let words = collect_words(words)?;
        let dictionary = Arc::clone(&self.dictionary);
        let analyses = py.detach(move || {
            words
                .iter()
                .map(|word| dictionary.analyze(word))
                .collect::<Vec<_>>()
        });
        Ok(analyses
            .into_iter()
            .map(|analyses| analyses.into_iter().map(Analysis::from).collect())
            .collect())
    }

    /// Леммы списка словоформ
    ///
    /// Описание:
    ///     Лемма самого вероятного разбора; слово, которого нет в словаре, возвращается
    ///     в нижнем регистре. Разбор идет без GIL
    ///
    /// Аргументы:
    ///     words (Iterable[str]): Словоформы
    ///
    /// Вывод:
    ///     list[str]: Леммы
    ///
    /// Исключения:
    ///     TypeError: Если передана строка, а не набор слов, или слово не строка
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> Analyzer().lemmatize("Мы стали друзьями в ковиде".split())
    ///     ['мы', 'стать', 'друг', 'в', 'ковиде']
    fn lemmatize(&self, py: Python<'_>, words: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
        let words = collect_words(words)?;
        let dictionary = Arc::clone(&self.dictionary);
        Ok(py.detach(move || {
            words
                .into_iter()
                .map(|word| match dictionary.analyze(&word).into_iter().next() {
                    Some(analysis) => analysis.lemma,
                    None => word.to_lowercase(),
                })
                .collect()
        }))
    }

    /// Есть ли словоформа в словаре
    ///
    /// Аргументы:
    ///     word (str): Словоформа
    ///
    /// Вывод:
    ///     bool: Есть ли словоформа в словаре
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> morph = Analyzer()
    ///     >>> morph.is_known("Ежи"), morph.is_known("вайб")
    ///     (True, False)
    fn is_known(&self, word: &str) -> bool {
        self.dictionary.is_known(word)
    }

    /// Теги OpenCorpora разборов словоформы в порядке analyze (для проверки словаря в тестах)
    fn _tags(&self, word: &str) -> Vec<String> {
        let tags = self.dictionary.tags();
        self.dictionary
            .analyze(word)
            .iter()
            .map(|analysis| tags[analysis.tag as usize].clone())
            .collect()
    }

    /// Описание источника словаря в JSON (для проверки словаря в тестах)
    fn _meta(&self) -> &str {
        self.dictionary.meta()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Analyzer(path={})",
            py_repr(py, &self.path.display().to_string())?
        ))
    }
}

fn py_repr(py: Python<'_>, value: &str) -> PyResult<String> {
    Ok(PyString::new(py, value).repr()?.to_string())
}

fn default_path(py: Python<'_>) -> PyResult<PathBuf> {
    let init: PathBuf = py.import("rumorph")?.getattr("__file__")?.extract()?;
    Ok(init.with_file_name("data").join("ru.dict"))
}

fn collect_words(words: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    if words.is_instance_of::<PyString>() {
        return Err(PyTypeError::new_err(
            "Ожидается набор слов, а не строка: передайте список слов",
        ));
    }
    words
        .try_iter()?
        .map(|word| {
            word?
                .extract::<String>()
                .map_err(|_| PyTypeError::new_err("Каждое слово должно быть строкой"))
        })
        .collect()
}

#[pymodule]
fn _rumorph(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Analyzer>()?;
    m.add_class::<Analysis>()?;
    Ok(())
}
