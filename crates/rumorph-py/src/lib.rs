//! Модуль Python `rumorph._rumorph`

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pyo3::create_exception;
use pyo3::exceptions::{PyLookupError, PyOSError, PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyDict, PyFloat, PyFrozenSet, PyString};

create_exception!(
    rumorph,
    FormNotFoundError,
    PyLookupError,
    "У лексемы нет формы с запрошенными признаками или формы, согласованной с количеством"
);

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
    /// Образец признаков: наружу отдается копия
    feats: Py<PyDict>,
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
            feats: dict.unbind(),
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
///     feats (dict[str, str]): Признаки UD: {"Case": "Gen", "Number": "Sing"}; каждый раз новый
///         словарь, его изменение не меняет разбор
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
    /// Разбор ядра: его таблица словоизменения и форма дают остальные формы лексемы
    inner: rumorph::Analysis,
    #[pyo3(get)]
    pos: Py<PyString>,
    feats: Py<PyDict>,
    #[pyo3(get)]
    labels: Py<PyFrozenSet>,
    ud: Arc<UdText>,
    analyzer: Py<Analyzer>,
}

#[pymethods]
impl Analysis {
    #[getter]
    fn word(&self) -> &str {
        &self.inner.word
    }

    #[getter]
    fn lemma(&self) -> &str {
        &self.inner.lemma
    }

    #[getter]
    fn prob(&self) -> f64 {
        self.inner.prob
    }

    #[getter]
    fn source(&self) -> &'static str {
        self.inner.source.as_str()
    }

    /// Формы лексемы
    ///
    /// Описание:
    ///     Формы с той же леммой в порядке словаря; вероятность каждой формы - 1
    ///
    /// Вывод:
    ///     list[Analysis]: Формы лексемы
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> [form.word for form in Analyzer().analyze("ёж")[0].forms()][:6]
    ///     ['ёж', 'ежа', 'ежу', 'ежа', 'ежом', 'еже']
    fn forms(&self, py: Python<'_>) -> PyResult<Vec<Analysis>> {
        self.analyzer
            .get()
            .dictionary
            .forms(&self.inner)
            .into_iter()
            .map(|form| Analyzer::to_py(py, &self.analyzer, form))
            .collect()
    }

    /// Форма слова с заданными признаками
    ///
    /// Описание:
    ///     Ищет среди форм лексемы форму с заданными признаками UD; остальные признаки
    ///     по возможности сохраняются: «кошка» в родительном падеже - «кошки», а не «кошек»
    ///
    /// Аргументы:
    ///     **feats (str): Признаки UD и их значения: Case="Gen", Number="Plur"
    ///
    /// Вывод:
    ///     str: Словоформа в написании словаря
    ///
    /// Исключения:
    ///     FormNotFoundError: Если у лексемы нет формы с такими признаками
    ///     ValueError: Если такого признака или значения нет в словаре
    ///     TypeError: Если значение признака не строка
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> cat = Analyzer().analyze("кошка")[0]
    ///     >>> cat.inflect(Case="Gen"), cat.inflect(Case="Gen", Number="Plur")
    ///     ('кошки', 'кошек')
    #[pyo3(signature = (**feats))]
    fn inflect(&self, feats: Option<&Bound<'_, PyDict>>) -> PyResult<String> {
        let dictionary = &self.analyzer.get().dictionary;
        let mut requested: Vec<(String, String)> = Vec::new();
        for (name, value) in feats.into_iter().flatten() {
            let name: String = name.extract()?;
            let Ok(value) = value.cast::<PyString>() else {
                return Err(PyTypeError::new_err(format!(
                    "Значение признака {name} должно быть строкой, а не {}",
                    value.get_type().name()?
                )));
            };
            let value = value.to_str()?.to_owned();
            let Some(values) = dictionary.features().get(&name) else {
                return Err(PyValueError::new_err(format!(
                    "Неизвестный признак UD: {name}; есть {}",
                    join(dictionary.features().keys())
                )));
            };
            if !values.contains(&value) {
                return Err(PyValueError::new_err(format!(
                    "Неизвестное значение признака {name}: {value}; есть {}",
                    join(values)
                )));
            }
            requested.push((name, value));
        }
        requested.sort();
        let pairs: Vec<(&str, &str)> = requested
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        dictionary
            .inflect(&self.inner, &pairs)
            .map(|form| form.word)
            .ok_or_else(|| {
                let feats: Vec<String> = requested
                    .iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect();
                FormNotFoundError::new_err(format!(
                    "У слова «{}» нет формы {}",
                    self.inner.word,
                    feats.join("|")
                ))
            })
    }

    /// Форма слова, согласованная с количеством
    ///
    /// Описание:
    ///     Согласует существительное, прилагательное, местоименное прилагательное или причастие
    ///     с количеством в падеже разбора: «1 рубль», «2 рубля», «5 рублей», «о 2 рублях».
    ///     Отрицательное количество согласуется как его модуль
    ///
    /// Аргументы:
    ///     number (int): Количество
    ///
    /// Вывод:
    ///     str: Словоформа в написании словаря
    ///
    /// Исключения:
    ///     FormNotFoundError: Если слово не согласуется с количеством или у лексемы нет нужной формы
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> ruble = Analyzer().analyze("рубль")[0]
    ///     >>> [f"{n} {ruble.agree(n)}" for n in (1, 2, 5, 21)]
    ///     ['1 рубль', '2 рубля', '5 рублей', '21 рубль']
    fn agree(&self, number: &Bound<'_, PyAny>) -> PyResult<String> {
        // Согласование зависит от двух последних цифр и от того, меньше ли количество 5,
        // поэтому числа больше u64 сводятся к 100-199
        let count = match number.extract::<i128>() {
            Ok(number) => {
                let count = number.unsigned_abs();
                u64::try_from(count).unwrap_or(100 + (count % 100) as u64)
            }
            Err(err) if err.is_instance_of::<PyOverflowError>(number.py()) => {
                100 + number.abs()?.rem(100)?.extract::<u64>()?
            }
            Err(err) => return Err(err),
        };
        self.analyzer
            .get()
            .dictionary
            .agree(&self.inner, count)
            .map(|form| form.word)
            .ok_or_else(|| {
                FormNotFoundError::new_err(format!(
                    "Слово «{}» не согласуется с количеством {}",
                    self.inner.word,
                    number
                        .str()
                        .map_or_else(|_| count.to_string(), |text| text.to_string())
                ))
            })
    }

    #[getter]
    fn feats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.feats.bind(py).copy()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Analysis(word={}, lemma={}, pos={}, feats={}, labels={}, prob={}, source={})",
            py_repr(py, &self.inner.word)?,
            py_repr(py, &self.inner.lemma)?,
            py_repr(py, &self.ud.pos)?,
            py_repr(py, &self.ud.feats)?,
            py_repr(py, &self.ud.labels)?,
            PyFloat::new(py, self.inner.prob).repr()?,
            py_repr(py, self.inner.source.as_str())?
        ))
    }

    fn __eq__(&self, other: &Bound<'_, Self>) -> bool {
        let other = other.get();
        self.inner.word == other.inner.word
            && self.inner.lemma == other.inner.lemma
            && self.inner.prob == other.inner.prob
            && self.inner.source == other.inner.source
            && self.ud == other.ud
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        (
            &self.inner.word,
            &self.inner.lemma,
            self.inner.prob.to_bits(),
            self.inner.source.as_str(),
            &self.ud,
        )
            .hash(&mut hasher);
        hasher.finish()
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> Reduced<'py, (Py<Analyzer>, AnalysisState)> {
        let restore = py.import("rumorph._rumorph")?.getattr("_analysis")?;
        let state = (
            self.inner.word.clone(),
            self.inner.lemma.clone(),
            self.ud.pos.clone(),
            self.ud.feats.clone(),
            self.ud.labels.clone(),
            self.inner.prob,
            self.inner.source.as_str().to_owned(),
            self.inner.table,
            self.inner.form,
        );
        Ok((restore, (self.analyzer.clone_ref(py), state)))
    }
}

/// Функция восстановления объекта из pickle и ее аргументы
type Reduced<'py, Args> = PyResult<(Bound<'py, PyAny>, Args)>;

/// Поля разбора для pickle: слово, лемма, часть речи, признаки, пометы, вероятность, источник,
/// таблица словоизменения и форма в ней
type AnalysisState = (
    String,
    String,
    String,
    String,
    String,
    f64,
    String,
    u16,
    u16,
);

/// Восстановление разбора из pickle: по сохраненным таблице и форме, если словарь тот же,
/// иначе - среди разборов слова с теми же леммой, частью речи, признаками и пометами;
/// вероятность берется сохраненная
#[pyfunction]
fn _analysis(py: Python<'_>, analyzer: Py<Analyzer>, state: AnalysisState) -> PyResult<Analysis> {
    let (word, lemma, pos, feats, labels, prob, source, table, form) = state;
    if source != rumorph::Source::Dictionary.as_str() {
        return Err(PyValueError::new_err(format!(
            "Неизвестный источник разбора: {source}"
        )));
    }
    let dictionary = &analyzer.get().dictionary;
    let same = |analysis: &rumorph::Analysis| {
        let tag = &dictionary.ud_tags()[usize::from(analysis.ud)];
        analysis.word == word
            && analysis.lemma == lemma
            && tag.pos == pos
            && tag.feats_string() == feats
            && tag.labels.join(",") == labels
    };
    let found = dictionary
        .analysis(&word, table, form)
        .filter(|analysis| same(analysis))
        .or_else(|| {
            dictionary
                .analyze(&word)
                .into_iter()
                .find(|analysis| same(analysis))
        });
    let Some(mut inner) = found else {
        return Err(PyValueError::new_err(format!(
            "В словаре нет разбора «{word}» с леммой «{lemma}», частью речи {pos}, признаками «{feats}» и пометами «{labels}»"
        )));
    };
    inner.prob = prob;
    Analyzer::to_py(py, &analyzer, inner)
}

/// Анализаторы, восстановленные из pickle: один на словарь в процессе, а не на каждый объект
static RESTORED: PyOnceLock<Py<PyDict>> = PyOnceLock::new();

/// Восстановление анализатора из pickle
#[pyfunction]
fn _analyzer(py: Python<'_>, path: Option<PathBuf>) -> PyResult<Py<Analyzer>> {
    let restored = RESTORED
        .get_or_init(py, || PyDict::new(py).unbind())
        .bind(py);
    let key = path.clone().into_pyobject(py)?;
    if let Some(analyzer) = restored.get_item(&key)? {
        return Ok(analyzer.cast_into::<Analyzer>()?.unbind());
    }
    let analyzer = Py::new(py, Analyzer::new(py, path)?)?;
    Ok(restored
        .call_method1("setdefault", (key, analyzer))?
        .cast_into::<Analyzer>()?
        .unbind())
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

    fn __reduce__<'py>(&self, py: Python<'py>) -> Reduced<'py, (Option<PathBuf>,)> {
        let restore = py.import("rumorph._rumorph")?.getattr("_analyzer")?;
        Ok((restore, (self.explicit.then(|| self.path.clone()),)))
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
    fn analyze(slf: &Bound<'_, Self>, word: &str) -> PyResult<Vec<Analysis>> {
        let analyzer = slf.clone().unbind();
        slf.get()
            .dictionary
            .analyze(word)
            .into_iter()
            .map(|analysis| Self::to_py(slf.py(), &analyzer, analysis))
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
        slf: &Bound<'_, Self>,
        words: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<Vec<Analysis>>> {
        let (py, analyzer) = (slf.py(), slf.clone().unbind());
        let words = collect_words(words)?;
        let dictionary = Arc::clone(&slf.get().dictionary);
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
                    .map(|analysis| Self::to_py(py, &analyzer, analysis))
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

    /// Разборы OpenCorpora до перевода в UD и слияния: написание, тег, начальная форма, вероятность
    /// (для проверки словаря)
    fn _opencorpora(&self, word: &str) -> Vec<(String, String, String, f64)> {
        self.dictionary
            .opencorpora(word)
            .into_iter()
            .map(|(spelling, tag, lemma, prob)| (spelling, tag.to_owned(), lemma, prob))
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
    fn to_py(
        py: Python<'_>,
        analyzer: &Py<Analyzer>,
        analysis: rumorph::Analysis,
    ) -> PyResult<Analysis> {
        let this = analyzer.get();
        let number = usize::from(analysis.ud);
        let ud = this.ud[number].get_or_try_init(py, || {
            UdObjects::from_tag(py, &this.dictionary.ud_tags()[number])
        })?;
        Ok(Analysis {
            inner: analysis,
            pos: ud.pos.clone_ref(py),
            feats: ud.feats.clone_ref(py),
            labels: ud.labels.clone_ref(py),
            ud: Arc::clone(&ud.text),
            analyzer: analyzer.clone_ref(py),
        })
    }
}

fn join<'a>(values: impl IntoIterator<Item = &'a String>) -> String {
    values
        .into_iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
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
    m.add("FormNotFoundError", m.py().get_type::<FormNotFoundError>())?;
    m.add_function(wrap_pyfunction!(_analysis, m)?)?;
    m.add_function(wrap_pyfunction!(_analyzer, m)?)?;
    Ok(())
}
