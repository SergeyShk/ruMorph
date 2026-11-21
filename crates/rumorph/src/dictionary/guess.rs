//! Угадывание незнакомых слов по окончанию: формы словарных слов с тем же окончанием и их
//! таблицы словоизменения; разборы строчного слова среди них выбирает модель класса парадигмы.
//! Слово с заглавной буквы угадывается в основном как имя собственное

use super::paradigm::{Features, Paradigms};
use super::{Analysis, Dictionary, Entry, Source};

/// Индекс угадывания незнакомых слов по окончанию
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum GuessIndex {
    /// Нарицательные слова
    Common,
    /// Имена собственные: для слов с заглавной буквы
    Proper,
}

impl GuessIndex {
    /// Наибольшая длина окончания в буквах
    pub const MAX_ENDING: usize = 5;

    /// Сколько раз окончание должно встретиться в словаре, чтобы попасть в индекс угадывания
    pub const MIN_COUNT: u32 = 3;

    /// Первый байт ключа окончания в автомате угадывания
    pub(crate) fn byte(self) -> u8 {
        match self {
            GuessIndex::Common => 0,
            GuessIndex::Proper => 1,
        }
    }
}

/// Форма таблицы из списков окончаний незнакомого слова
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Candidate {
    pub entry: Entry,
    /// Длина самого длинного окончания слова, в списке которого есть форма
    pub length: u8,
    /// Число словоформ словаря с этим окончанием и формой
    pub count: u32,
    /// Доля формы в списке этого окончания
    pub share: f32,
}

/// Тег UD и номера суффиксов формы леммы и формы кандидата
#[derive(Clone, Copy, Debug)]
struct Cell {
    ud: u16,
    lemma_suffix: usize,
    form_suffix: usize,
}

/// Кандидат примера обучения модели класса парадигмы: разбор, его признаки для модели и верен ли он
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct GuessCandidate {
    pub analysis: Analysis,
    pub lemma_suffix: String,
    pub form_suffix: String,
    pub length: u8,
    pub count: u32,
    pub share: f32,
    pub gold: bool,
}

/// Доля имен собственных среди незнакомых словарю слов с заглавной буквы в обучающих частях
/// UD Taiga и GSD
const PROPER_SHARE: f64 = 0.91;

/// Доля имен собственных среди незнакомых слов с заглавной буквы на -ский, -цкий и их формы
/// («Тильзитского», «Стэнфордская»): в UD Taiga и GSD имен среди них меньше, чем среди прочих
/// слов с заглавной буквы (0,62 против 0,91), а доля прилагательных делится между несколькими их
/// разборами, поэтому здесь она ниже
const ADJECTIVAL_PROPER_SHARE: f64 = 0.25;

/// Число имен собственных (не аббревиатур, а в GSD, где Abbr почти не размечен, и не слов капсом)
/// в обучающих частях UD Taiga и GSD: у имени в словаре полная
/// парадигма множественного числа, а в текстах оно редкость, поэтому угаданная форма весит
/// по частоте своего числа («Шатов» - фамилия, а не «Шат» во множественном числе). Падеж
/// остается за частотами словаря: «Копытова» - чаще родительный, чем женская фамилия
const PROPER_NUMBER: &[(&str, f64)] = &[("Sing", 57779.0), ("Plur", 613.0)];

/// Разборов незнакомого слова по модели класса парадигмы - не больше стольких самых вероятных
const MAX_GUESSED: usize = 10;

/// Без модели (имена собственные, словари до формата 9) разборы берутся из стольких первых форм
/// списка окончания: список упорядочен по числу словоформ, а длиннее он для модели
const MAX_COUNTED: usize = 10;

/// Разбор по модели класса парадигмы с весом меньше этой доли веса лучшего отбрасывается: формы
/// из списков всех окончаний слова дают и почти невозможные разборы
const MIN_GUESSED_SHARE: f64 = 0.01;

/// Слова длиннее не угадываются: окончание у них ничего не говорит, а копии слова в разборах
/// дорого стоят
const MAX_GUESSED_LENGTH: usize = 100;

/// Наименьшая длина слова в смешанном регистре, которое угадывается как строчное: короче - чаще
/// единица измерения
const MIN_MIXED_LENGTH: usize = 5;

/// Наименьшая длина угадываемого строчного слова: у слова из одной-двух букв («ви», «ся») окончание
/// ничего не говорит, оно разбирается по правилу; с заглавной («Рэ», «Ли») это обычно имя
const MIN_LOWERCASE_LENGTH: usize = 3;

impl Dictionary {
    /// Слово с заглавной буквы, все разборы которого - редких лексем дополнительного словаря
    /// (Викисловаря) без имени собственного, разбирается и как незнакомое имя собственное:
    /// в обучающих частях UD Taiga и GSD такие существительные с заглавной - имена в 87 % случаев,
    /// почти как незнакомые словарю («Сенна», «Робинзон»), а прилагательные - в 4 %
    /// («Двухместный»), и у них разборы не добавляются
    pub(super) fn add_rare_proper(&self, word: &str, analyses: &mut Vec<Analysis>) {
        let mut chars = word.chars();
        let titlecase = chars.next().is_some_and(char::is_uppercase)
            && chars.clone().next().is_some()
            && chars.all(char::is_lowercase);
        let proper = |analysis: &Analysis| {
            self.proper_nouns
                .get(usize::from(analysis.ud))
                .is_some_and(|&proper| proper)
        };
        let adjective = |analysis: &Analysis| {
            self.ud_tag(analysis.ud)
                .is_some_and(|tag| tag.pos() == "ADJ")
        };
        if !titlecase
            || analyses.iter().any(proper)
            || analyses.iter().all(adjective)
            || !self.rare_word(word)
        {
            return;
        }
        let lower = word.to_lowercase();
        let guessed = self.guess_with(&[(GuessIndex::Proper, 1.0)], word, &lower, None, false);
        if guessed.is_empty() {
            return;
        }
        let share = if adjectival(&lower) {
            ADJECTIVAL_PROPER_SHARE
        } else {
            PROPER_SHARE
        };
        for analysis in analyses.iter_mut() {
            analysis.prob *= 1.0 - share;
        }
        analyses.extend(guessed.into_iter().map(|analysis| Analysis {
            prob: analysis.prob * share,
            ..analysis
        }));
        analyses.sort_by(|a, b| b.prob.total_cmp(&a.prob));
    }

    /// Угаданные разборы незнакомого слова кириллицей в нижнем регистре или с заглавной буквы
    pub(super) fn guess(&self, word: &str) -> Vec<Analysis> {
        let chars: Vec<char> = word.chars().collect();
        if chars.is_empty()
            || chars.len() > MAX_GUESSED_LENGTH
            || !chars.iter().all(|&c| super::rules::cyrillic(c))
        {
            return Vec::new();
        }
        let lower = word.to_lowercase();
        // Аббревиатура с падежным окончанием («МЭРТа», «ЦУПе») угадывается, как слово с заглавной
        let capitals = chars.iter().take_while(|c| c.is_uppercase()).count();
        let abbreviation_with_ending = capitals >= 2
            && (1..=3).contains(&(chars.len() - capitals))
            && chars[capitals..].iter().all(|c| c.is_lowercase());
        let proper = if adjectival(&lower) {
            ADJECTIVAL_PROPER_SHARE
        } else {
            PROPER_SHARE
        };
        // Смешанный регистр: заглавные внутри строчного слова («прихАдите») и аббревиатура
        // с суффиксом («КВНщиками», «ЗОЖное», «НКФин»), если она не аббревиатура с падежным
        // окончанием, - как строчное слово, слитное название («ЛайфНьюс») - как слово с заглавной.
        // Короткое слово со строчной буквы в смешанном регистре - чаще единица измерения («дБ»,
        // «кГц»), как и короткая аббревиатура, у которой не нашлось окончания («МГц»)
        let lowercase = chars.iter().all(|c| c.is_lowercase());
        let mixed = !lowercase
            && !chars[1..].iter().all(|c| c.is_lowercase())
            && !chars.iter().all(|c| c.is_uppercase());
        let lowercase_tail = capitals >= 2 && chars[capitals..].iter().all(|c| c.is_lowercase());
        if lowercase || chars[0].is_lowercase() || (lowercase_tail && !abbreviation_with_ending) {
            if (mixed && chars.len() < MIN_MIXED_LENGTH)
                || (lowercase && chars.len() < MIN_LOWERCASE_LENGTH)
            {
                return Vec::new();
            }
            return self.guess_with(&[(GuessIndex::Common, 1.0)], word, &lower, None, true);
        }
        // Лемма аббревиатуры с окончанием - ее заглавная часть («КВНу» - «квн», а не «квна»),
        // а названия, которое кончается заглавной буквой, - все слово («РусАЛ», а не «русала»)
        let stem: Option<String> = if abbreviation_with_ending {
            Some(lower.chars().take(capitals).collect())
        } else {
            (mixed && chars[chars.len() - 1].is_uppercase()).then(|| lower.clone())
        };
        let title: &[(GuessIndex, f64)] = &[
            (GuessIndex::Proper, proper),
            (GuessIndex::Common, 1.0 - proper),
        ];
        // У слова с заглавной нарицательные разборы - малая доля, и модель их не выбирает: так
        // имена в тексте не платят за нее
        let analyses = self.guess_with(title, word, &lower, stem.as_deref(), false);
        if analyses.is_empty() && lowercase_tail && chars.len() >= MIN_MIXED_LENGTH {
            return self.guess_with(&[(GuessIndex::Common, 1.0)], word, &lower, None, true);
        }
        analyses
    }

    /// Разборы по индексам угадывания `indexes` с их долями; `model` - выбирать нарицательные
    /// разборы моделью класса парадигмы, если она есть в словаре
    fn guess_with(
        &self,
        indexes: &[(GuessIndex, f64)],
        word: &str,
        lower: &str,
        stem: Option<&str>,
        model: bool,
    ) -> Vec<Analysis> {
        let mut analyses = Vec::new();
        for &(index, share) in indexes {
            let found: Vec<(Analysis, f64)> = self
                .guess_in(index, word, lower, stem, model)
                .into_iter()
                .map(|(analysis, weight)| {
                    let number = match index {
                        GuessIndex::Proper => self.proper_weight(analysis.ud),
                        GuessIndex::Common => 1.0,
                    };
                    (analysis, weight * number)
                })
                .collect();
            // Формы с нулевым весом отброшены, поэтому сумма больше нуля
            let total: f64 = found.iter().map(|(_, weight)| weight).sum();
            analyses.extend(found.into_iter().map(|(analysis, weight)| Analysis {
                prob: share * weight / total,
                ..analysis
            }));
        }
        let total: f64 = analyses.iter().map(|analysis| analysis.prob).sum();
        for analysis in &mut analyses {
            analysis.prob /= total;
        }
        let analyses = self.merge(analyses);
        // Первой идет лексема с наибольшей суммой долей: у «манжетик» три формы лексемы «манжетик»
        // вместе вероятнее одной формы «манжетика»
        let pos = |analysis: &Analysis| self.ud_tag(analysis.ud).map(|tag| tag.pos());
        let mass: Vec<f64> = (analyses.iter())
            .map(|analysis| {
                (analyses.iter())
                    .filter(|other| other.lemma == analysis.lemma && pos(other) == pos(analysis))
                    .map(|other| other.prob)
                    .sum()
            })
            .collect();
        let mut order: Vec<usize> = (0..analyses.len()).collect();
        order.sort_by(|&a, &b| mass[b].total_cmp(&mass[a]));
        let mut slots: Vec<Option<Analysis>> = analyses.into_iter().map(Some).collect();
        order
            .into_iter()
            .filter_map(|at| slots[at].take())
            .collect()
    }

    /// Разборы незнакомого слова в индексе `index` с весами: нарицательные с `model` - по модели
    /// класса парадигмы, если она есть в словаре, иначе по самому длинному окончанию слова, для
    /// которого в индексе есть
    /// подходящие формы, с числом словоформ словаря. Словоформа разбора - как в тексте, лемма -
    /// в нижнем регистре. Окончание через «е» находит и формы с «ё»: «ё» суффикса формы
    /// подставляется в словоформу. С основой `stem` берутся только разборы с леммой, равной ей
    fn guess_in(
        &self,
        index: GuessIndex,
        word: &str,
        lower: &str,
        stem: Option<&str>,
        model: bool,
    ) -> Vec<(Analysis, f64)> {
        // Имена собственные модель не выбирает: в текстах это чаще иностранные имена
        // и несклоняемые названия, каких в словаре мало
        if model
            && index == GuessIndex::Common
            && let Ok(Some(model)) = self.paradigm_model()
        {
            return self.guess_by_model(model, word, lower, stem);
        }
        let bounds: Vec<usize> = lower.char_indices().map(|(at, _)| at).collect();
        for length in (1..=GuessIndex::MAX_ENDING.min(bounds.len().saturating_sub(1))).rev() {
            let ending = &lower[bounds[bounds.len() - length]..];
            let Some(forms) = self
                .alphabet
                .key(index.byte(), ending)
                .and_then(|key| self.guesses.get(key.bytes()))
                .and_then(|offset| self.guess_records_at(offset))
            else {
                continue;
            };
            let found: Vec<(Analysis, f64)> = forms
                .iter()
                .take(MAX_COUNTED)
                .filter(|&&[_, count]| count > 0)
                .filter_map(|&[packed, count]| {
                    let entry = unpack(packed);
                    Some((self.guessed(entry, word, lower, stem)?, f64::from(count)))
                })
                .collect();
            if !found.is_empty() {
                return found;
            }
        }
        Vec::new()
    }

    /// Разборы по модели класса парадигмы: формы из списков всех окончаний слова, не больше
    /// `MAX_GUESSED` самых вероятных, с весами модели
    fn guess_by_model(
        &self,
        model: &Paradigms,
        word: &str,
        lower: &str,
        stem: Option<&str>,
    ) -> Vec<(Analysis, f64)> {
        let found: Vec<(Candidate, Cell)> = self
            .guess_candidates(GuessIndex::Common, lower, &[])
            .into_iter()
            .filter_map(|candidate| {
                Some((candidate, self.guess_cell(candidate.entry, lower, stem)?))
            })
            .collect();
        let Some(longest) = found.iter().map(|(candidate, _)| candidate.length).max() else {
            return Vec::new();
        };
        let vector = model.word(lower);
        let scores: Vec<f64> = found
            .iter()
            .map(|(candidate, cell)| {
                let features = Features {
                    ud: cell.ud,
                    lemma_suffix: cell.lemma_suffix,
                    form_suffix: cell.form_suffix,
                    length: candidate.length,
                    count: candidate.count,
                    share: candidate.share,
                    longest: candidate.length == longest,
                };
                f64::from(model.score(&vector, &features))
            })
            .collect();
        let top = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        // Формы одной таблицы с одним суффиксом - одна лексема с одной основой. Модель выбирает
        // лексему, а совпадающие формы («щусевского» - мужской или средний род) различает
        // произвольно: в обучении верны все. Вес лексемы делится по числу словоформ, как без модели
        let lexeme = |at: usize| (found[at].0.entry.table, found[at].1.form_suffix);
        let mut order: Vec<usize> = (0..found.len()).collect();
        order.sort_by_key(|&at| lexeme(at));
        let mut weights = vec![0.0; found.len()];
        for group in order.chunk_by(|&a, &b| lexeme(a) == lexeme(b)) {
            let mass: f64 = group.iter().map(|&at| (scores[at] - top).exp()).sum();
            let count: f64 = group.iter().map(|&at| f64::from(found[at].0.count)).sum();
            for &at in group {
                weights[at] = mass * f64::from(found[at].0.count) / count;
            }
        }
        // Разборы строятся только для самых вероятных; при равных весах - порядок списков
        order.sort_unstable();
        order.sort_by(|&a, &b| weights[b].total_cmp(&weights[a]));
        let least = weights[order[0]] * MIN_GUESSED_SHARE;
        order
            .into_iter()
            .take(MAX_GUESSED)
            .take_while(|&at| weights[at] >= least)
            .filter_map(|at| {
                let analysis = self.guessed(found[at].0.entry, word, lower, stem)?;
                Some((analysis, weights[at]))
            })
            .collect()
    }

    /// Тег UD и номера суффиксов формы леммы и формы `entry` для незнакомого слова `lower` - без
    /// строк, которые строит `guessed`; `None`, если `guessed` не дал бы разбора
    fn guess_cell(&self, entry: Entry, lower: &str, stem: Option<&str>) -> Option<Cell> {
        let lexeme = self.table_lexeme(entry.table, "")?;
        if usize::from(entry.form) >= lexeme.cells.len() {
            return None;
        }
        let (prefix, form_suffix) = lexeme.affixes(entry.form);
        let (prefix, suffix) = (self.prefixes.at(prefix), self.suffixes.at(form_suffix));
        let tail = lower.get(lower.len().checked_sub(suffix.len())?..)?;
        let stem_end = lower.len() - suffix.len();
        if !ends_like(tail, suffix) || !lower.starts_with(prefix) || prefix.len() > stem_end {
            return None;
        }
        let word_stem = &lower[prefix.len()..stem_end];
        let (lemma_prefix, lemma_suffix) = lexeme.affixes(lexeme.lemma_form(entry.form));
        let (lemma_prefix_text, lemma_suffix_text) = (
            self.prefixes.at(lemma_prefix),
            self.suffixes.at(lemma_suffix),
        );
        let valid = match stem {
            Some(stem) => {
                stem.strip_prefix(lemma_prefix_text)
                    .and_then(|rest| rest.strip_suffix(lemma_suffix_text))
                    == Some(word_stem)
            }
            None => [lemma_prefix_text, word_stem, lemma_suffix_text]
                .iter()
                .any(|part| part.chars().any(vowel)),
        };
        valid.then_some(Cell {
            ud: lexeme.ud(entry.form),
            lemma_suffix,
            form_suffix,
        })
    }

    /// Разбор незнакомого слова как формы `entry`; `None`, если слово не кончается ее суффиксом
    /// или основа - не образец. С основой `stem` берется только разбор с леммой, равной ей
    fn guessed(
        &self,
        entry: Entry,
        word: &str,
        lower: &str,
        stem: Option<&str>,
    ) -> Option<Analysis> {
        let Entry { table, form } = entry;
        let (word, lower) = self.with_suffix(table, form, word, lower)?;
        let lexeme = self.lexeme(table, form, &lower)?;
        let lemma = self.spell(&lexeme, lexeme.lemma_form(form));
        // Основа без гласной («тов» - «т», «Лем» - «лй») - не образец, а совпадение
        match stem {
            Some(stem) if lemma != stem => return None,
            None if !lemma.chars().any(vowel) => return None,
            _ => {}
        }
        Some(Analysis {
            word,
            lemma,
            ud: lexeme.ud(form),
            tag: Some(lexeme.tag(form)),
            prob: 0.0,
            source: Source::Guess,
            entry: Some(entry),
        })
    }

    /// Формы таблиц из списков всех окончаний слова `lower` в индексе `index`, от самого длинного
    /// окончания; форма берется из самого длинного, в списке которого она есть. Формы `exclude` -
    /// разборы словарного слова: словоформа прибавила единицу к своей форме у каждого своего
    /// окончания, и без этих единиц списки такие, как если бы слова не было в словаре
    pub(super) fn guess_candidates(
        &self,
        index: GuessIndex,
        lower: &str,
        exclude: &[Entry],
    ) -> Vec<Candidate> {
        let bounds: Vec<usize> = lower.char_indices().map(|(at, _)| at).collect();
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut list: Vec<(Entry, u32)> = Vec::new();
        for length in (1..=GuessIndex::MAX_ENDING.min(bounds.len().saturating_sub(1))).rev() {
            let ending = &lower[bounds[bounds.len() - length]..];
            let Some(forms) = self
                .alphabet
                .key(index.byte(), ending)
                .and_then(|key| self.guesses.get(key.bytes()))
                .and_then(|offset| self.guess_records_at(offset))
            else {
                continue;
            };
            list.clear();
            list.extend(forms.iter().filter_map(|&[packed, count]| {
                let entry = unpack(packed);
                let count = count - u32::from(exclude.contains(&entry)).min(count);
                (count > 0).then_some((entry, count))
            }));
            let total: u32 = list.iter().map(|&(_, count)| count).sum();
            if total < GuessIndex::MIN_COUNT {
                continue;
            }
            candidates.extend(list.iter().map(|&(entry, count)| Candidate {
                entry,
                length: length as u8,
                count,
                share: count as f32 / total as f32,
            }));
        }
        // Повтор формы из более короткого окончания отбрасывается: остается первое вхождение.
        // Ключ - форма и место в списке одним числом: так сортировка дешевле
        let mut keys: Vec<u64> = (candidates.iter().enumerate())
            .map(|(at, candidate)| {
                let Entry { table, form } = candidate.entry;
                (u64::from(table) << 48) | (u64::from(form) << 32) | at as u64
            })
            .collect();
        keys.sort_unstable();
        keys.dedup_by_key(|key| *key >> 32);
        let mut order: Vec<usize> = keys.iter().map(|&key| key as u32 as usize).collect();
        order.sort_unstable();
        order.into_iter().map(|at| candidates[at]).collect()
    }

    /// Примеры обучения модели класса парадигмы: кандидаты словарного слова в нижнем регистре
    /// среди нарицательных, как если бы слова не было в словаре, и верен ли каждый - та же форма
    /// таблицы или те же лемма и тег UD, что у разбора словаря
    #[doc(hidden)]
    pub fn guess_examples(&self, word: &str) -> Vec<GuessCandidate> {
        let own: Vec<Analysis> = self.candidates(word);
        let entries: Vec<Entry> = own.iter().filter_map(|analysis| analysis.entry).collect();
        self.guess_candidates(GuessIndex::Common, word, &entries)
            .into_iter()
            .filter_map(|candidate| {
                let analysis = self.guessed(candidate.entry, word, word, None)?;
                let gold = entries.contains(&candidate.entry)
                    || own
                        .iter()
                        .any(|known| known.ud == analysis.ud && known.lemma == analysis.lemma);
                let (lemma_suffix, form_suffix) = self.cell_suffixes(candidate.entry)?;
                Some(GuessCandidate {
                    analysis,
                    lemma_suffix: lemma_suffix.to_owned(),
                    form_suffix: form_suffix.to_owned(),
                    length: candidate.length,
                    count: candidate.count,
                    share: candidate.share,
                    gold,
                })
            })
            .collect()
    }

    /// Суффиксы формы леммы и самой формы `entry`
    fn cell_suffixes(&self, entry: Entry) -> Option<(&str, &str)> {
        let (lemma, form) = self.cell_suffix_numbers(entry)?;
        Some((self.suffixes.at(lemma), self.suffixes.at(form)))
    }

    /// Номера суффиксов формы леммы и самой формы `entry`
    fn cell_suffix_numbers(&self, entry: Entry) -> Option<(usize, usize)> {
        let lexeme = self.table_lexeme(entry.table, "")?;
        if usize::from(entry.form) >= lexeme.cells.len() {
            return None;
        }
        let suffix = |form: u16| lexeme.affixes(form).1;
        Some((suffix(lexeme.lemma_form(entry.form)), suffix(entry.form)))
    }

    /// Словоформа как в тексте и в нижнем регистре, в которых «е» на месте «ё» суффикса формы
    /// заменена на «ё»; `None`, если слово не кончается суффиксом формы
    fn with_suffix(
        &self,
        table: u16,
        form: u16,
        word: &str,
        lower: &str,
    ) -> Option<(String, String)> {
        let start = *self.table_offsets.get(usize::from(table))? as usize;
        let forms = self.paradigm(*self.tables().get(start)? as u16)?;
        if usize::from(form) >= forms.len() / 3 {
            return None;
        }
        let suffix = self.suffixes.at(usize::from(forms[usize::from(form)]));
        if lower.ends_with(suffix) {
            return Some((word.to_owned(), lower.to_owned()));
        }
        if !suffix.contains('ё') || !lower.ends_with(&suffix.replace('ё', "е")) {
            return None;
        }
        // «е» и «ё» занимают одинаково байт, поэтому граница суффикса та же; меняются только
        // «е» на местах «ё», регистр остальных букв сохраняется
        let replace = |text: &str| {
            let start = text.len() - suffix.len();
            let tail: String = text[start..]
                .chars()
                .zip(suffix.chars())
                .map(|pair| match pair {
                    ('е', 'ё') => 'ё',
                    ('Е', 'ё') => 'Ё',
                    (letter, _) => letter,
                })
                .collect();
            [&text[..start], &tail].concat()
        };
        Some((replace(word), replace(lower)))
    }

    /// Вес угаданного имени собственного по частоте его числа в корпусе
    fn proper_weight(&self, ud: u16) -> f64 {
        let number = self.feat(ud, "Number");
        PROPER_NUMBER
            .iter()
            .find(|&&(known, _)| Some(known) == number)
            .map_or(1.0, |&(_, count)| count)
    }

    /// Список форм для угадывания: пары `(таблица << 16) | форма`, число словоформ
    fn guess_records_at(&self, offset: u64) -> Option<&[[u32; 2]]> {
        let pairs = super::list(self.guess_records(), u32::try_from(offset).ok()?, 2)?;
        Some(pairs.as_chunks::<2>().0)
    }
}

/// Конец слова `tail` - суффикс формы: тот же или с «е» на всех местах «ё» суффикса, как
/// в `with_suffix`; длины в байтах у них равны
fn ends_like(tail: &str, suffix: &str) -> bool {
    if tail == suffix {
        return true;
    }
    let mut yo = false;
    for (letter, expected) in tail.chars().zip(suffix.chars()) {
        let expected = if expected == 'ё' {
            yo = true;
            'е'
        } else {
            expected
        };
        if letter != expected {
            return false;
        }
    }
    yo
}

/// Форма таблицы из записи списка окончания: `(таблица << 16) | форма`
fn unpack(packed: u32) -> Entry {
    Entry {
        table: (packed >> 16) as u16,
        form: packed as u16,
    }
}

fn vowel(c: char) -> bool {
    "аеёиоуыэюя".contains(c)
}

/// Слово на -ский, -цкий или их форму: у слова с заглавной буквы это чаще прилагательное
fn adjectival(lower: &str) -> bool {
    const ENDINGS: &[&str] = &[
        "ий", "ая", "ое", "ие", "ого", "ому", "им", "ом", "ой", "ую", "их", "ими", "ей", "ою",
    ];
    ENDINGS.iter().any(|ending| {
        lower
            .strip_suffix(ending)
            .is_some_and(|stem| stem.ends_with("ск") || stem.ends_with("цк"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endings_with_yo() {
        assert!(ends_like("ём", "ём") && ends_like("ем", "ём"));
        assert!(!ends_like("ам", "ём") && !ends_like("ем", "ем ") && !ends_like("ём", "ем"));
        assert!(ends_like("еле", "ёле") && !ends_like("ёле", "ёлё"));
    }

    #[test]
    fn adjectival_endings() {
        for word in ["тильзитского", "стэнфордская", "бельгийские"]
        {
            assert!(adjectival(word), "{word}");
        }
        for word in ["шатов", "москва", "киска"] {
            assert!(!adjectival(word), "{word}");
        }
    }
}
