//! Разбор по частям: слово через дефис - по последней части («бизнес-ланча»), слово с известной
//! приставкой - по слову без нее («экодомов»). Основа части продлевается первой частью или
//! приставкой, поэтому лемма и формы - у целого слова

use super::rules::cyrillic;
use super::{Analysis, Dictionary, Source, UdTag};

/// Частицы после дефиса: слово разбирается по первой части («сколь-нибудь», «смотри-ка»)
const PARTICLES: &[&str] = &["то", "либо", "нибудь", "ка", "таки", "де", "с"];

/// Приставки, с которыми незнакомое слово разбирается по словарному слову без приставки
const PREFIXES: &[&str] = &[
    "общеевро",
    "интернет",
    "электро",
    "гастро",
    "крипто",
    "псевдо",
    "стерео",
    "супер",
    "ультра",
    "аудио",
    "видео",
    "гидро",
    "гипер",
    "квази",
    "кибер",
    "контр",
    "макро",
    "макси",
    "мульти",
    "радио",
    "сверх",
    "термо",
    "транс",
    "турбо",
    "энерго",
    "авиа",
    "авто",
    "агро",
    "аква",
    "анти",
    "вело",
    "вице",
    "евро",
    "медиа",
    "мега",
    "мета",
    "микро",
    "мини",
    "мото",
    "нано",
    "недо",
    "пост",
    "пред",
    "теле",
    "фото",
    "экзо",
    "арт",
    "био",
    "гео",
    "лже",
    "нео",
    "пан",
    "пара",
    "пра",
    "про",
    "спец",
    "сюр",
    "эко",
    "экс",
    "ре",
];

/// Части речи последней части слова через дефис: служебные слова и буквы - не части сложного слова
const HYPHENATED_CLASSES: &[&str] = &["NOUN", "PROPN", "ADJ", "VERB", "ADV", "NUM", "INTJ", "X"];

/// Части речи, которые образуются с приставками
const OPEN_CLASSES: &[&str] = &["NOUN", "ADJ", "VERB", "ADV"];

/// Приставки, с которыми глагол меняет вид («платил» - «проплатил»): глаголы без них не берутся
const ASPECT_PREFIXES: &[&str] = &["про", "недо"];

impl Dictionary {
    /// Разборы незнакомого слова по частям; пустой список, если части не разобрались
    pub(super) fn parts(&self, word: &str) -> Vec<Analysis> {
        if word.contains('-') {
            self.hyphenated(word)
        } else {
            self.prefixed(word)
        }
    }

    /// Слово через дефис: разборы последней части с основой, продленной первой частью,
    /// или разборы первой части, если последняя - частица
    fn hyphenated(&self, word: &str) -> Vec<Analysis> {
        let lowered = word.to_lowercase();
        let (Some((left, right)), Some((left_lower, right_lower))) =
            (word.rsplit_once('-'), lowered.rsplit_once('-'))
        else {
            return Vec::new();
        };
        if left.is_empty()
            || left.starts_with('-')
            || left.ends_with('-')
            || !left
                .chars()
                .all(|c| cyrillic(c) || c.is_ascii_alphanumeric() || c == '-')
            || right.is_empty()
            || !right.chars().all(cyrillic)
            // Число с литерой («5-Б», «10-А») - не слово из частей
            || (left.bytes().all(|b| b.is_ascii_digit()) && right.chars().count() < 3)
        {
            return Vec::new();
        }
        if PARTICLES.contains(&right_lower) {
            // Частица одна: «кто-то-то» не разбирается, иначе цепочка частиц - глубокая рекурсия
            if left_lower
                .rsplit_once('-')
                .is_some_and(|(_, last)| PARTICLES.contains(&last))
            {
                return Vec::new();
            }
            // Разбор первой части с частицей: формы строятся по ней (`without_particle`)
            let analyses = self
                .analyze(left)
                .into_iter()
                .map(|analysis| Analysis {
                    word: word.to_owned(),
                    lemma: format!("{}-{right_lower}", analysis.lemma),
                    source: Source::Guess,
                    ..analysis
                })
                .collect();
            return normalized(self.merge(analyses));
        }
        // Протяжка («Что-о», «Да-а-а») - разбор слова без повторов последней буквы; в левой части
        // только кириллица, латиница и цифры, поэтому позиции в нижнем регистре те же
        let stretched = repeats(right_lower, left_lower);
        let mut end = left.len();
        while stretched
            && let Some((shorter, part)) = left_lower[..end].rsplit_once('-')
            && repeats(part, shorter)
        {
            end = shorter.len();
        }
        // Заикание («Н-нет», «Д-д-да») - разбор последней части
        let mut letters = left_lower.split('-');
        let stutter = letters.next().is_some_and(|first| {
            first.chars().count() == 1
                && letters.all(|letter| letter == first)
                && right_lower.starts_with(first)
        });
        if stretched || stutter {
            let part = if stretched { &left[..end] } else { right };
            // Часть с дефисом - только из словаря: иначе протяжка и частица («да-то-о-то-о»)
            // вызывают друг друга без конца
            let found = if part.contains('-') {
                self.merge(self.candidates(part))
            } else {
                self.analyze(part)
            };
            // Протянутая буква - междометие («А-а»), а не союз или сокращение («Ш-ш»)
            let letter = stretched && part.chars().count() == 1;
            let analyses = found
                .into_iter()
                .filter(|analysis| !letter || self.ud_tags[usize::from(analysis.ud)].pos == "INTJ")
                .map(|analysis| Analysis {
                    word: word.to_owned(),
                    source: Source::Guess,
                    entry: None,
                    ..analysis
                })
                .collect();
            return normalized(self.merge(analyses));
        }
        // Последняя часть - знаменательное слово, а не предлог или буква («К-за», «Коба-Т»)
        if right.chars().count() < 2 {
            return Vec::new();
        }
        let left_analyses = self.candidates(left);
        let analyses = self
            .analyze(right)
            .into_iter()
            .filter(|analysis| {
                HYPHENATED_CLASSES.contains(&self.ud_tags[usize::from(analysis.ud)].pos.as_str())
            })
            .filter_map(|analysis| {
                // Обе части изменяются («дивана-кровати», «думал-думал»): лемма - из лемм частей,
                // форм нет
                if let Some(first) = left_analyses
                    .iter()
                    .find(|first| self.inflected_together(first, &analysis))
                    .filter(|first| !self.rarer_than_lemma(first, &left_analyses))
                {
                    return Some(Analysis {
                        word: word.to_owned(),
                        lemma: format!("{}-{}", first.lemma, analysis.lemma),
                        source: Source::Guess,
                        entry: None,
                        ..analysis
                    });
                }
                // Последняя часть - в написании разбора: с «ё» из словаря («грязно-жёлтый»)
                let word = format!("{left}-{}", with_yo(right, &analysis.word));
                let lemma = match analysis.entry {
                    Some(entry) => {
                        let lower = word.to_lowercase();
                        let lexeme = self.lexeme(entry.table, entry.form, &lower)?;
                        self.spell(&lexeme, lexeme.lemma_form(entry.form))
                    }
                    None => format!("{left_lower}-{}", analysis.lemma),
                };
                Some(Analysis {
                    word,
                    lemma,
                    source: Source::Guess,
                    ..analysis
                })
            })
            .collect();
        normalized(self.merge(analyses))
    }

    /// Первая часть слова через дефис изменяется вместе с последней: та же часть речи, падеж
    /// и число (у глагола - форма, время, лицо, род и число), и это не начальная форма
    /// («дивана-кровати», но не «диван-кровать» и не «интернет-магазина»)
    fn inflected_together(&self, first: &Analysis, last: &Analysis) -> bool {
        let (first_tag, last_tag) = (
            &self.ud_tags[usize::from(first.ud)],
            &self.ud_tags[usize::from(last.ud)],
        );
        fn feat<'a>(tag: &'a UdTag, name: &str) -> Option<&'a str> {
            tag.feats
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.as_str())
        }
        let names: &[&str] = if feat(last_tag, "Case").is_some() {
            &["Case", "Number"]
        } else {
            &["VerbForm", "Tense", "Person", "Gender", "Number"]
        };
        first.lemma != first.word
            && first_tag.pos == last_tag.pos
            && matches!(first_tag.pos.as_str(), "NOUN" | "PROPN" | "ADJ" | "VERB")
            && names
                .iter()
                .all(|name| feat(first_tag, name) == feat(last_tag, name))
    }

    /// У первой части есть более вероятное нарицательное существительное в начальной форме:
    /// «поп» в «поп-песен» - «поп», а не «попа» в родительном множественного
    fn rarer_than_lemma(&self, first: &Analysis, left_analyses: &[Analysis]) -> bool {
        let noun = |analysis: &Analysis| self.ud_tags[usize::from(analysis.ud)].pos == "NOUN";
        noun(first)
            && left_analyses
                .iter()
                .any(|other| noun(other) && other.lemma == other.word && other.prob > first.prob)
    }

    /// Угаданный разбор слова с частицей через дефис («смотри-ка») без частицы и частица
    /// в написании слова: формы строятся по первой части. У словарных слов («кто-то») частица -
    /// часть основы
    pub(super) fn without_particle(&self, analysis: &Analysis) -> Option<(Analysis, String)> {
        if analysis.source != Source::Guess {
            return None;
        }
        let entry = analysis.entry?;
        let (base, particle) = split_particle(&analysis.word)?;
        let lower = particle.to_lowercase();
        self.lexeme(entry.table, entry.form, &base.to_lowercase())?;
        let lemma = analysis.lemma.strip_suffix(&lower)?.strip_suffix('-')?;
        let base = Analysis {
            word: base.to_owned(),
            lemma: lemma.to_owned(),
            ..analysis.clone()
        };
        Some((base, particle.to_owned()))
    }

    /// Слово в нижнем регистре с известной приставкой: разборы словарного слова без приставки,
    /// основа которых продлена приставкой
    fn prefixed(&self, word: &str) -> Vec<Analysis> {
        if !word.chars().all(|c| cyrillic(c) && c.is_lowercase()) {
            return Vec::new();
        }
        for prefix in PREFIXES {
            let Some(rest) = word.strip_prefix(prefix) else {
                continue;
            };
            if rest.chars().count() < 3 {
                continue;
            }
            let analyses: Vec<Analysis> = self
                .merge(self.candidates(rest))
                .into_iter()
                .filter(|analysis| {
                    let pos = self.ud_tags[usize::from(analysis.ud)].pos.as_str();
                    OPEN_CLASSES.contains(&pos)
                        && !(pos == "VERB" && ASPECT_PREFIXES.contains(prefix))
                })
                .filter_map(|analysis| {
                    let entry = analysis.entry?;
                    // Слово без приставки - в написании словаря: с «ё» («экс-королём»)
                    let word = format!("{prefix}{}", with_yo(rest, &analysis.word));
                    let lemma = {
                        let lexeme = self.lexeme(entry.table, entry.form, &word)?;
                        self.spell(&lexeme, lexeme.lemma_form(entry.form))
                    };
                    Some(Analysis {
                        word,
                        lemma,
                        source: Source::Guess,
                        ..analysis
                    })
                })
                .collect();
            if !analyses.is_empty() {
                return normalized(analyses);
            }
        }
        Vec::new()
    }
}

/// Разборы с вероятностями, нормированными к единице (при нулевой сумме - поровну),
/// по убыванию вероятности
fn normalized(mut analyses: Vec<Analysis>) -> Vec<Analysis> {
    let total: f64 = analyses.iter().map(|analysis| analysis.prob).sum();
    let count = analyses.len() as f64;
    for analysis in &mut analyses {
        analysis.prob = if total > 0.0 {
            analysis.prob / total
        } else {
            1.0 / count
        };
    }
    analyses.sort_by(|a, b| b.prob.total_cmp(&a.prob));
    analyses
}

/// Часть слова как в тексте, но с «ё» на местах «ё» в написании разбора
fn with_yo(text: &str, spelled: &str) -> String {
    if text.chars().count() != spelled.chars().count() {
        return text.to_owned();
    }
    text.chars()
        .zip(spelled.chars())
        .map(|pair| match pair {
            ('е', 'ё' | 'Ё') => 'ё',
            ('Е', 'ё' | 'Ё') => 'Ё',
            (letter, _) => letter,
        })
        .collect()
}

/// Часть `part` - повтор последней буквы `before`: «о» в «что-о»
fn repeats(part: &str, before: &str) -> bool {
    !part.is_empty()
        && before
            .chars()
            .last()
            .is_some_and(|last| part.chars().all(|c| c == last))
}

/// Слово с частицей через дефис: первая часть и частица в написании слова
pub(super) fn split_particle(word: &str) -> Option<(&str, &str)> {
    let (base, particle) = word.rsplit_once('-')?;
    PARTICLES
        .contains(&particle.to_lowercase().as_str())
        .then_some((base, particle))
}

/// Форма слова с частицей `particle` через дефис: «смотрите» - «смотрите-ка»
pub(super) fn with_particle(form: Analysis, particle: &str) -> Analysis {
    Analysis {
        word: format!("{}-{particle}", form.word),
        lemma: format!("{}-{}", form.lemma, particle.to_lowercase()),
        ..form
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_letters() {
        assert!(repeats("о", "что"));
        assert!(repeats("аа", "да"));
        assert!(!repeats("за", "к"));
        assert!(!repeats("", "да"));
    }

    #[test]
    fn yo_from_spelling() {
        assert_eq!(with_yo("желтый", "жёлтый"), "жёлтый");
        assert_eq!(with_yo("ЖЕЛТЫЙ", "жёлтый"), "ЖЁЛТЫЙ");
        assert_eq!(with_yo("кот", "кот"), "кот");
        assert_eq!(with_yo("кот", "котёнок"), "кот");
    }
}
