//! Модуль Python `rumorph._rumorph`

use std::cell::{RefCell, UnsafeCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::SystemTime;

use pyo3::create_exception;
use pyo3::exceptions::{PyLookupError, PyOSError, PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyFrozenSet, PyInt, PyString, PyTuple};

/// Сколько слов `analyze_many` и `lemmatize` разбирают за раз без GIL: между кусками
/// переводятся результаты и проверяется Ctrl+C
const CHUNK: usize = 8192;

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
    #[pyo3(get)]
    pos: Py<PyString>,
    feats: Py<PyDict>,
    #[pyo3(get)]
    labels: Py<PyFrozenSet>,
    ud: Arc<UdText>,
    handle: Arc<Handle>,
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

    /// Разборы равны при равных словоформе, лемме, части речи, признаках, пометах и источнике:
    /// вероятность зависит от остальных разборов словоформы, поэтому разбор равен своей форме
    fn __eq__(&self, other: &Bound<'_, Self>) -> bool {
        let other = other.get();
        self.inner.word == other.inner.word
            && self.inner.lemma == other.inner.lemma
            && self.inner.source == other.inner.source
            && self.ud == other.ud
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        (
            &self.inner.word,
            &self.inner.lemma,
            self.inner.source.as_str(),
            &self.ud,
        )
            .hash(&mut hasher);
        hasher.finish()
    }

    fn __copy__(slf: Bound<'_, Self>) -> Bound<'_, Self> {
        slf
    }

    fn __deepcopy__<'py>(slf: Bound<'py, Self>, _memo: &Bound<'py, PyAny>) -> Bound<'py, Self> {
        slf
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
            self.inner.entry.map(|entry| (entry.table, entry.form)),
        );
        Ok((restore, (self.handle.analyzer.clone_ref(py), state)))
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
    Option<(u16, u16)>,
);

/// Восстановление разбора из pickle: по сохраненным таблице и форме, если словарь тот же,
/// иначе - среди разборов слова с теми же леммой, частью речи, признаками и пометами;
/// вероятность берется сохраненная
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
    let same = |analysis: &rumorph::Analysis| {
        dictionary.ud_tag(analysis.ud).is_some_and(|tag| {
            analysis.word == word
                && analysis.source == source
                && analysis.lemma == lemma
                && tag.pos() == pos
                && tag.feats_string() == feats
                && tag.labels().collect::<Vec<_>>().join(",") == labels
        })
    };
    let found = entry
        .and_then(|(table, form)| {
            dictionary.analysis(&word, rumorph::Entry { table, form }, source)
        })
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
    Analyzer::to_py(py, &Analyzer::handle(analyzer.bind(py)), inner)
}

/// Словарь и объекты его тегов UD: общие для анализаторов с одним файлом словаря
struct Shared {
    dictionary: rumorph::Dictionary,
    /// Объекты тегов UD создаются при первом разборе с тегом: в тексте встречается малая доля тегов
    ud: Vec<OnceLock<UdObjects>>,
}

impl Shared {
    fn new(dictionary: rumorph::Dictionary) -> Self {
        let ud = dictionary.ud_tags().map(|_| OnceLock::new()).collect();
        Shared { dictionary, ud }
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
    entries: UnsafeCell<Vec<(FileKey, Slot)>>,
    last: UnsafeCell<Option<Arc<Shared>>>,
}

/// Словарь в списке загруженных
enum Slot {
    /// Словарь загружает другой поток: его ждут, а не загружают второй раз
    Loading,
    Loaded(Weak<Shared>),
}

// SAFETY: `entries` и `last` читаются и меняются только под флагом `locked`
unsafe impl Sync for Dictionaries {}

static DICTIONARIES: Dictionaries = Dictionaries {
    locked: AtomicBool::new(false),
    entries: UnsafeCell::new(Vec::new()),
    last: UnsafeCell::new(None),
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

    /// Действие со списком словарей под флагом
    fn with<T>(&self, action: impl FnOnce(&mut Vec<(FileKey, Slot)>) -> T) -> T {
        struct Unlock<'a>(&'a Dictionaries);
        impl Drop for Unlock<'_> {
            fn drop(&mut self) {
                self.0.unlock();
            }
        }
        self.lock();
        let _unlock = Unlock(self);
        // SAFETY: флаг взят только этим потоком
        action(unsafe { &mut *self.entries.get() })
    }

    /// Запоминает последний полученный словарь; вывод - прежний, его освобождают уже без флага
    fn keep(&self, shared: &Arc<Shared>) -> Option<Arc<Shared>> {
        self.with(|_| {
            // SAFETY: флаг взят только этим потоком
            let last = unsafe { &mut *self.last.get() };
            if last
                .as_ref()
                .is_some_and(|known| Arc::ptr_eq(known, shared))
            {
                return None;
            }
            last.replace(Arc::clone(shared))
        })
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
    DICTIONARIES.with(|entries| entries.retain(|(_, slot)| matches!(slot, Slot::Loaded(_))));
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
        DICTIONARIES.with(|entries| {
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
    let previous = DICTIONARIES.keep(&shared);
    Ok((shared, previous))
}

fn find_or_load(path: &Path) -> Result<Arc<Shared>, rumorph::Error> {
    let Some(key) = FileKey::new(path, &std::fs::metadata(path)?) else {
        return Ok(Arc::new(Shared::new(rumorph::Dictionary::open(path)?)));
    };
    loop {
        let lookup = DICTIONARIES.with(|entries| {
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
                let shared = Arc::new(Shared::new(rumorph::Dictionary::open(path)?));
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

/// Восстановление анализатора из pickle: словарь берется из загруженных в процессе
#[pyfunction]
fn _analyzer(py: Python<'_>, path: Option<PathBuf>) -> PyResult<Analyzer> {
    Analyzer::open(py, path)
}

/// Морфологический анализатор
///
/// Описание:
///     Разбирает словоформы по словарю OpenCorpora: лемма, часть речи и признаки UD,
///     вероятность. Регистр не важен, буква «е» в слове может обозначать «ё». Словарь
///     загружается один раз при создании анализатора; анализатор можно использовать
///     из нескольких потоков. Файл словаря отображается в память и общий для процессов:
///     пока он открыт, его нельзя перезаписывать на месте. Последний загруженный словарь
///     остается открытым и без анализаторов
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
    shared: Arc<Shared>,
    path: PathBuf,
    /// Путь задан явно: иначе pickle не сохраняет путь, и в другом окружении берется словарь его пакета
    explicit: bool,
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
    #[pyo3(signature = (path=None))]
    fn new(py: Python<'_>, path: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let path = match path {
            Some(path) if !path.is_none() => Some(path.extract::<PathBuf>().map_err(|_| {
                type_error("Путь к словарю должен быть строкой или os.PathLike", path)
            })?),
            _ => None,
        };
        Self::open(py, path)
    }

    fn __copy__(slf: Bound<'_, Self>) -> Bound<'_, Self> {
        slf
    }

    fn __deepcopy__<'py>(slf: Bound<'py, Self>, _memo: &Bound<'py, PyAny>) -> Bound<'py, Self> {
        slf
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
    /// Анализатор со словарем файла `path` (по умолчанию - словарем из пакета); путь хранится
    /// абсолютным, чтобы pickle работал в другом рабочем каталоге
    fn open(py: Python<'_>, path: Option<PathBuf>) -> PyResult<Self> {
        let explicit = path.is_some();
        let path = match path {
            Some(path) => std::path::absolute(&path).unwrap_or(path),
            None => default_path(py)?,
        };
        let (shared, previous) = py
            .detach(|| dictionary(&path))
            .map_err(|err| load_error(&path, err))?;
        drop(previous);
        Ok(Analyzer {
            shared,
            path,
            explicit,
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
        let this = handle.analyzer.get();
        let number = usize::from(analysis.ud);
        let shared = &this.shared;
        let ud = once(&shared.ud[number], || {
            let tag = shared.dictionary.ud_tag(analysis.ud);
            UdObjects::from_tag(py, tag.ok_or_else(|| PyValueError::new_err("Нет тега UD"))?)
        })?;
        Ok(Analysis {
            inner: analysis,
            pos: ud.pos.clone_ref(py),
            feats: ud.feats.clone_ref(py),
            labels: ud.labels.clone_ref(py),
            ud: Arc::clone(&ud.text),
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
    Ok(())
}
