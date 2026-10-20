//! Разбор по частям: слово через дефис - по последней части («бизнес-ланча»), слово с известной
//! приставкой - по слову без нее («экодомов»). Основа части продлевается первой частью или
//! приставкой, поэтому лемма и формы - у целого слова

use super::rules::cyrillic;
use super::{Analysis, Dictionary, Source};

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
                .all(|c| cyrillic(c) || c.is_ascii_digit() || c == '-')
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
        let analyses = self
            .analyze(right)
            .into_iter()
            .filter_map(|analysis| {
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
        normalized(analyses)
    }

    /// Разбор слова с частицей через дефис («смотри-ка») без частицы и частица в написании слова,
    /// если формы строятся по первой части, а не по всему слову, как у «кто-то»
    pub(super) fn without_particle(&self, analysis: &Analysis) -> Option<(Analysis, String)> {
        let entry = analysis.entry?;
        let (base, particle) = analysis.word.rsplit_once('-')?;
        let lower = particle.to_lowercase();
        if !PARTICLES.contains(&lower.as_str())
            || self
                .lexeme(entry.table, entry.form, &analysis.word.to_lowercase())
                .is_some()
        {
            return None;
        }
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
    fn yo_from_spelling() {
        assert_eq!(with_yo("желтый", "жёлтый"), "жёлтый");
        assert_eq!(with_yo("ЖЕЛТЫЙ", "жёлтый"), "ЖЁЛТЫЙ");
        assert_eq!(with_yo("кот", "кот"), "кот");
        assert_eq!(with_yo("кот", "котёнок"), "кот");
    }
}
