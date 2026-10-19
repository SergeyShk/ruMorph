//! Модуль Python `rumorph._rumorph`

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyFloat, PyFrozenSet, PyMappingProxy, PyString, PyType};

/// Часть речи, признаки и пометы UD в текстовом виде: для сравнения, хэша, repr и pickle
#[derive(PartialEq, Eq, Hash)]
struct UdText {
    pos: String,
    feats: String,
    labels: String,
}

/// Объекты Python тега UD, общие для всех разборов с этим тегом
struct UdObjects {
    pos: Py<PyString>,
    feats: Py<PyMappingProxy>,
    labels: Py<PyFrozenSet>,
    text: Arc<UdText>,
}

impl UdObjects {
    fn new(py: Python<'_>, pos: &str, feats: &[(&str, &str)], labels: &[&str]) -> PyResult<Self> {
        let dict = PyDict::new(py);
        for (name, value) in feats {
            dict.set_item(PyString::intern(py, name), PyString::intern(py, value))?;
        }
        let text = UdText {
            pos: pos.to_owned(),
            feats: feats
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("|"),
            labels: labels.join(","),
        };
        Ok(UdObjects {
            pos: PyString::intern(py, pos).unbind(),
            feats: PyMappingProxy::new(py, dict.as_mapping()).unbind(),
            labels: PyFrozenSet::new(py, labels.iter().map(|label| PyString::intern(py, label)))?
                .unbind(),
            text: Arc::new(text),
        })
    }

    fn from_tag(py: Python<'_>, tag: &rumorph::UdTag) -> PyResult<Self> {
        let feats: Vec<(&str, &str)> = tag
            .feats
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        let labels: Vec<&str> = tag.labels.iter().map(String::as_str).collect();
        Self::new(py, &tag.pos, &feats, &labels)
    }
}

/// Разбор словоформы
///
/// Описание:
///     Часть речи и признаки - по Universal Dependencies в соглашениях UD_Russian-SynTagRus.
///     Разборы неизменяемы; их можно сравнивать, класть в множества и сохранять через pickle
///
/// Атрибуты:
///     word (str): Словоформа в написании словаря: с буквой «ё», если она есть в словаре
///     lemma (str): Начальная форма
///     pos (str): Часть речи UD: NOUN, VERB, ADJ, DET, ADP и т. д.
///     feats (Mapping[str, str]): Признаки UD: {"Case": "Gen", "Number": "Sing"}
///     labels (frozenset[str]): Пометы словаря без аналога в UD: Infr (разговорное), Arch
///         (устаревшее), Qual (качественное) и т. д.
///     prob (float): Вероятность разбора; сумма по всем разборам словоформы равна 1
///     source (str): Откуда взят разбор: "dictionary" - словоформа есть в словаре
///
/// Пример использования:
///     >>> from rumorph import Analyzer
///     >>> analysis = Analyzer().analyze("стали")[0]
///     >>> analysis.lemma, analysis.pos, analysis.feats["Tense"], sorted(analysis.labels)
///     ('стать', 'VERB', 'Past', ['intr'])
#[pyclass(frozen, module = "rumorph")]
struct Analysis {
    #[pyo3(get)]
    word: String,
    #[pyo3(get)]
    lemma: String,
    #[pyo3(get)]
    pos: Py<PyString>,
    #[pyo3(get)]
    feats: Py<PyMappingProxy>,
    #[pyo3(get)]
    labels: Py<PyFrozenSet>,
    #[pyo3(get)]
    prob: f64,
    #[pyo3(get)]
    source: &'static str,
    ud: Arc<UdText>,
}

#[pymethods]
impl Analysis {
    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Analysis(word={}, lemma={}, pos={}, feats={}, labels={}, prob={}, source={})",
            py_repr(py, &self.word)?,
            py_repr(py, &self.lemma)?,
            py_repr(py, &self.ud.pos)?,
            py_repr(py, &self.ud.feats)?,
            py_repr(py, &self.ud.labels)?,
            PyFloat::new(py, self.prob).repr()?,
            py_repr(py, self.source)?
        ))
    }

    fn __eq__(&self, other: &Bound<'_, Self>) -> bool {
        let other = other.get();
        self.word == other.word
            && self.lemma == other.lemma
            && self.prob == other.prob
            && self.source == other.source
            && self.ud == other.ud
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        (
            &self.word,
            &self.lemma,
            self.prob.to_bits(),
            self.source,
            &self.ud,
        )
            .hash(&mut hasher);
        hasher.finish()
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(Bound<'py, PyAny>, (AnalysisState,))> {
        let restore = py.import("rumorph._rumorph")?.getattr("_analysis")?;
        let state = (
            self.word.clone(),
            self.lemma.clone(),
            self.ud.pos.clone(),
            self.ud.feats.clone(),
            self.ud.labels.clone(),
            self.prob,
            self.source.to_owned(),
        );
        Ok((restore, (state,)))
    }
}

/// Поля разбора для pickle: слово, лемма, часть речи, признаки, пометы, вероятность, источник
type AnalysisState = (String, String, String, String, String, f64, String);

/// Восстановление разбора из pickle
#[pyfunction]
fn _analysis(py: Python<'_>, state: AnalysisState) -> PyResult<Analysis> {
    let (word, lemma, pos, feats, labels, prob, source) = state;
    let source = match source.as_str() {
        "dictionary" => rumorph::Source::Dictionary.as_str(),
        _ => {
            return Err(PyValueError::new_err(format!(
                "Неизвестный источник разбора: {source}"
            )));
        }
    };
    let feats = feats
        .split('|')
        .filter(|feat| !feat.is_empty())
        .map(|feat| feat.split_once('='))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| PyValueError::new_err(format!("Неверные признаки UD: {feats}")))?;
    let labels: Vec<&str> = labels
        .split(',')
        .filter(|label| !label.is_empty())
        .collect();
    let ud = UdObjects::new(py, &pos, &feats, &labels)?;
    Ok(Analysis {
        word,
        lemma,
        pos: ud.pos,
        feats: ud.feats,
        labels: ud.labels,
        prob,
        source,
        ud: ud.text,
    })
}

/// Морфологический анализатор
///
/// Описание:
///     Разбирает словоформы по словарю OpenCorpora: лемма, часть речи и признаки UD,
///     вероятность. Регистр не важен, буква «е» в слове может обозначать «ё». Словарь
///     загружается один раз при создании анализатора; анализатор можно использовать
///     из нескольких потоков
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
    /// Объекты тегов UD создаются при первом разборе с тегом: в тексте встречается малая доля тегов
    ud: Vec<PyOnceLock<UdObjects>>,
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
        let ud = dictionary
            .ud_tags()
            .iter()
            .map(|_| PyOnceLock::new())
            .collect();
        Ok(Analyzer {
            dictionary: Arc::new(dictionary),
            ud,
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
    ///     >>> [(analysis.lemma, analysis.pos, analysis.feats.get("Case")) for analysis in Analyzer().analyze("Стали")][:3]
    ///     [('стать', 'VERB', None), ('сталь', 'NOUN', 'Gen'), ('сталь', 'NOUN', 'Nom')]
    fn analyze(&self, py: Python<'_>, word: &str) -> PyResult<Vec<Analysis>> {
        self.dictionary
            .analyze(word)
            .into_iter()
            .map(|analysis| self.to_py(py, analysis))
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
        analyses
            .into_iter()
            .map(|analyses| {
                analyses
                    .into_iter()
                    .map(|analysis| self.to_py(py, analysis))
                    .collect()
            })
            .collect()
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

    /// Разборы OpenCorpora до перевода в UD: написание, тег, начальная форма (для проверки словаря)
    fn _opencorpora(&self, word: &str) -> Vec<(String, String, String)> {
        self.dictionary
            .opencorpora(word)
            .into_iter()
            .map(|(spelling, tag, lemma)| (spelling, tag.to_owned(), lemma))
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

impl Analyzer {
    fn to_py(&self, py: Python<'_>, analysis: rumorph::Analysis) -> PyResult<Analysis> {
        let number = usize::from(analysis.ud);
        let ud = self.ud[number].get_or_try_init(py, || {
            UdObjects::from_tag(py, &self.dictionary.ud_tags()[number])
        })?;
        Ok(Analysis {
            word: analysis.word,
            lemma: analysis.lemma,
            pos: ud.pos.clone_ref(py),
            feats: ud.feats.clone_ref(py),
            labels: ud.labels.clone_ref(py),
            prob: analysis.prob,
            source: analysis.source.as_str(),
            ud: Arc::clone(&ud.text),
        })
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
