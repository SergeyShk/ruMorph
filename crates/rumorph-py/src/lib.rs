//! Модуль Python `rumorph._rumorph`

use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::SystemTime;

use pyo3::create_exception;
use pyo3::exceptions::{
    PyAttributeError, PyFileNotFoundError, PyLookupError, PyOSError, PyOverflowError, PyTypeError,
    PyValueError,
};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyFrozenSet, PyInt, PyString, PyTuple};

/// Сколько слов `analyze_many` и `lemmatize` разбирают за раз без GIL: между кусками
/// переводятся результаты и проверяется Ctrl+C
const CHUNK: usize = 8192;

/// Python работает без GIL (3.14t): поля разборов собираются заново, а не копируются из общих
/// объектов тега
static GIL_DISABLED: AtomicBool = AtomicBool::new(false);

fn gil_disabled() -> bool {
    GIL_DISABLED.load(Ordering::Relaxed)
}

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
    /// Образец признаков: с GIL наружу отдается копия
    feats: Py<PyDict>,
    /// Признаки интернированными строками: без GIL из них собирается новый словарь, а не копия
    /// образца, счетчик ссылок которого меняли бы все потоки
    pairs: Vec<(Py<PyString>, Py<PyString>)>,
    labels: Py<PyFrozenSet>,
    label_strings: Vec<Py<PyString>>,
    /// Признаки и пометы строками Python для pickle: один объект на тег, поэтому pickle списка
    /// разборов хранит их один раз
    feats_text: Py<PyString>,
    labels_text: Py<PyString>,
    text: UdText,
}

impl UdObjects {
    fn new(py: Python<'_>, pos: &str, feats: &[(&str, &str)], labels: &[&str]) -> PyResult<Self> {
        let pairs: Vec<(Py<PyString>, Py<PyString>)> = feats
            .iter()
            .map(|(name, value)| {
                (
                    PyString::intern(py, name).unbind(),
                    PyString::intern(py, value).unbind(),
                )
            })
            .collect();
        let dict = PyDict::new(py);
        for (name, value) in &pairs {
            dict.set_item(name, value)?;
        }
        let label_strings: Vec<Py<PyString>> = labels
            .iter()
            .map(|label| PyString::intern(py, label).unbind())
            .collect();
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
            pairs,
            labels: PyFrozenSet::new(py, &label_strings)?.unbind(),
            label_strings,
            feats_text: PyString::new(py, &text.feats).unbind(),
            labels_text: PyString::new(py, &text.labels).unbind(),
            text,
        })
    }

    fn from_tag(py: Python<'_>, tag: rumorph::UdTag) -> PyResult<Self> {
        let feats: Vec<(&str, &str)> = tag.feats().collect();
        let labels: Vec<&str> = tag.labels().collect();
        Self::new(py, tag.pos(), &feats, &labels)
    }
}

/// Разбор словоформы
///
/// Описание:
///     Часть речи и признаки - по Universal Dependencies в соглашениях UD_Russian-SynTagRus.
///     Разборы неизменяемы; их можно сравнивать (без вероятности: разбор равен своей форме),
///     класть в множества и сохранять через pickle
///
/// Атрибуты:
///     word (str): Словоформа в написании словаря: с буквой «ё», если она есть в словаре; у разборов
///         по правилам и угаданных - как в тексте, но у угаданных с «ё» из окончания словарной
///         формы
///     lemma (str): Начальная форма
///     pos (str): Часть речи UD: NOUN, VERB, ADJ, DET, ADP и т. д.
///     feats (dict[str, str]): Признаки UD: {"Case": "Gen", "Number": "Sing"}; каждый раз новый
///         словарь, его изменение не меняет разбор
///     labels (frozenset[str]): Пометы словаря без аналога в UD: Infr (разговорное), Arch
///         (устаревшее), Qual (качественное) и т. д.
///     prob (float): Вероятность разбора; сумма по всем разборам словоформы равна 1
///     source (str): Откуда взят разбор: "dictionary" - словоформа есть в словаре, "rule" - токена
///         нет в словаре, разбор по правилу (число, знак, латиница, римское число, инициал,
///         аббревиатура, сокращение с точкой), "guess" - незнакомое слово, разбор угадан
///         по окончанию или по частям (через дефис, с приставкой)
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
    /// Ручка анализатора потока: через нее берутся словарь и объекты тега. Своих ссылок на общие
    /// объекты тега у разбора нет: без GIL их счетчики ссылок меняли бы все потоки
    handle: Arc<Handle>,
}

impl Analysis {
    /// Объекты тега разбора: их создает `Analyzer::to_py`
    fn tag(&self) -> PyResult<&UdObjects> {
        self.handle
            .analyzer
            .get()
            .shared
            .ud
            .get(usize::from(self.inner.ud))
            .and_then(OnceLock::get)
            .map(Box::as_ref)
            .ok_or_else(|| PyValueError::new_err("Нет тега UD"))
    }
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
    fn pos(&self, py: Python<'_>) -> PyResult<Py<PyString>> {
        Ok(self.tag()?.pos.clone_ref(py))
    }

    #[getter]
    fn labels<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyFrozenSet>> {
        let tag = self.tag()?;
        if gil_disabled() {
            return PyFrozenSet::new(py, &tag.label_strings);
        }
        Ok(tag.labels.bind(py).clone())
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
        self.handle
            .analyzer
            .get()
            .shared
            .dictionary
            .forms(&self.inner)
            .into_iter()
            .map(|form| Analyzer::to_py(py, &self.handle, form))
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
        let dictionary = &self.handle.analyzer.get().shared.dictionary;
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
    ///     Отрицательное количество согласуется как его модуль, дробное - в родительном падеже
    ///     единственного числа: «1,5 рубля», «о 2,5 рубля»
    ///
    /// Аргументы:
    ///     number (int | float | Decimal | Fraction): Количество; целое - любое число с __index__,
    ///         а также Decimal и Fraction без дробной части
    ///
    /// Вывод:
    ///     str: Словоформа в написании словаря
    ///
    /// Исключения:
    ///     FormNotFoundError: Если слово не согласуется с количеством или у лексемы нет нужной формы
    ///     TypeError: Если количество не число
    ///     ValueError: Если количество - бесконечность или NaN
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> ruble = Analyzer().analyze("рубль")[0]
    ///     >>> [f"{n} {ruble.agree(n)}" for n in (1, 2, 5, 21)]
    ///     ['1 рубль', '2 рубля', '5 рублей', '21 рубль']
    fn agree(&self, number: &Bound<'_, PyAny>) -> PyResult<String> {
        let dictionary = &self.handle.analyzer.get().shared.dictionary;
        let form = match count(number)? {
            Some(count) => dictionary.agree(&self.inner, count),
            None => dictionary.agree_fraction(&self.inner),
        };
        form.map(|form| form.word).ok_or_else(|| {
            FormNotFoundError::new_err(format!(
                "Слово «{}» не согласуется с количеством {}",
                self.inner.word,
                number
                    .str()
                    .map_or_else(|_| "?".into(), |text| text.to_string())
            ))
        })
    }

    #[getter]
    fn feats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let tag = self.tag()?;
        if !gil_disabled() {
            return tag.feats.bind(py).copy();
        }
        let feats = PyDict::new(py);
        for (name, value) in &tag.pairs {
            feats.set_item(name, value)?;
        }
        Ok(feats)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let ud = &self.tag()?.text;
        Ok(format!(
            "Analysis(word={}, lemma={}, pos={}, feats={}, labels={}, prob={}, source={})",
            py_repr(py, &self.inner.word)?,
            py_repr(py, &self.inner.lemma)?,
            py_repr(py, &ud.pos)?,
            py_repr(py, &ud.feats)?,
            py_repr(py, &ud.labels)?,
            PyFloat::new(py, self.inner.prob).repr()?,
            py_repr(py, self.inner.source.as_str())?
        ))
    }

    /// Разборы равны при равных словоформе, лемме, части речи, признаках, пометах и источнике:
    /// вероятность зависит от остальных разборов словоформы, поэтому разбор равен своей форме
    fn __eq__(&self, other: &Bound<'_, Self>) -> bool {
        let other = other.get();
        self.inner.word == other.inner.word
            && self.inner.lemma == other.inner.lemma
            && self.inner.source == other.inner.source
            && matches!((self.tag(), other.tag()), (Ok(own), Ok(other)) if own.text == other.text)
    }

    fn __hash__(&self) -> PyResult<u64> {
        let mut hasher = DefaultHasher::new();
        (
            &self.inner.word,
            &self.inner.lemma,
            self.inner.source.as_str(),
            &self.tag()?.text,
        )
            .hash(&mut hasher);
        Ok(hasher.finish())
    }

    fn __copy__(slf: Bound<'_, Self>) -> Bound<'_, Self> {
        slf
    }

    fn __deepcopy__<'py>(slf: Bound<'py, Self>, _memo: &Bound<'py, PyAny>) -> Bound<'py, Self> {
        slf
    }

    fn __reduce__<'py>(&self, py: Python<'py>) -> Reduced<'py, (Py<Analyzer>, PickledState)> {
        let restore = py.import("rumorph._rumorph")?.getattr("_analysis")?;
        let tag = self.tag()?;
        let source = match self.inner.source {
            rumorph::Source::Rule => intern!(py, "rule"),
            rumorph::Source::Guess => intern!(py, "guess"),
            _ => intern!(py, "dictionary"),
        };
        let state = (
            self.inner.word.clone(),
            self.inner.lemma.clone(),
            tag.pos.clone_ref(py),
            tag.feats_text.clone_ref(py),
            tag.labels_text.clone_ref(py),
            self.inner.prob,
            source.clone().unbind(),
            self.inner.entry.map(|entry| (entry.table, entry.form)),
        );
        Ok((restore, (self.handle.analyzer.clone_ref(py), state)))
    }
}

/// Функция восстановления объекта из pickle и ее аргументы
type Reduced<'py, Args> = PyResult<(Bound<'py, PyAny>, Args)>;

/// Поля разбора в pickle: строки тега и источника - общие объекты
type PickledState = (
    String,
    String,
    Py<PyString>,
    Py<PyString>,
    Py<PyString>,
    f64,
    Py<PyString>,
    Option<(u16, u16)>,
);

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
    Option<(u16, u16)>,
);

/// Восстановление разбора из pickle: по сохраненным таблице и форме, если словарь тот же,
/// иначе - среди разборов слова с теми же леммой, частью речи, признаками и пометами;
/// вероятность берется сохраненная. Разбор, которого в словаре нет (сохранен со словарем
/// другой версии), восстанавливается как сохранен, но без формы таблицы, а если нет и его
/// тега - разбор слова, у которого к сохраненным признакам добавились новые («Нидерландов»
/// без рода в 0.2.0)
#[pyfunction]
fn _analysis(py: Python<'_>, analyzer: Py<Analyzer>, state: AnalysisState) -> PyResult<Analysis> {
    let (word, lemma, pos, feats, labels, prob, source, entry) = state;
    let Some(source) = [
        rumorph::Source::Dictionary,
        rumorph::Source::Rule,
        rumorph::Source::Guess,
    ]
    .into_iter()
    .find(|known| known.as_str() == source) else {
        return Err(PyValueError::new_err(format!(
            "Неизвестный источник разбора: {source}"
        )));
    };
    let dictionary = &analyzer.get().shared.dictionary;
    // Признаки сравниваются без учета порядка: до 0.2.0 он был другим
    let mut wanted: Vec<&str> = feats.split('|').filter(|feat| !feat.is_empty()).collect();
    wanted.sort_unstable();
    // Тег с теми же частью речи и пометами и с теми же признаками, а при `wider` - и с новыми
    let tag_like = |tag: &rumorph::UdTag<'_>, wider: bool| {
        let mut own: Vec<String> = tag
            .feats()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        own.sort_unstable();
        let feats = if wider {
            wanted
                .iter()
                .all(|feat| own.iter().any(|known| known == feat))
        } else {
            own == wanted
        };
        tag.pos() == pos && feats && tag.labels().collect::<Vec<_>>().join(",") == labels
    };
    let like = |analysis: &rumorph::Analysis, wider: bool| {
        analysis.word == word
            && analysis.source == source
            && analysis.lemma == lemma
            && dictionary
                .ud_tag(analysis.ud)
                .is_some_and(|tag| tag_like(&tag, wider))
    };
    let found = entry
        .and_then(|(table, form)| {
            dictionary.analysis(&word, rumorph::Entry { table, form }, source)
        })
        .filter(|analysis| like(analysis, false))
        .or_else(|| {
            let analyses = dictionary.analyze(&word);
            let ud = || {
                (dictionary.ud_tags().zip(0u16..))
                    .find_map(|(tag, ud)| tag_like(&tag, false).then_some(ud))
            };
            match analyses.iter().position(|analysis| like(analysis, false)) {
                Some(at) => analyses.into_iter().nth(at),
                None => match ud() {
                    Some(ud) => Some(rumorph::Analysis::new(
                        word.clone(),
                        lemma.clone(),
                        ud,
                        prob,
                        source,
                    )),
                    None => analyses.into_iter().find(|analysis| like(analysis, true)),
                },
            }
        });
    let Some(mut inner) = found else {
        return Err(PyValueError::new_err(format!(
            "Разбор «{word}» сохранен со словарем другой версии: в этом словаре нет части речи {pos} с признаками «{feats}» и пометами «{labels}»"
        )));
    };
    inner.prob = prob;
    Analyzer::to_py(py, &Analyzer::handle(analyzer.bind(py)), inner)
}

/// Словарь и объекты его тегов UD: общие для анализаторов с одним файлом словаря
struct Shared {
    dictionary: rumorph::Dictionary,
    /// Объекты тегов UD создаются при первом разборе с тегом: в тексте встречается малая доля тегов,
    /// поэтому ячейка - указатель, а не сами объекты
    ud: Vec<OnceLock<Box<UdObjects>>>,
    /// Путь, по которому словарь загружен
    path: PathBuf,
    /// Копии словаря со своими словами: последняя держится, пока жив сам словарь, поэтому разборы
    /// из pickle в задачах пула не собирают свои слова заново, а файл освобождается вместе
    /// со словарем
    copies: Mutex<WordCopies>,
}

/// Копии словаря со своими словами и последняя собранная
#[derive(Default)]
struct WordCopies {
    entries: Vec<(Arc<OwnWords>, Weak<Shared>)>,
    last: Option<Arc<Shared>>,
}

impl Shared {
    fn new(dictionary: rumorph::Dictionary, path: &Path) -> Self {
        let ud = dictionary.ud_tags().map(|_| OnceLock::new()).collect();
        Shared {
            dictionary,
            ud,
            path: path.to_owned(),
            copies: Mutex::default(),
        }
    }
}

/// Файл словаря: его идентичность, размер и время изменения (на случай перезаписи на месте),
/// время создания (замена файлом с тем же размером и временем изменения)
#[derive(Clone, PartialEq, Eq)]
struct FileKey {
    id: FileId,
    size: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

/// На Unix - устройство и номер файла: симлинки и «..» дают тот же файл, замена через rename -
/// другой
#[cfg(unix)]
type FileId = (u64, u64);

#[cfg(not(unix))]
type FileId = PathBuf;

#[cfg(unix)]
fn file_id(_path: &Path, metadata: &std::fs::Metadata) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn file_id(path: &Path, _metadata: &std::fs::Metadata) -> Option<FileId> {
    std::fs::canonicalize(path).ok()
}

impl FileKey {
    /// Ключ обычного файла; у канала или устройства ключа нет: их словарь не делится
    fn new(path: &Path, metadata: &std::fs::Metadata) -> Option<Self> {
        if !metadata.is_file() {
            return None;
        }
        Some(FileKey {
            id: file_id(path, metadata)?,
            size: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
        })
    }
}

/// Словари, загруженные в процессе. Анализаторы с одним файлом словаря делят словарь, а разборы
/// из pickle не загружают его еще раз; словарь освобождается вместе с последним анализатором,
/// кроме последнего полученного: его процесс держит и без анализаторов, иначе каждая задача
/// `multiprocessing.Pool` и анализатор, созданный на один вызов, загружали бы словарь заново.
///
/// Блокировка - свой флаг, а не `Mutex`: перед fork ее берет поток, который делает fork
/// (`os.register_at_fork`), и снимает в обоих процессах. Иначе дочерний процесс может получить ее
/// запертой потоком, которого в нем нет, и зависнуть на первом же анализаторе
struct Dictionaries {
    locked: AtomicBool,
    state: UnsafeCell<Loaded>,
}

/// Список словарей и последний полученный словарь
struct Loaded {
    entries: Vec<(FileKey, Slot)>,
    last: Option<Arc<Shared>>,
}

impl Loaded {
    /// Запоминает последний полученный словарь; вывод - прежний, его освобождают уже без флага
    fn keep(&mut self, shared: &Arc<Shared>) -> Option<Arc<Shared>> {
        if self
            .last
            .as_ref()
            .is_some_and(|known| Arc::ptr_eq(known, shared))
        {
            return None;
        }
        self.last.replace(Arc::clone(shared))
    }
}

/// Словарь в списке загруженных
enum Slot {
    /// Словарь загружает другой поток: его ждут, а не загружают второй раз
    Loading,
    Loaded(Weak<Shared>),
}

// SAFETY: `state` читается и меняется только под флагом `locked`
unsafe impl Sync for Dictionaries {}

static DICTIONARIES: Dictionaries = Dictionaries {
    locked: AtomicBool::new(false),
    state: UnsafeCell::new(Loaded {
        entries: Vec::new(),
        last: None,
    }),
};

impl Dictionaries {
    /// Флаг ждут без GIL, а под флагом нет обращений к Python: он снимается и тогда, когда поток
    /// с GIL ждет его перед fork
    fn lock(&self) {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            std::thread::yield_now();
        }
    }

    fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }

    /// Действие со словарями под флагом
    fn with<T>(&self, action: impl FnOnce(&mut Loaded) -> T) -> T {
        self.lock();
        self.locked_with(action)
    }

    /// Действие со словарями, если флаг свободен: поток с GIL не ждет флаг
    fn try_with<T>(&self, action: impl FnOnce(&mut Loaded) -> T) -> Option<T> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()?;
        Some(self.locked_with(action))
    }

    fn locked_with<T>(&self, action: impl FnOnce(&mut Loaded) -> T) -> T {
        struct Unlock<'a>(&'a Dictionaries);
        impl Drop for Unlock<'_> {
            fn drop(&mut self) {
                self.0.unlock();
            }
        }
        let _unlock = Unlock(self);
        // SAFETY: флаг взят только этим потоком
        action(unsafe { &mut *self.state.get() })
    }
}

/// Перед fork: флаг словарей берет поток, который делает fork
#[pyfunction]
fn _before_fork() {
    DICTIONARIES.lock();
}

#[pyfunction]
fn _after_fork_in_parent() {
    DICTIONARIES.unlock();
}

/// В дочернем процессе нет потоков, которые загружали словари: их отметки снимаются
#[pyfunction]
fn _after_fork_in_child() {
    DICTIONARIES.unlock();
    DICTIONARIES.with(|loaded| {
        loaded
            .entries
            .retain(|(_, slot)| matches!(slot, Slot::Loaded(_)));
    });
}

/// Что делать со словарем файла
enum Lookup {
    Found(Arc<Shared>),
    Wait,
    Load,
}

/// Отметка «загружается» у словаря, который загружает этот поток: при выходе - словарь
/// или, если загрузка не удалась, снятие отметки (иначе другие потоки ждали бы вечно)
struct Loading<'a> {
    key: &'a FileKey,
    shared: Option<Weak<Shared>>,
}

impl Drop for Loading<'_> {
    fn drop(&mut self) {
        DICTIONARIES.with(|loaded| {
            let entries = &mut loaded.entries;
            let Some(at) = entries
                .iter()
                .position(|(known, slot)| known == self.key && matches!(slot, Slot::Loading))
            else {
                return;
            };
            match self.shared.take() {
                Some(shared) => entries[at].1 = Slot::Loaded(shared),
                None => {
                    entries.remove(at);
                }
            }
        });
    }
}

/// Словарь файла `path`: загруженный в процессе, если файл тот же и не изменился, иначе новый.
/// Флаг словарей берется только на поиск и запись, загрузка идет без него
/// Вывод - словарь и прежний последний словарь: его освобождают в потоке Python, иначе
/// его объекты Python уходят в пул отложенных decref PyO3, мьютекс которого может застать fork
fn dictionary(path: &Path) -> Result<(Arc<Shared>, Option<Arc<Shared>>), rumorph::Error> {
    let shared = find_or_load(path)?;
    let previous = DICTIONARIES.with(|loaded| loaded.keep(&shared));
    Ok((shared, previous))
}

/// Словарь, загруженный в процессе по пути `path`: им восстанавливается pickle, если файла уже нет
fn loaded_from(py: Python<'_>, path: &Path) -> Option<Arc<Shared>> {
    // Флаг ждут без GIL, а лишние ссылки на словари освобождаются уже с GIL
    let alive: Vec<Arc<Shared>> = py.detach(|| {
        DICTIONARIES.with(|loaded| {
            let entries = loaded.entries.iter().filter_map(|(_, slot)| match slot {
                Slot::Loaded(shared) => shared.upgrade(),
                Slot::Loading => None,
            });
            loaded.last.iter().cloned().chain(entries).collect()
        })
    });
    alive.into_iter().find(|shared| shared.path == path)
}

/// Словарь файла `path`, если он уже загружен и флаг словарей свободен: так анализатор,
/// созданный на один вызов, не отпускает GIL. Иначе `None`
fn loaded_dictionary(path: &Path) -> Option<(Arc<Shared>, Option<Arc<Shared>>)> {
    let key = FileKey::new(path, &std::fs::metadata(path).ok()?)?;
    DICTIONARIES
        .try_with(|loaded| {
            let shared = loaded.entries.iter().find_map(|(known, slot)| match slot {
                Slot::Loaded(shared) if *known == key => shared.upgrade(),
                _ => None,
            })?;
            let previous = loaded.keep(&shared);
            Some((shared, previous))
        })
        .flatten()
}

fn find_or_load(path: &Path) -> Result<Arc<Shared>, rumorph::Error> {
    let Some(key) = FileKey::new(path, &std::fs::metadata(path)?) else {
        return Ok(Arc::new(Shared::new(
            rumorph::Dictionary::open(path)?,
            path,
        )));
    };
    loop {
        let lookup = DICTIONARIES.with(|loaded| {
            let entries = &mut loaded.entries;
            entries.retain(|(_, slot)| match slot {
                Slot::Loading => true,
                Slot::Loaded(shared) => shared.strong_count() > 0,
            });
            let Some(at) = entries.iter().position(|(known, _)| *known == key) else {
                entries.push((key.clone(), Slot::Loading));
                return Lookup::Load;
            };
            match &entries[at].1 {
                Slot::Loading => Lookup::Wait,
                Slot::Loaded(shared) => match shared.upgrade() {
                    Some(shared) => Lookup::Found(shared),
                    // Последний анализатор удален после проверки выше
                    None => {
                        entries[at].1 = Slot::Loading;
                        Lookup::Load
                    }
                },
            }
        });
        match lookup {
            Lookup::Found(shared) => return Ok(shared),
            Lookup::Wait => std::thread::sleep(std::time::Duration::from_millis(1)),
            Lookup::Load => {
                let mut loading = Loading {
                    key: &key,
                    shared: None,
                };
                let shared = Arc::new(Shared::new(rumorph::Dictionary::open(path)?, path));
                loading.shared = Some(Arc::downgrade(&shared));
                return Ok(shared);
            }
        }
    }
}

/// Команда `rumorph` с аргументами `args`: код выхода; разбор аргументов, чтение и вывод -
/// в ядре, словарь по умолчанию - словарь пакета
#[pyfunction]
fn _main(py: Python<'_>, args: Vec<std::ffi::OsString>) -> PyResult<u8> {
    let path = default_path(py)?;
    let version = package_version();
    Ok(py.detach(|| rumorph::cli::run(args, &version, Some(&path))))
}

/// Версия крейта в записи PEP 440: «0.1.0-dev.12» - «0.1.0.dev12»
fn package_version() -> String {
    env!("CARGO_PKG_VERSION").replace("-dev.", ".dev")
}

/// Восстановление анализатора из pickle: анализатор с тем же путем к словарю, разборы которого
/// есть в потоке (pickle разборов не создает анализатор на каждый разбор), иначе новый со словарем
/// файла, а если файла уже нет - со словарем, загруженным из него в процессе
#[pyfunction]
#[pyo3(signature = (path, words=None))]
fn _analyzer(
    py: Python<'_>,
    path: Option<PathBuf>,
    words: Option<String>,
) -> PyResult<Py<Analyzer>> {
    let explicit = path.is_some();
    let path = Analyzer::resolve(py, path)?;
    let words = words.unwrap_or_default();
    let found = HANDLES
        .try_with(|handles| {
            handles
                .borrow()
                .slots
                .iter()
                .filter_map(|(_, handle)| handle.upgrade())
                .find(|handle| {
                    let analyzer = handle.analyzer.get();
                    analyzer.explicit == explicit
                        && analyzer.path == path
                        && analyzer.words.text == words
                })
                .map(|handle| handle.analyzer.clone_ref(py))
        })
        .ok()
        .flatten();
    if let Some(analyzer) = found {
        return Ok(analyzer);
    }
    let base = match Analyzer::load(py, path.clone(), explicit) {
        Ok(analyzer) => analyzer,
        // Тип ошибки - OSError, а значение по errno - FileNotFoundError
        Err(err) if err.value(py).is_instance_of::<PyFileNotFoundError>() => {
            match loaded_from(py, &path) {
                Some(shared) => Analyzer {
                    base: Arc::clone(&shared),
                    shared,
                    path,
                    explicit,
                    words: Arc::default(),
                },
                None => return Err(err),
            }
        }
        Err(err) => return Err(err),
    };
    if words.is_empty() {
        Py::new(py, base)
    } else {
        Py::new(py, Analyzer::with_words(py, base, words)?)
    }
}

/// Морфологический анализатор
///
/// Описание:
///     Разбирает словоформы по словарю OpenCorpora: лемма, часть речи и признаки UD,
///     вероятность. Регистр для поиска не важен, но меняет доли имен собственных, буква «е»
///     в слове может обозначать «ё». Словарь
///     загружается один раз при создании анализатора; анализатор можно использовать
///     из нескольких потоков. Файл словаря отображается в память и общий для процессов:
///     пока он открыт, его нельзя перезаписывать на месте. Последний загруженный словарь
///     остается открытым и без анализаторов
///
/// Аргументы:
///     path (str | os.PathLike | None): Путь к файлу словаря; по умолчанию - словарь из пакета
///     words (Mapping[str, str | tuple[str, str]] | None): Свои слова и их образцы: слово
///         изменяется как словарное слово-образец в той же форме (обычно в начальной). Если
///         у образца лексемы разных частей речи, образец - пара «слово, часть речи UD».
///         Образцом может быть и свое слово, стоящее раньше
///
/// Исключения:
///     FileNotFoundError: Если файла словаря нет
///     OSError: Если файл словаря не читается
///     TypeError: Если путь не строка и не os.PathLike или свои слова - не словарь строк
///     ValueError: Если файл не является словарем ruMorph или поврежден, образца нет в словаре,
///         у него лексемы разных частей речи без указания части речи, слово не изменяется
///         по образцу или в слове пробел, управляющий или невидимый символ
///
/// Пример использования:
///     >>> from rumorph import Analyzer
///     >>> morph = Analyzer()
///     >>> [(analysis.lemma, round(analysis.prob, 3)) for analysis in morph.analyze("стали")][:2]
///     [('стать', 0.988), ('сталь', 0.005)]
///     >>> morph = Analyzer(words={"зумер": "геймер", "зечь": ("печь", "VERB")})
///     >>> morph.analyze("зумерами")[0].lemma, morph.analyze("зечет")[0].lemma
///     ('зумер', 'зечь')
#[pyclass(frozen, module = "rumorph")]
struct Analyzer {
    shared: Arc<Shared>,
    /// Общий словарь файла: у анализатора со своими словами `shared` - его копия. Пока жив
    /// анализатор, словарь файла находится и после удаления файла, а копия - в его `copies`
    base: Arc<Shared>,
    path: PathBuf,
    /// Путь задан явно: иначе pickle не сохраняет путь, и в другом окружении берется словарь его пакета
    explicit: bool,
    words: Arc<OwnWords>,
}

/// Свое слово: слово, образец и часть речи образца, если она задана
type OwnWord = (String, String, Option<String>);

/// Свои слова анализатора одной строкой: pickle анализатора и каждого разбора копирует одну
/// строку, а не тысячи кортежей
#[derive(Default)]
struct OwnWords {
    /// Строки «слово, образец, часть речи» через табуляцию. Табуляций и переводов строк в полях
    /// нет: слово с ними отвергает ядро, а образец и часть речи с ними не находятся
    text: String,
    /// Та же строка в Python
    object: Option<Py<PyString>>,
}

impl OwnWords {
    fn text(words: &[OwnWord]) -> String {
        words
            .iter()
            .map(|(word, like, pos)| format!("{word}\t{like}\t{}\n", pos.as_deref().unwrap_or("")))
            .collect()
    }

    fn parse(text: &str) -> Vec<OwnWord> {
        text.lines()
            .map(|line| {
                let mut fields = line.split('\t').map(str::to_owned);
                let (word, like) = (
                    fields.next().unwrap_or_default(),
                    fields.next().unwrap_or_default(),
                );
                let pos = fields.next().filter(|pos| !pos.is_empty());
                (word, like, pos)
            })
            .collect()
    }
}

/// Анализатор для разборов одного потока. Разборы держат анализатор через ручку своего потока:
/// в Python без GIL счетчик ссылок общего объекта, который меняют сразу несколько потоков,
/// в несколько раз замедляет разбор
struct Handle {
    analyzer: Py<Analyzer>,
}

/// Сколько ручек помнит поток: анализаторов, которыми поток пользуется попеременно, немного,
/// а длинный список ручек сделал бы распаковку тысяч разборов из pickle квадратичной
const HANDLE_SLOTS: usize = 4;

/// Ручки анализаторов потока: адрес объекта анализатора и слабая ссылка - ручка живет, пока живы
/// ее разборы, и не держит анализатор дольше них; новая ручка занимает место умершей или
/// следующее место по кругу
struct Handles {
    slots: [(usize, Weak<Handle>); HANDLE_SLOTS],
    next: usize,
}

thread_local! {
    static HANDLES: RefCell<Handles> = const {
        RefCell::new(Handles {
            slots: [const { (0, Weak::new()) }; HANDLE_SLOTS],
            next: 0,
        })
    };
}

#[pymethods]
impl Analyzer {
    #[new]
    #[pyo3(signature = (path=None, words=None))]
    fn new(
        py: Python<'_>,
        path: Option<&Bound<'_, PyAny>>,
        words: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let path = match path {
            Some(path) if !path.is_none() => Some(path.extract::<PathBuf>().map_err(|_| {
                type_error("Путь к словарю должен быть строкой или os.PathLike", path)
            })?),
            _ => None,
        };
        let words = match words {
            Some(words) if !words.is_none() => own_words(words)?,
            _ => Vec::new(),
        };
        Self::open(py, path, words)
    }

    fn __copy__(slf: Bound<'_, Self>) -> Bound<'_, Self> {
        slf
    }

    fn __deepcopy__<'py>(slf: Bound<'py, Self>, _memo: &Bound<'py, PyAny>) -> Bound<'py, Self> {
        slf
    }

    fn __reduce__<'py>(
        &self,
        py: Python<'py>,
    ) -> Reduced<'py, (Option<PathBuf>, Option<Py<PyString>>)> {
        let restore = py.import("rumorph._rumorph")?.getattr("_analyzer")?;
        let words = self.words.object.as_ref().map(|words| words.clone_ref(py));
        Ok((restore, (self.explicit.then(|| self.path.clone()), words)))
    }

    /// Путь к файлу словаря
    #[getter]
    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    /// Разборы словоформы
    ///
    /// Описание:
    ///     Разборы идут по убыванию вероятности. Токен, которого нет в словаре, разбирается
    ///     по правилам (числа, знаки, латиница, сокращения), а разборы незнакомого слова
    ///     кириллицей угадываются по окончанию или по частям. Если ничего не подошло, токен
    ///     получает разбор X без признаков; пустой список - у токена с пробелом или невидимым
    ///     символом
    ///
    /// Аргументы:
    ///     word (str): Словоформа
    ///
    /// Вывод:
    ///     list[Analysis]: Разборы словоформы
    ///
    /// Исключения:
    ///     TypeError: Если слово не строка
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> [(analysis.lemma, analysis.pos, analysis.feats.get("Case")) for analysis in Analyzer().analyze("Стали")][:3]
    ///     [('стать', 'VERB', None), ('сталь', 'NOUN', 'Gen'), ('сталь', 'NOUN', 'Nom')]
    fn analyze(slf: &Bound<'_, Self>, word: &Bound<'_, PyAny>) -> PyResult<Vec<Analysis>> {
        let word = text(word)?;
        let handle = Self::handle(slf);
        slf.get()
            .shared
            .dictionary
            .analyze(&word)
            .into_iter()
            .map(|analysis| Self::to_py(slf.py(), &handle, analysis))
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
    ///     [1, 4, 2, 1]
    fn analyze_many(
        slf: &Bound<'_, Self>,
        words: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<Vec<Analysis>>> {
        let (py, handle) = (slf.py(), Self::handle(slf));
        let words = collect_words(words)?;
        let dictionary = &slf.get().shared.dictionary;
        let mut result = Vec::with_capacity(words.len());
        for chunk in words.chunks(CHUNK) {
            let analyses = py.detach(|| {
                chunk
                    .iter()
                    .map(|word| dictionary.analyze(word))
                    .collect::<Vec<_>>()
            });
            for analyses in analyses {
                result.push(
                    analyses
                        .into_iter()
                        .map(|analysis| Self::to_py(py, &handle, analysis))
                        .collect::<PyResult<_>>()?,
                );
            }
            py.check_signals()?;
        }
        Ok(result)
    }

    /// Леммы списка словоформ
    ///
    /// Описание:
    ///     Лемма самого вероятного разбора; токен без разборов возвращается в нижнем регистре.
    ///     Разбор идет без GIL
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
    ///     ['мы', 'стать', 'друг', 'в', 'ковид']
    fn lemmatize(&self, py: Python<'_>, words: &Bound<'_, PyAny>) -> PyResult<Vec<Py<PyString>>> {
        let words = collect_words(words)?;
        // Одинаковые леммы - один объект строки: память списка лемм корпуса в разы меньше
        let mut strings: HashMap<String, Py<PyString>> = HashMap::new();
        let mut result = Vec::with_capacity(words.len());
        for chunk in words.chunks(CHUNK) {
            let lemmas = py.detach(|| {
                chunk
                    .iter()
                    .map(
                        |word| match self.shared.dictionary.analyze(word).into_iter().next() {
                            Some(analysis) => analysis.lemma,
                            None => word.to_lowercase(),
                        },
                    )
                    .collect::<Vec<_>>()
            });
            for lemma in lemmas {
                let object = match strings.get(&lemma) {
                    Some(object) => object.clone_ref(py),
                    None => {
                        let object = PyString::new(py, &lemma).unbind();
                        strings.insert(lemma, object.clone_ref(py));
                        object
                    }
                };
                result.push(object);
            }
            py.check_signals()?;
        }
        Ok(result)
    }

    /// Есть ли словоформа в словаре
    ///
    /// Аргументы:
    ///     word (str): Словоформа
    ///
    /// Вывод:
    ///     bool: Есть ли словоформа в словаре
    ///
    /// Исключения:
    ///     TypeError: Если слово не строка
    ///
    /// Пример использования:
    ///     >>> from rumorph import Analyzer
    ///     >>> morph = Analyzer()
    ///     >>> morph.is_known("Ежи"), morph.is_known("вайб")
    ///     (True, False)
    fn is_known(&self, word: &Bound<'_, PyAny>) -> PyResult<bool> {
        Ok(self.shared.dictionary.is_known(&text(word)?))
    }

    /// Разборы OpenCorpora до перевода в UD и слияния: написание, тег, начальная форма, вероятность
    /// (для проверки словаря)
    fn _opencorpora(&self, word: &str) -> Vec<(String, String, String, f64)> {
        self.shared
            .dictionary
            .opencorpora(word)
            .into_iter()
            .map(|(spelling, tag, lemma, prob)| (spelling, tag.to_owned(), lemma, prob))
            .collect()
    }

    /// Доли разборов словоформы - по частотам корпуса UD (для сверки с pymorphy3 в тестах)
    fn _table_frequencies(&self, word: &str) -> bool {
        self.shared.dictionary.table_frequencies(word)
    }

    /// Описание источника словаря в JSON (для проверки словаря в тестах)
    fn _meta(&self) -> &str {
        self.shared.dictionary.meta()
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Analyzer(path={})",
            py_repr(py, &self.path.display().to_string())?
        ))
    }
}

impl Analyzer {
    /// Анализатор со словарем файла `path` (по умолчанию - словарем из пакета)
    fn open(py: Python<'_>, path: Option<PathBuf>, words: Vec<OwnWord>) -> PyResult<Self> {
        let explicit = path.is_some();
        let path = Self::resolve(py, path)?;
        let base = Self::load(py, path, explicit)?;
        if words.is_empty() {
            Ok(base)
        } else {
            Self::with_words(py, base, OwnWords::text(&words))
        }
    }

    /// Путь к словарю: абсолютный, чтобы pickle работал в другом рабочем каталоге
    fn resolve(py: Python<'_>, path: Option<PathBuf>) -> PyResult<PathBuf> {
        match path {
            Some(path) => Ok(std::path::absolute(&path).unwrap_or(path)),
            None => default_path(py),
        }
    }

    fn load(py: Python<'_>, path: PathBuf, explicit: bool) -> PyResult<Self> {
        let (shared, previous) = match loaded_dictionary(&path) {
            Some(found) => found,
            None => py
                .detach(|| dictionary(&path))
                .map_err(|err| load_error(&path, err))?,
        };
        drop(previous);
        Ok(Analyzer {
            base: Arc::clone(&shared),
            shared,
            path,
            explicit,
            words: Arc::default(),
        })
    }

    /// Анализатор `base` со своими словами в записи `OwnWords::text`: копия его словаря со своими
    /// словами собирается один раз на словарь и набор слов
    fn with_words(py: Python<'_>, base: Analyzer, text: String) -> PyResult<Self> {
        let copies = || (base.base.copies.lock()).unwrap_or_else(PoisonError::into_inner);
        let found = {
            let mut copies = copies();
            copies
                .entries
                .retain(|(_, shared)| shared.strong_count() > 0);
            copies
                .entries
                .iter()
                .filter(|(known, _)| known.text == text)
                .find_map(|(known, shared)| Some((shared.upgrade()?, Arc::clone(known))))
        };
        if let Some((shared, words)) = found {
            return Ok(Analyzer {
                shared,
                words,
                ..base
            });
        }
        let words = OwnWords::parse(&text);
        let dictionary = py
            .detach(|| {
                let samples = words.iter().map(|(word, like, pos)| {
                    let sample = match pos {
                        Some(pos) => rumorph::Sample::from((like.as_str(), pos.as_str())),
                        None => rumorph::Sample::from(like.as_str()),
                    };
                    (word.as_str(), sample)
                });
                base.base.dictionary.clone().with_words(samples)
            })
            .map_err(|err| PyValueError::new_err(err.to_string()))?;
        let object = Some(PyString::new(py, &text).unbind());
        let (shared, words) = (
            Arc::new(Shared::new(dictionary, &base.path)),
            Arc::new(OwnWords { text, object }),
        );
        // Прежняя последняя копия освобождается после снятия блокировки
        let previous = {
            let mut copies = copies();
            copies
                .entries
                .push((Arc::clone(&words), Arc::downgrade(&shared)));
            copies.last.replace(Arc::clone(&shared))
        };
        drop(previous);
        Ok(Analyzer {
            shared,
            words,
            ..base
        })
    }

    /// Ручка анализатора для разборов этого потока
    fn handle(slf: &Bound<'_, Self>) -> Arc<Handle> {
        let address = slf.as_ptr() as usize;
        let new = || {
            Arc::new(Handle {
                analyzer: slf.clone().unbind(),
            })
        };
        // В завершающемся потоке ручек уже нет: ручка создается без запоминания
        HANDLES
            .try_with(|handles| {
                let mut handles = handles.borrow_mut();
                let found = handles
                    .slots
                    .iter()
                    .filter(|(known, _)| *known == address)
                    .find_map(|(_, handle)| handle.upgrade());
                if let Some(handle) = found {
                    return handle;
                }
                let slot = match handles
                    .slots
                    .iter()
                    .position(|(_, handle)| handle.strong_count() == 0)
                {
                    Some(slot) => slot,
                    None => {
                        let slot = handles.next;
                        handles.next = (slot + 1) % HANDLE_SLOTS;
                        slot
                    }
                };
                let handle = new();
                handles.slots[slot] = (address, Arc::downgrade(&handle));
                handle
            })
            .unwrap_or_else(|_| new())
    }

    fn to_py(
        py: Python<'_>,
        handle: &Arc<Handle>,
        analysis: rumorph::Analysis,
    ) -> PyResult<Analysis> {
        let shared = &handle.analyzer.get().shared;
        let missing = || PyValueError::new_err("Нет тега UD");
        let cell = shared
            .ud
            .get(usize::from(analysis.ud))
            .ok_or_else(missing)?;
        once(cell, || {
            UdObjects::from_tag(
                py,
                shared.dictionary.ud_tag(analysis.ud).ok_or_else(missing)?,
            )
            .map(Box::new)
        })?;
        Ok(Analysis {
            inner: analysis,
            handle: Arc::clone(handle),
        })
    }
}

/// Значение ячейки, созданное при первом обращении. Значение создается до записи в ячейку,
/// и поток, в отличие от `PyOnceLock`, не ждет Python, заняв ее: fork, заставший такое ожидание,
/// оставил бы ячейку занятой навсегда, и дочерний процесс завис бы на ней. Если два потока создали значение
/// одновременно, лишнее освобождается здесь, в потоке Python
fn once<T>(cell: &OnceLock<T>, init: impl FnOnce() -> PyResult<T>) -> PyResult<&T> {
    if let Some(value) = cell.get() {
        return Ok(value);
    }
    let value = init()?;
    Ok(cell.get_or_init(move || value))
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
    let iterator = words.try_iter().map_err(|err| {
        if err.is_instance_of::<PyTypeError>(words.py()) {
            type_error("Ожидается набор слов", words)
        } else {
            err
        }
    })?;
    iterator
        .enumerate()
        .map(|(number, word)| {
            let word = word?;
            let Ok(word) = word.cast::<PyString>() else {
                return Err(PyTypeError::new_err(format!(
                    "Каждое слово должно быть строкой: элемент {number} - {}",
                    word.get_type().name()?
                )));
            };
            lossy(word)
        })
        .collect()
}

/// Строка Python как строка Rust: непарный суррогат (`surrogateescape`, `os.fsdecode`) - U+FFFD
fn lossy(word: &Bound<'_, PyString>) -> PyResult<String> {
    if let Ok(text) = word.to_str() {
        return Ok(text.to_owned());
    }
    let encoded = word.call_method1("encode", ("utf-16-le", "surrogatepass"))?;
    let bytes = encoded.extract::<Vec<u8>>()?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .collect();
    Ok(String::from_utf16_lossy(&units))
}

/// Слово как строка Rust; непарные суррогаты заменяются на U+FFFD
fn text(word: &Bound<'_, PyAny>) -> PyResult<String> {
    let Ok(word) = word.cast::<PyString>() else {
        return Err(type_error("Слово должно быть строкой", word));
    };
    lossy(word)
}

/// Свои слова из словаря Python: слово - образец или пара «образец, часть речи»; непарные
/// суррогаты заменяются на U+FFFD, как в запросе
fn own_words(words: &Bound<'_, PyAny>) -> PyResult<Vec<OwnWord>> {
    let py = words.py();
    let not_mapping = || type_error("Свои слова должны быть словарем «слово: образец»", words);
    if words.is_instance_of::<PyString>() {
        return Err(not_mapping());
    }
    // Ошибка внутри items() своего Mapping - его собственная, а не «не словарь»
    let items = match words.getattr(intern!(py, "items")) {
        Ok(items) => items.call0()?,
        Err(err) if err.is_instance_of::<PyAttributeError>(py) => return Err(not_mapping()),
        Err(err) => return Err(err),
    };
    let iterator = items.try_iter().map_err(|err| {
        if err.is_instance_of::<PyTypeError>(py) {
            not_mapping()
        } else {
            err
        }
    })?;
    let mut result = Vec::new();
    for item in iterator {
        let item = item?;
        let pair = item.cast::<PyTuple>().ok().filter(|pair| pair.len() == 2);
        let Some(pair) = pair else {
            let message = "Элемент items() своих слов должен быть парой «слово, образец»";
            return Err(match item.cast::<PyTuple>() {
                Ok(tuple) => PyTypeError::new_err(format!(
                    "{message}, а не кортежем из {} элементов",
                    tuple.len()
                )),
                Err(_) => type_error(message, &item),
            });
        };
        let (word, sample) = (pair.get_item(0)?, pair.get_item(1)?);
        let Ok(word) = word.cast::<PyString>() else {
            return Err(type_error("Свое слово должно быть строкой", &word));
        };
        let Some((like, pos)) = sample_strings(&sample) else {
            return Err(type_error(
                "Образец должен быть строкой или парой строк «образец, часть речи»",
                &sample,
            ));
        };
        let pos = pos.map(|pos| lossy(&pos)).transpose()?;
        result.push((lossy(word)?, lossy(&like)?, pos));
    }
    Ok(result)
}

/// Образец и часть речи: строка или пара строк
fn sample_strings<'py>(
    sample: &Bound<'py, PyAny>,
) -> Option<(Bound<'py, PyString>, Option<Bound<'py, PyString>>)> {
    if let Ok(like) = sample.cast::<PyString>() {
        return Some((like.clone(), None));
    }
    let pair = sample
        .cast::<PyTuple>()
        .ok()
        .filter(|pair| pair.len() == 2)?;
    let like = pair.get_item(0).ok()?.cast_into::<PyString>().ok()?;
    let pos = pair.get_item(1).ok()?.cast_into::<PyString>().ok()?;
    Some((like, Some(pos)))
}

/// TypeError с типом полученного значения: «Слово должно быть строкой, а не int»
fn type_error(message: &str, value: &Bound<'_, PyAny>) -> PyErr {
    let name = value
        .get_type()
        .name()
        .map_or_else(|_| "?".into(), |name| name.to_string());
    PyTypeError::new_err(format!("{message}, а не {name}"))
}

/// Количество для `agree`: целое - всё, у чего есть `__index__` (int, bool, numpy.int64),
/// и `Decimal` или `Fraction` без дробной части, дробное (`None`) - остальные float, Decimal
/// и Fraction. Согласование зависит от двух последних цифр и от того, меньше ли количество 5,
/// поэтому числа больше u64 сводятся к 100-199
fn count(number: &Bound<'_, PyAny>) -> PyResult<Option<u64>> {
    static INDEX: OnceLock<Py<PyAny>> = OnceLock::new();
    let py = number.py();
    let integer = if number.is_instance_of::<PyInt>() {
        number.clone()
    } else {
        let index = once(&INDEX, || {
            PyResult::Ok(py.import("operator")?.getattr("index")?.unbind())
        })?;
        match index.bind(py).call1((number,)) {
            Ok(integer) => integer,
            Err(err) if err.is_instance_of::<PyTypeError>(py) => match integral(number)? {
                Some(integer) => integer,
                None => return Ok(None),
            },
            Err(err) => return Err(err),
        }
    };
    Ok(Some(match integer.extract::<i128>() {
        Ok(value) => {
            let count = value.unsigned_abs();
            u64::try_from(count).unwrap_or(100 + (count % 100) as u64)
        }
        Err(err) if err.is_instance_of::<PyOverflowError>(py) => {
            100 + integer.abs()?.rem(100)?.extract::<u64>()?
        }
        Err(err) => return Err(err),
    }))
}

/// Целое значение количества без `__index__`: `Decimal` без знаков после запятой («21», но не
/// «21.0») и `Fraction` со знаменателем 1; `None` - дробное количество. Конечность проверяется
/// без перевода в float: «1E+400» - конечное целое
fn integral<'py>(number: &Bound<'py, PyAny>) -> PyResult<Option<Bound<'py, PyAny>>> {
    static TYPES: OnceLock<(Py<PyAny>, Py<PyAny>, Py<PyAny>)> = OnceLock::new();
    let py = number.py();
    let (decimal, rational, real) = once(&TYPES, || {
        let numbers = py.import("numbers")?;
        PyResult::Ok((
            py.import("decimal")?.getattr("Decimal")?.unbind(),
            numbers.getattr("Rational")?.unbind(),
            numbers.getattr("Real")?.unbind(),
        ))
    })?;
    let infinite = || {
        PyValueError::new_err(format!(
            "Количество должно быть конечным числом, а не {}",
            number
                .str()
                .map_or_else(|_| "?".into(), |text| text.to_string())
        ))
    };
    if number.is_instance(decimal.bind(py))? {
        if !number.call_method0("is_finite")?.is_truthy()? {
            return Err(infinite());
        }
        let parts = number.call_method0("as_tuple")?;
        let exponent: i64 = parts.getattr("exponent")?.extract()?;
        if exponent < 0 {
            return Ok(None);
        }
        // Не через int: с большим порядком перевод квадратичный и держит GIL. Целое - цифры
        // коэффициента и `exponent` нулей, согласованию нужны две последние цифры и то,
        // меньше ли количество 100
        let digits = parts.getattr("digits")?.cast_into::<PyTuple>()?;
        let count = digits.len();
        let digit = |from_end: usize| -> PyResult<u64> {
            if from_end < count {
                digits.get_item(count - 1 - from_end)?.extract()
            } else {
                Ok(0)
            }
        };
        let (tens, ones) = match exponent {
            0 => (digit(1)?, digit(0)?),
            1 => (digit(0)?, 0),
            _ => (0, 0),
        };
        let last = tens * 10 + ones;
        let value = if count == 1 && digit(0)? == 0 {
            0
        } else if count as i64 + exponent > 2 {
            100 + last
        } else {
            last
        };
        return Ok(Some(PyInt::new(py, value).into_any()));
    }
    if number.is_instance(rational.bind(py))? {
        return if number.getattr("denominator")?.eq(1)? {
            number.getattr("numerator").map(Some)
        } else {
            Ok(None)
        };
    }
    if !number.is_instance(real.bind(py))? {
        return Err(type_error("Количество должно быть числом", number));
    }
    // Число, которое не помещается во float, - конечное
    match number
        .call_method0("__float__")
        .and_then(|value| value.extract::<f64>())
    {
        Ok(value) if !value.is_finite() => Err(infinite()),
        Ok(_) => Ok(None),
        Err(err) if err.is_instance_of::<PyOverflowError>(py) => Ok(None),
        Err(err) => Err(err),
    }
}

#[pymodule]
fn _rumorph(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Analyzer>()?;
    m.add_class::<Analysis>()?;
    m.add("FormNotFoundError", m.py().get_type::<FormNotFoundError>())?;
    m.add_function(wrap_pyfunction!(_analysis, m)?)?;
    m.add_function(wrap_pyfunction!(_analyzer, m)?)?;
    m.add_function(wrap_pyfunction!(_main, m)?)?;
    let os = m.py().import("os")?;
    // На Windows fork нет
    if os.hasattr("register_at_fork")? {
        let handlers = PyDict::new(m.py());
        handlers.set_item("before", wrap_pyfunction!(_before_fork, m)?)?;
        handlers.set_item(
            "after_in_parent",
            wrap_pyfunction!(_after_fork_in_parent, m)?,
        )?;
        handlers.set_item("after_in_child", wrap_pyfunction!(_after_fork_in_child, m)?)?;
        os.call_method("register_at_fork", (), Some(&handlers))?;
    }
    m.add("__version__", package_version())?;
    let sys = m.py().import("sys")?;
    if let Ok(enabled) = sys.getattr("_is_gil_enabled") {
        GIL_DISABLED.store(!enabled.call0()?.is_truthy()?, Ordering::Relaxed);
    }
    Ok(())
}
