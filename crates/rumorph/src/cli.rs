//! Команда `rumorph`: морфологический анализ текста из файлов и стандартного ввода

use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::{Analysis, Dictionary};

const HELP: &str = "\
rumorph - морфологический анализ русского текста: леммы, части речи и признаки Universal Dependencies

Использование: rumorph [параметры] [файл ...]

Файлы читаются по очереди, без файлов и вместо «-» - стандартный ввод; вход - в UTF-8.
Результат пишется в стандартный вывод.

Параметры:
  -f, --format ФОРМАТ    формат вывода:
                           lemmas - леммы через пробел, строка на предложение (по умолчанию)
                           conllu - CoNLL-U: лемма, часть речи и признаки самого вероятного разбора
                           tsv - токен, лемма, часть речи, признаки и вероятность через табуляцию
                           jsonl - JSON Lines: предложение в строке, все разборы каждого токена
  -i, --input ВХОД       вход:
                           text - текст, предложение - строка (по умолчанию)
                           tokens - токен в строке, пустая строка разделяет предложения
                           conllu - CoNLL-U: заполняются LEMMA, UPOS и FEATS
  -d, --dictionary ФАЙЛ  файл словаря ruMorph (по умолчанию - словарь пакета)
  -h, --help             показать эту справку и выйти
  -V, --version          показать версию и выйти
";

/// Выполняет команду с аргументами `args` (без имени программы) и возвращает код выхода:
/// 0 - успех, 1 - ошибка чтения файла или словаря, 2 - неверные аргументы. `version` печатает
/// `--version`; `dictionary` - словарь по умолчанию, без него - словарь крейта `rumorph-dict-ru`
pub fn run(args: Vec<OsString>, version: &str, dictionary: Option<&Path>) -> u8 {
    let mut output = BufWriter::new(io::stdout().lock());
    execute(
        args,
        version,
        dictionary,
        &mut io::stdin().lock(),
        &mut output,
        &mut io::stderr(),
    )
}

/// Формат вывода
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Lemmas,
    Conllu,
    Tsv,
    Jsonl,
}

/// Вид входа
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Input {
    Text,
    Tokens,
    Conllu,
}

#[derive(Debug, PartialEq)]
enum Command {
    Help,
    Version,
    Run {
        format: Format,
        input: Input,
        dictionary: Option<PathBuf>,
        files: Vec<OsString>,
    },
}

fn parse(args: Vec<OsString>) -> Result<Command, String> {
    let (mut format, mut input, mut dictionary) = (Format::Lemmas, Input::Text, None);
    let mut files = Vec::new();
    let mut options = true;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let option = match arg.to_str() {
            Some(text) if options && text.starts_with('-') && text != "-" => text.to_owned(),
            _ => {
                files.push(arg);
                continue;
            }
        };
        let (name, inline) = match option.split_once('=') {
            Some((name, value)) if option.starts_with("--") => (name, Some(value)),
            _ if !option.starts_with("--") && option.len() > 2 => option
                .split_at_checked(2)
                .map_or((option.as_str(), None), |(n, v)| (n, Some(v))),
            _ => (option.as_str(), None),
        };
        let mut value = || -> Result<OsString, String> {
            match inline {
                Some(value) => Ok(value.into()),
                None => args
                    .next()
                    .ok_or_else(|| format!("параметру {name} нужно значение")),
            }
        };
        match name {
            "--" => options = false,
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-f" | "--format" => {
                format = match value()?.to_str() {
                    Some("lemmas") => Format::Lemmas,
                    Some("conllu") => Format::Conllu,
                    Some("tsv") => Format::Tsv,
                    Some("jsonl") => Format::Jsonl,
                    other => return Err(unknown(name, other, "lemmas, conllu, tsv, jsonl")),
                }
            }
            "-i" | "--input" => {
                input = match value()?.to_str() {
                    Some("text") => Input::Text,
                    Some("tokens") => Input::Tokens,
                    Some("conllu") => Input::Conllu,
                    other => return Err(unknown(name, other, "text, tokens, conllu")),
                }
            }
            "-d" | "--dictionary" => dictionary = Some(PathBuf::from(value()?)),
            _ => return Err(format!("неизвестный параметр {option}")),
        }
    }
    Ok(Command::Run {
        format,
        input,
        dictionary,
        files,
    })
}

fn unknown(name: &str, value: Option<&str>, known: &str) -> String {
    format!(
        "неизвестное значение параметра {name}: {} (допустимые: {known})",
        value.unwrap_or("?")
    )
}

fn execute(
    args: Vec<OsString>,
    version: &str,
    default: Option<&Path>,
    stdin: &mut dyn BufRead,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> u8 {
    // Сообщение в stderr: если и он закрыт, сказать уже некому
    let mut report = |message: String| drop(writeln!(stderr, "rumorph: {message}"));
    let (format, input, dictionary, files) = match parse(args) {
        Ok(Command::Run {
            format,
            input,
            dictionary,
            files,
        }) => (format, input, dictionary, files),
        Ok(Command::Help) => return finish(stdout.write_all(HELP.as_bytes())),
        Ok(Command::Version) => return finish(writeln!(stdout, "rumorph {version}")),
        Err(message) => {
            report(format!("{message}\nСправка: rumorph --help"));
            return 2;
        }
    };
    let dictionary = match load(dictionary.as_deref().or(default)) {
        Ok(dictionary) => dictionary,
        Err(message) => {
            report(message);
            return 1;
        }
    };
    let files = if files.is_empty() {
        vec![OsString::from("-")]
    } else {
        files
    };
    let mut writer = Writer {
        dictionary: &dictionary,
        format,
        output: stdout,
        sentence: 0,
    };
    let mut code = 0;
    for file in files {
        let name = Path::new(&file).display().to_string();
        let result = if file == "-" {
            writer.process(input, stdin)
        } else {
            match File::open(&file) {
                Ok(opened) => writer.process(input, &mut BufReader::new(opened)),
                Err(err) => Err(Failure::Read(err)),
            }
        };
        match result {
            Ok(()) => {}
            Err(Failure::Read(err)) => {
                report(format!("{name}: {}", describe(&err)));
                code = 1;
            }
            Err(Failure::Input(line, message)) => {
                report(format!("{name}: строка {line}: {message}"));
                code = 1;
            }
            Err(Failure::Write(err)) if err.kind() == io::ErrorKind::BrokenPipe => return 0,
            Err(Failure::Write(err)) => {
                report(format!("ошибка записи: {err}"));
                return 1;
            }
        }
    }
    match finish(writer.output.flush()) {
        0 => code,
        failed => failed,
    }
}

/// Код выхода после записи: закрытый канал (`rumorph ... | head`) - не ошибка
fn finish(result: io::Result<()>) -> u8 {
    match result {
        Ok(()) => 0,
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(_) => 1,
    }
}

/// Словарь из файла `path`; без пути - словарь крейта `rumorph-dict-ru`
fn load(path: Option<&Path>) -> Result<Dictionary, String> {
    match path {
        Some(path) => Dictionary::open(path).map_err(|err| match err {
            crate::Error::Io(err) => format!("{}: {}", path.display(), describe(&err)),
            err => format!("{}: {err}", path.display()),
        }),
        #[cfg(feature = "bundled")]
        None => Dictionary::bundled().map_err(|err| err.to_string()),
        #[cfg(not(feature = "bundled"))]
        None => Err("словарь не задан: укажите файл словаря в --dictionary".into()),
    }
}

/// Ошибка ввода-вывода по-русски для частых случаев
fn describe(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::NotFound => "файл не найден".into(),
        io::ErrorKind::PermissionDenied => "нет доступа".into(),
        io::ErrorKind::IsADirectory => "это каталог".into(),
        io::ErrorKind::InvalidData => "файл не в UTF-8".into(),
        _ => err.to_string(),
    }
}

enum Failure {
    /// Ошибка чтения входа
    Read(io::Error),
    /// Неверная строка входа: номер и что не так
    Input(usize, String),
    Write(io::Error),
}

/// Токен предложения: словоформа и есть ли после нее пробел в тексте
struct Token {
    word: String,
    space_after: bool,
}

/// Строка предложения CoNLL-U: комментарий или строка без разбора как есть либо слово -
/// номер токена и столбцы
enum Line {
    Raw(String),
    Word(usize, Vec<String>),
}

#[derive(Default)]
struct Sentence {
    /// Текст предложения: строка входа или комментарий `# text` CoNLL-U
    text: Option<String>,
    tokens: Vec<Token>,
    /// Строки входа CoNLL-U
    lines: Vec<Line>,
}

struct Writer<'a> {
    dictionary: &'a Dictionary,
    format: Format,
    output: &'a mut dyn Write,
    /// Номер последнего предложения: `# sent_id` CoNLL-U
    sentence: usize,
}

impl Writer<'_> {
    /// Читает вход построчно и пишет разбор каждого предложения
    fn process(&mut self, input: Input, reader: &mut dyn BufRead) -> Result<(), Failure> {
        let mut line = String::new();
        let mut number = 0;
        let mut sentence = Sentence::default();
        loop {
            line.clear();
            let read = reader.read_line(&mut line).map_err(Failure::Read)?;
            if read == 0 {
                break;
            }
            number += 1;
            let mut text = line.trim_end_matches(['\n', '\r']);
            if number == 1 {
                text = text.strip_prefix('\u{feff}').unwrap_or(text);
            }
            match input {
                Input::Text => {
                    let sentence = Sentence {
                        text: Some(text.to_owned()),
                        tokens: tokenize(text),
                        lines: Vec::new(),
                    };
                    self.write(&sentence).map_err(Failure::Write)?;
                }
                _ if text.trim().is_empty() => {
                    self.write(&sentence).map_err(Failure::Write)?;
                    sentence = Sentence::default();
                }
                Input::Tokens => sentence.tokens.push(Token {
                    word: text.to_owned(),
                    space_after: true,
                }),
                Input::Conllu => conllu_line(&mut sentence, text)
                    .map_err(|message| Failure::Input(number, message))?,
            }
        }
        if !sentence.tokens.is_empty() || !sentence.lines.is_empty() {
            self.write(&sentence).map_err(Failure::Write)?;
        }
        Ok(())
    }

    fn write(&mut self, sentence: &Sentence) -> io::Result<()> {
        let empty = sentence.tokens.is_empty() && sentence.lines.is_empty();
        // Пустая строка текста остается пустой строкой лемм; в остальных форматах ее нет
        if empty && !(self.format == Format::Lemmas && sentence.text.is_some()) {
            return Ok(());
        }
        let analyses: Vec<Vec<Analysis>> = sentence
            .tokens
            .iter()
            .map(|token| self.dictionary.analyze(&token.word))
            .collect();
        let lemma = |index: usize| -> String {
            analyses[index].first().map_or_else(
                || sentence.tokens[index].word.to_lowercase(),
                |analysis| analysis.lemma.clone(),
            )
        };
        let mut out = String::new();
        match self.format {
            Format::Lemmas => {
                let lemmas: Vec<String> = (0..sentence.tokens.len()).map(lemma).collect();
                out.push_str(&lemmas.join(" "));
                out.push('\n');
            }
            Format::Tsv => {
                for (index, token) in sentence.tokens.iter().enumerate() {
                    let (pos, feats, prob) = self.first(&analyses[index]);
                    let _ = writeln!(
                        out,
                        "{}\t{}\t{pos}\t{feats}\t{prob}",
                        token.word,
                        lemma(index)
                    );
                }
                out.push('\n');
            }
            Format::Conllu if !sentence.lines.is_empty() => {
                for line in &sentence.lines {
                    match line {
                        Line::Raw(text) => out.push_str(text),
                        Line::Word(index, columns) => {
                            let (pos, feats, _) = self.first(&analyses[*index]);
                            let mut columns = columns.clone();
                            columns[2] = lemma(*index);
                            columns[3] = pos.to_owned();
                            columns[5] = feats;
                            out.push_str(&columns.join("\t"));
                        }
                    }
                    out.push('\n');
                }
                out.push('\n');
            }
            Format::Conllu => {
                self.sentence += 1;
                let _ = writeln!(out, "# sent_id = {}", self.sentence);
                if let Some(text) = &sentence.text {
                    let _ = writeln!(out, "# text = {text}");
                }
                let last = sentence.tokens.len().saturating_sub(1);
                for (index, token) in sentence.tokens.iter().enumerate() {
                    let (pos, feats, _) = self.first(&analyses[index]);
                    let misc = if token.space_after || index == last {
                        "_"
                    } else {
                        "SpaceAfter=No"
                    };
                    let _ = writeln!(
                        out,
                        "{}\t{}\t{}\t{pos}\t_\t{feats}\t_\t_\t_\t{misc}",
                        index + 1,
                        token.word,
                        lemma(index)
                    );
                }
                out.push('\n');
            }
            Format::Jsonl => {
                out.push('{');
                if let Some(text) = &sentence.text {
                    out.push_str("\"text\": ");
                    json_string(&mut out, text);
                    out.push_str(", ");
                }
                out.push_str("\"tokens\": [");
                for (index, token) in sentence.tokens.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str("{\"word\": ");
                    json_string(&mut out, &token.word);
                    out.push_str(", \"analyses\": [");
                    for (number, analysis) in analyses[index].iter().enumerate() {
                        if number > 0 {
                            out.push_str(", ");
                        }
                        self.json_analysis(&mut out, analysis);
                    }
                    out.push_str("]}");
                }
                out.push_str("]}\n");
            }
        }
        self.output.write_all(out.as_bytes())
    }

    /// Часть речи, признаки CoNLL-U и вероятность самого вероятного разбора
    fn first(&self, analyses: &[Analysis]) -> (&str, String, f64) {
        let Some(analysis) = analyses.first() else {
            return ("X", "_".into(), 0.0);
        };
        match self.dictionary.ud_tag(analysis.ud) {
            Some(tag) if tag.feats().len() > 0 => (tag.pos(), tag.feats_string(), analysis.prob),
            Some(tag) => (tag.pos(), "_".into(), analysis.prob),
            None => ("X", "_".into(), analysis.prob),
        }
    }

    fn json_analysis(&self, out: &mut String, analysis: &Analysis) {
        let tag = self.dictionary.ud_tag(analysis.ud);
        out.push_str("{\"lemma\": ");
        json_string(out, &analysis.lemma);
        out.push_str(", \"pos\": ");
        json_string(out, tag.map_or("X", |tag| tag.pos()));
        out.push_str(", \"feats\": {");
        for (number, (name, value)) in tag.into_iter().flat_map(|tag| tag.feats()).enumerate() {
            if number > 0 {
                out.push_str(", ");
            }
            json_string(out, name);
            out.push_str(": ");
            json_string(out, value);
        }
        out.push_str("}, \"labels\": [");
        for (number, label) in tag.into_iter().flat_map(|tag| tag.labels()).enumerate() {
            if number > 0 {
                out.push_str(", ");
            }
            json_string(out, label);
        }
        let _ = write!(
            out,
            "], \"prob\": {}, \"source\": \"{}\"}}",
            analysis.prob,
            analysis.source.as_str()
        );
    }
}

/// Строка CoNLL-U в предложение: комментарий, слово или многословный токен и пустой узел,
/// которые остаются как есть
fn conllu_line(sentence: &mut Sentence, line: &str) -> Result<(), String> {
    if let Some(comment) = line.strip_prefix('#') {
        if let Some(text) = comment.trim_start().strip_prefix("text = ") {
            sentence.text = Some(text.to_owned());
        }
        sentence.lines.push(Line::Raw(line.to_owned()));
        return Ok(());
    }
    let columns: Vec<String> = line.split('\t').map(str::to_owned).collect();
    if columns.len() != 10 {
        return Err(format!(
            "в строке CoNLL-U {} столбцов вместо 10",
            columns.len()
        ));
    }
    if columns[0].parse::<usize>().is_err() {
        sentence.lines.push(Line::Raw(line.to_owned()));
        return Ok(());
    }
    sentence.tokens.push(Token {
        word: columns[1].clone(),
        space_after: true,
    });
    sentence
        .lines
        .push(Line::Word(sentence.tokens.len() - 1, columns));
    Ok(())
}

/// Буква или цифра слова, в том числе знак ударения и мягкий перенос внутри слова
fn word_char(c: char) -> bool {
    c.is_alphanumeric() || ('\u{300}'..='\u{36F}').contains(&c) || c == '\u{AD}'
}

/// Токены строки текста: слова (с дефисами и апострофами между буквами), числа (с точкой
/// или запятой между цифрами) и знаки; одинаковые знаки подряд («...», «!!!») - один токен
fn tokenize(line: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut tokens: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        if c.is_whitespace() {
            if let Some(last) = tokens.last_mut() {
                last.space_after = true;
            }
            i += 1;
            continue;
        }
        let mut end = i + 1;
        if word_char(c) {
            loop {
                while end < chars.len() && word_char(chars[end].1) {
                    end += 1;
                }
                let joined = chars.get(end).zip(chars.get(end + 1)).is_some_and(
                    |(&(_, joiner), &(_, next))| {
                        let before = chars[end - 1].1;
                        match joiner {
                            '-' | '\u{2010}' | '\u{2011}' | '\'' | '’' => word_char(next),
                            '.' | ',' => before.is_ascii_digit() && next.is_ascii_digit(),
                            _ => false,
                        }
                    },
                );
                if !joined {
                    break;
                }
                end += 1;
            }
        } else {
            while end < chars.len() && chars[end].1 == c {
                end += 1;
            }
        }
        let stop = chars.get(end).map_or(line.len(), |&(at, _)| at);
        tokens.push(Token {
            word: line[start..stop].to_owned(),
            space_after: false,
        });
        i = end;
    }
    tokens
}

/// Строка JSON с экранированием
fn json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::tests::dictionary_bytes_of;

    fn words(line: &str) -> Vec<(String, bool)> {
        tokenize(line)
            .into_iter()
            .map(|token| (token.word, token.space_after))
            .collect()
    }

    #[test]
    fn tokenizes_text() {
        let tokens: Vec<String> = words("Мы стали друзьями... В 2005-м году - 3,14%!")
            .into_iter()
            .map(|(word, _)| word)
            .collect();
        assert_eq!(
            tokens,
            [
                "Мы",
                "стали",
                "друзьями",
                "...",
                "В",
                "2005-м",
                "году",
                "-",
                "3,14",
                "%",
                "!"
            ]
        );
        let tokens: Vec<String> = words("«Кто-то» д’Артаньян 1.5, а не 1. ё\u{301}жик")
            .into_iter()
            .map(|(word, _)| word)
            .collect();
        assert_eq!(
            tokens,
            [
                "«",
                "Кто-то",
                "»",
                "д’Артаньян",
                "1.5",
                ",",
                "а",
                "не",
                "1",
                ".",
                "ё\u{301}жик"
            ]
        );
        assert_eq!(
            words("Ну,да"),
            [
                ("Ну".into(), false),
                (",".into(), false),
                ("да".into(), false)
            ]
        );
        assert_eq!(
            words(" Ну  да "),
            [("Ну".into(), true), ("да".into(), true)]
        );
        assert!(words("   ").is_empty());
        assert_eq!(
            words("-кто-"),
            [
                ("-".into(), false),
                ("кто".into(), false),
                ("-".into(), false)
            ]
        );
    }

    #[test]
    fn parses_arguments() {
        let args = |items: &[&str]| parse(items.iter().map(OsString::from).collect());
        assert_eq!(
            args(&["-f", "conllu", "--input=tokens", "-dru.dict", "a.txt", "-"]),
            Ok(Command::Run {
                format: Format::Conllu,
                input: Input::Tokens,
                dictionary: Some(PathBuf::from("ru.dict")),
                files: vec!["a.txt".into(), "-".into()],
            })
        );
        assert_eq!(args(&["a", "--help"]), Ok(Command::Help));
        assert_eq!(args(&["-V"]), Ok(Command::Version));
        assert_eq!(
            args(&["--", "-f"]),
            Ok(Command::Run {
                format: Format::Lemmas,
                input: Input::Text,
                dictionary: None,
                files: vec!["-f".into()],
            })
        );
        assert_eq!(
            args(&["-f", "xml"]),
            Err(
                "неизвестное значение параметра -f: xml (допустимые: lemmas, conllu, tsv, jsonl)"
                    .into()
            )
        );
        assert_eq!(
            args(&["--format"]),
            Err("параметру --format нужно значение".into())
        );
        assert_eq!(args(&["--all"]), Err("неизвестный параметр --all".into()));
    }

    /// Словарь «мама», «мыла» и «раму» в файле: команда читает словарь из файла
    struct TestDictionary(PathBuf);

    impl TestDictionary {
        fn new() -> Self {
            let noun = |case: &str| format!("Case={case}|Gender=Fem|Number=Sing");
            let verb = |feats: &str| feats.to_owned();
            let data = dictionary_bytes_of(
                &[
                    ("NOUN", "мам", vec![("а", noun("Nom"))]),
                    ("NOUN", "рам", vec![("а", noun("Nom")), ("у", noun("Acc"))]),
                    (
                        "VERB",
                        "мы",
                        vec![
                            ("ть", verb("VerbForm=Inf")),
                            ("ла", verb("Gender=Fem|Number=Sing|Tense=Past")),
                        ],
                    ),
                ],
                &[],
            );
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NUMBER: AtomicUsize = AtomicUsize::new(0);
            let number = NUMBER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("rumorph-cli-{}-{number}.dict", std::process::id()));
            std::fs::write(&path, data).unwrap();
            TestDictionary(path)
        }

        /// Код выхода, вывод и ошибки команды с аргументами `args` и входом `input`
        fn run(&self, args: &[&str], input: &[u8]) -> (u8, String, String) {
            self.run_into(args, input, &mut Vec::new())
        }

        fn run_into(
            &self,
            args: &[&str],
            mut input: &[u8],
            stdout: &mut dyn Write,
        ) -> (u8, String, String) {
            let args = args.iter().map(OsString::from).collect();
            let mut captured = Vec::new();
            let mut stderr = Vec::new();
            let code = {
                let mut tee = Tee(stdout, &mut captured);
                execute(
                    args,
                    "1.2.3",
                    Some(&self.0),
                    &mut input,
                    &mut tee,
                    &mut stderr,
                )
            };
            (
                code,
                String::from_utf8(captured).unwrap(),
                String::from_utf8(stderr).unwrap(),
            )
        }
    }

    impl Drop for TestDictionary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Запись в два места: в проверяемый поток и в буфер для сравнения
    struct Tee<'a>(&'a mut dyn Write, &'a mut Vec<u8>);

    impl Write for Tee<'_> {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            let written = self.0.write(data)?;
            self.1.extend_from_slice(&data[..written]);
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    #[test]
    fn writes_formats() {
        let dict = TestDictionary::new();
        let text = "\u{feff}Мама мыла раму.\r\n\nРаму!\n".as_bytes();
        assert_eq!(
            dict.run(&[], text),
            (0, "мама мыть рама .\n\nрама !\n".into(), String::new())
        );
        let (code, tsv, _) = dict.run(&["-f", "tsv"], "Раму!".as_bytes());
        assert_eq!(code, 0);
        assert_eq!(
            tsv,
            "Раму\tрама\tNOUN\tCase=Acc|Gender=Fem|Number=Sing\t1\n!\t!\tPUNCT\t_\t1\n\n"
        );
        let (_, conllu, _) = dict.run(&["-f", "conllu"], "Мама мыла раму.".as_bytes());
        assert_eq!(
            conllu,
            "# sent_id = 1\n# text = Мама мыла раму.\n\
             1\tМама\tмама\tNOUN\t_\tCase=Nom|Gender=Fem|Number=Sing\t_\t_\t_\t_\n\
             2\tмыла\tмыть\tVERB\t_\tGender=Fem|Number=Sing|Tense=Past\t_\t_\t_\t_\n\
             3\tраму\tрама\tNOUN\t_\tCase=Acc|Gender=Fem|Number=Sing\t_\t_\t_\tSpaceAfter=No\n\
             4\t.\t.\tPUNCT\t_\t_\t_\t_\t_\t_\n\n"
        );
        let (_, jsonl, _) = dict.run(&["-f", "jsonl"], "\"Раму\"".as_bytes());
        let punct = "{\"word\": \"\\\"\", \"analyses\": [{\"lemma\": \"\\\"\", \"pos\": \"PUNCT\", \
                     \"feats\": {}, \"labels\": [], \"prob\": 1, \"source\": \"rule\"}]}";
        assert_eq!(
            jsonl,
            format!(
                "{{\"text\": \"\\\"Раму\\\"\", \"tokens\": [{punct}, \
                 {{\"word\": \"Раму\", \"analyses\": [{{\"lemma\": \"рама\", \"pos\": \"NOUN\", \
                 \"feats\": {{\"Case\": \"Acc\", \"Gender\": \"Fem\", \"Number\": \"Sing\"}}, \
                 \"labels\": [], \"prob\": 1, \"source\": \"dictionary\"}}]}}, {punct}]}}\n"
            )
        );
    }

    #[test]
    fn reads_tokens_and_conllu() {
        let dict = TestDictionary::new();
        let (_, lemmas, _) = dict.run(&["-i", "tokens"], "Мама\nмыла\n\n\nраму\n".as_bytes());
        assert_eq!(lemmas, "мама мыть\nрама\n");
        let conllu = "# text = Мама мыла\n1-2\tМамамыла\t_\t_\t_\t_\t_\t_\t_\t_\n\
                      1\tМама\t_\t_\tNN\t_\t2\tnsubj\t_\t_\n2\tмыла\t_\t_\tVB\t_\t0\troot\t_\t_\n";
        let (code, out, _) = dict.run(&["-i", "conllu", "-f", "conllu"], conllu.as_bytes());
        assert_eq!(code, 0);
        assert_eq!(
            out,
            "# text = Мама мыла\n1-2\tМамамыла\t_\t_\t_\t_\t_\t_\t_\t_\n\
             1\tМама\tмама\tNOUN\tNN\tCase=Nom|Gender=Fem|Number=Sing\t2\tnsubj\t_\t_\n\
             2\tмыла\tмыть\tVERB\tVB\tGender=Fem|Number=Sing|Tense=Past\t0\troot\t_\t_\n\n"
        );
        let (_, jsonl, _) = dict.run(&["-i", "conllu", "-f", "jsonl"], conllu.as_bytes());
        assert!(jsonl.starts_with("{\"text\": \"Мама мыла\", \"tokens\": [{\"word\": \"Мама\""));
        let (code, _, error) = dict.run(&["-i", "conllu"], "1\tМама\n".as_bytes());
        assert_eq!(code, 1);
        assert_eq!(
            error,
            "rumorph: -: строка 1: в строке CoNLL-U 2 столбцов вместо 10\n"
        );
    }

    #[test]
    fn reports_errors() {
        let dict = TestDictionary::new();
        assert_eq!(dict.run(&["--help"], b""), (0, HELP.into(), String::new()));
        assert_eq!(dict.run(&["-V"], b"").1, "rumorph 1.2.3\n");
        let (code, _, error) = dict.run(&["-x"], b"");
        assert_eq!(code, 2);
        assert_eq!(
            error,
            "rumorph: неизвестный параметр -x\nСправка: rumorph --help\n"
        );
        // Нет файла - ошибка, а остальные файлы все равно разбираются
        let (code, out, error) = dict.run(&["нет.txt", "-"], "раму".as_bytes());
        assert_eq!((code, out.as_str()), (1, "рама\n"));
        assert_eq!(error, "rumorph: нет.txt: файл не найден\n");
        let (code, _, error) = dict.run(&["-d", "нет.dict"], b"");
        assert_eq!(
            (code, error.as_str()),
            (1, "rumorph: нет.dict: файл не найден\n")
        );
        let (code, _, error) = dict.run(&[], b"\xff\n");
        assert_eq!((code, error.as_str()), (1, "rumorph: -: файл не в UTF-8\n"));
    }

    #[test]
    fn closed_pipe_is_not_an_error() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }
        let dict = TestDictionary::new();
        assert_eq!(
            dict.run_into(&[], "раму\n".as_bytes(), &mut Closed),
            (0, String::new(), String::new())
        );
    }

    #[test]
    fn escapes_json() {
        let mut out = String::new();
        json_string(&mut out, "a\"b\\c\n\t\u{1}ё");
        assert_eq!(out, "\"a\\\"b\\\\c\\n\\t\\u0001ё\"");
    }
}
