//! Модуль Python `rumorph._rumorph`

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyFloat, PyString, PyType};

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
            PyFloat::new(py, self.prob).repr()?,
            py_repr(py, self.source)?
        ))
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        (
            &self.word,
            &self.lemma,
            self.prob.to_bits(),
            self.source,
            self.tag,
        )
            .hash(&mut hasher);
        hasher.finish()
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(Bound<'py, PyAny>, AnalysisState)> {
        let restore = py.import("rumorph._rumorph")?.getattr("_analysis")?;
        Ok((
            restore,
            (
                self.word.clone(),
                self.lemma.clone(),
                self.prob,
                self.source,
                self.tag,
            ),
        ))
    }
}

/// Аргументы `_analysis`: поля разбора для pickle
type AnalysisState = (String, String, f64, &'static str, u16);

/// Восстановление разбора из pickle
#[pyfunction]
fn _analysis(word: String, lemma: String, prob: f64, source: &str, tag: u16) -> PyResult<Analysis> {
    let source = match source {
        "dictionary" => rumorph::Source::Dictionary.as_str(),
        _ => {
            return Err(PyValueError::new_err(format!(
                "Неизвестный источник разбора: {source}"
            )));
        }
    };
    Ok(Analysis {
        word,
        lemma,
        prob,
        source,
        tag,
    })
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
///     OSError: Если файл словаря не читается
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
    /// Путь задан явно: иначе pickle не сохраняет путь, и в другом окружении берется словарь его пакета
    explicit: bool,
}

#[pymethods]
impl Analyzer {
    #[new]
    #[pyo3(signature = (path=None))]
    fn new(py: Python<'_>, path: Option<PathBuf>) -> PyResult<Self> {
        let explicit = path.is_some();
        let path = match path {
            Some(path) => path,
            None => default_path(py)?,
        };
        let dictionary = py
            .detach(|| rumorph::Dictionary::open(&path))
            .map_err(|err| load_error(&path, err))?;
        Ok(Analyzer {
            dictionary: Arc::new(dictionary),
            path,
            explicit,
        })
    }

    fn __reduce__<'py>(slf: &Bound<'py, Self>) -> (Bound<'py, PyType>, (Option<PathBuf>,)) {
        let analyzer = slf.get();
        let path = analyzer.explicit.then(|| analyzer.path.clone());
        (slf.get_type(), (path,))
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

/// Ошибка чтения файла - OSError с errno и именем файла (Python выбирает подкласс по errno),
/// неверный формат - ValueError
fn load_error(path: &Path, err: rumorph::Error) -> PyErr {
    let rumorph::Error::Io(err) = err else {
        return PyValueError::new_err(format!("{}: {err}", path.display()));
    };
    let message = match err.kind() {
        ErrorKind::NotFound => "Нет файла словаря".to_owned(),
        ErrorKind::PermissionDenied => "Нет доступа к файлу словаря".to_owned(),
        ErrorKind::IsADirectory => "Путь к словарю указывает на каталог".to_owned(),
        _ => err.to_string(),
    };
    let filename = path.display().to_string();
    match err.raw_os_error() {
        #[cfg(windows)]
        Some(code) => PyOSError::new_err((0, message, filename, code)),
        #[cfg(not(windows))]
        Some(errno) => PyOSError::new_err((errno, message, filename)),
        None => PyOSError::new_err(format!("{message}: {filename}")),
    }
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
        .enumerate()
        .map(|(number, word)| {
            let word = word?;
            let Ok(word) = word.cast::<PyString>() else {
                return Err(PyTypeError::new_err(format!(
                    "Каждое слово должно быть строкой: элемент {number} - {}",
                    word.get_type().name()?
                )));
            };
            Ok(word.to_str()?.to_owned())
        })
        .collect()
}

#[pymodule]
fn _rumorph(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Analyzer>()?;
    m.add_class::<Analysis>()?;
    m.add_function(wrap_pyfunction!(_analysis, m)?)?;
    Ok(())
}
