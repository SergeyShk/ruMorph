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
        let lower = word.to_lowercase();
        let (Some((left, right)), Some((left_lower, right_lower))) =
            (word.rsplit_once('-'), lower.rsplit_once('-'))
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
        {
            return Vec::new();
        }
        if PARTICLES.contains(&right_lower) {
            let analyses = self
                .analyze(left)
                .into_iter()
                .map(|analysis| Analysis {
                    word: word.to_owned(),
                    lemma: format!("{}-{right_lower}", analysis.lemma),
                    source: Source::Guess,
                    entry: None,
                    ..analysis
                })
                .collect();
            return normalized(analyses);
        }
        let analyses = self
            .analyze(right)
            .into_iter()
            .filter_map(|analysis| {
                let lemma = match analysis.entry {
                    Some(entry) => {
                        let lexeme = self.lexeme(entry.table, entry.form, &lower)?;
                        // Основа как в тексте нужна, чтобы формы сохраняли заглавные буквы
                        self.lexeme(entry.table, entry.form, word)?;
                        self.spell(&lexeme, lexeme.lemma_form(entry.form))
                    }
                    None => format!("{left_lower}-{}", analysis.lemma),
                };
                Some(Analysis {
                    word: word.to_owned(),
                    lemma,
                    source: Source::Guess,
                    ..analysis
                })
            })
            .collect();
        normalized(analyses)
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
                    OPEN_CLASSES.contains(&self.ud_tags[usize::from(analysis.ud)].pos.as_str())
                })
                .filter_map(|analysis| {
                    let entry = analysis.entry?;
                    let lexeme = self.lexeme(entry.table, entry.form, word)?;
                    Some(Analysis {
                        word: word.to_owned(),
                        lemma: self.spell(&lexeme, lexeme.lemma_form(entry.form)),
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

/// Разборы с вероятностями, нормированными к единице, по убыванию вероятности
fn normalized(mut analyses: Vec<Analysis>) -> Vec<Analysis> {
    let total: f64 = analyses.iter().map(|analysis| analysis.prob).sum();
    if total > 0.0 {
        for analysis in &mut analyses {
            analysis.prob /= total;
        }
    }
    analyses.sort_by(|a, b| b.prob.total_cmp(&a.prob));
    analyses
}
