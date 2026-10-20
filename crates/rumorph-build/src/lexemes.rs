//! Объединение лексем OpenCorpora с одной леммой UD («мешать» переходный и непереходный,
//! «брызгать» с формами «брызгаю» и «брызжу»): в словаре они становятся одной лексемой

/// Сколько самых частых форм таблиц хранится для окончания в индексе угадывания
pub const MAX_GUESSES: usize = 10;

/// Сколько раз окончание должно встретиться в словаре, чтобы попасть в индекс угадывания
pub const MIN_ENDING_COUNT: u32 = 3;

/// Форма лексемы: префикс, суффикс, тег OpenCorpora, форма леммы и тег UD
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub prefix: String,
    pub suffix: String,
    pub tag: u16,
    pub lemma_form: u16,
    pub ud: u16,
}

/// Лексема: основа и формы
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lexeme {
    pub stem: String,
    pub forms: Vec<Form>,
}

/// Лексема из лексем `lexemes` по порядку: основа - общее начало их основ, остаток основы
/// переходит в суффиксы, повторы форм убираются; вывод - лексема и номера форм каждой
/// исходной лексемы в ней
pub fn combine(lexemes: &[Lexeme]) -> (Lexeme, Vec<Vec<u16>>) {
    let stem: String = lexemes
        .iter()
        .map(|lexeme| lexeme.stem.as_str())
        .reduce(|common, stem| {
            let length: usize = common
                .chars()
                .zip(stem.chars())
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a.len_utf8())
                .sum();
            &common[..length]
        })
        .unwrap_or_default()
        .to_owned();
    let mut combined = Lexeme {
        stem: stem.clone(),
        forms: Vec::new(),
    };
    let mut numbers = Vec::with_capacity(lexemes.len());
    for lexeme in lexemes {
        let rest = &lexeme.stem[stem.len()..];
        let mapped: Vec<u16> = lexeme
            .forms
            .iter()
            .map(|form| {
                let suffix = [rest, form.suffix.as_str()].concat();
                let same = |known: &Form| {
                    known.prefix == form.prefix && known.suffix == suffix && known.tag == form.tag
                };
                match combined.forms.iter().position(same) {
                    Some(position) => position as u16,
                    None => {
                        combined.forms.push(Form {
                            suffix,
                            ..form.clone()
                        });
                        (combined.forms.len() - 1) as u16
                    }
                }
            })
            .collect();
        numbers.push(mapped);
    }
    // Формы леммы - номера форм исходной лексемы: переводим их в номера объединенной,
    // у общей формы - по первой лексеме
    let mut assigned = vec![false; combined.forms.len()];
    for (lexeme, mapped) in lexemes.iter().zip(&numbers) {
        for (form, &number) in lexeme.forms.iter().zip(mapped) {
            if !assigned[usize::from(number)] {
                assigned[usize::from(number)] = true;
                combined.forms[usize::from(number)].lemma_form =
                    mapped[usize::from(form.lemma_form)];
            }
        }
    }
    (combined, numbers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(suffix: &str, tag: u16, lemma_form: u16) -> Form {
        Form {
            prefix: String::new(),
            suffix: suffix.into(),
            tag,
            lemma_form,
            ud: tag,
        }
    }

    fn words(lexeme: &Lexeme) -> Vec<String> {
        lexeme
            .forms
            .iter()
            .map(|form| [form.prefix.as_str(), &lexeme.stem, &form.suffix].concat())
            .collect()
    }

    #[test]
    fn combines_stems_and_forms() {
        let first = Lexeme {
            stem: "брызга".into(),
            forms: vec![form("ть", 0, 0), form("ю", 1, 0), form("л", 2, 0)],
        };
        let second = Lexeme {
            stem: "брыз".into(),
            forms: vec![form("гать", 0, 0), form("жу", 1, 0), form("гал", 2, 0)],
        };
        let (combined, numbers) = combine(&[first.clone(), second.clone()]);
        assert_eq!(combined.stem, "брыз");
        assert_eq!(
            words(&combined),
            ["брызгать", "брызгаю", "брызгал", "брызжу"]
        );
        assert_eq!(numbers, [vec![0, 1, 2], vec![0, 3, 2]]);
        assert!(combined.forms.iter().all(|form| form.lemma_form == 0));
        // Каждая форма исходной лексемы пишется так же и с тем же тегом
        for (lexeme, mapped) in [first, second].iter().zip(&numbers) {
            for (source, &number) in words(lexeme).iter().zip(mapped) {
                assert_eq!(&words(&combined)[usize::from(number)], source);
            }
        }
    }

    #[test]
    fn keeps_lemma_forms_and_prefixes() {
        let first = Lexeme {
            stem: "больш".into(),
            forms: vec![
                form("ой", 0, 0),
                Form {
                    prefix: "наи".into(),
                    ..form("ий", 1, 1)
                },
            ],
        };
        let second = Lexeme {
            stem: "больш".into(),
            forms: vec![form("е", 2, 1), form("ой", 0, 1)],
        };
        let (combined, numbers) = combine(&[first, second]);
        assert_eq!(words(&combined), ["большой", "наибольший", "больше"]);
        assert_eq!(numbers, [vec![0, 1], vec![2, 0]]);
        let lemma_forms: Vec<u16> = combined.forms.iter().map(|form| form.lemma_form).collect();
        assert_eq!(lemma_forms, [0, 1, 0]);
    }
}
