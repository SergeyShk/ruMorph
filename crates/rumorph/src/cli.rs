//! Команда `rumorph`: морфологический анализ текста из файлов и стандартного ввода

use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, IsTerminal, Write};
use std::iter::Peekable;
use std::path::{Path, PathBuf};

use unicode_segmentation::{GraphemeIndices, UnicodeSegmentation};

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
/// 0 - успех, 1 - ошибка чтения файла, словаря или записи, 2 - неверные аргументы. `version`
/// печатает `--version`; `dictionary` - словарь по умолчанию, без него - словарь крейта
/// `rumorph-dict-ru`
pub fn run(args: Vec<OsString>, version: &str, dictionary: Option<&Path>) -> u8 {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    let mut output = BufWriter::new(stdout.lock());
    execute(
        args,
        version,
        dictionary,
        interactive,
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
    interactive: bool,
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
        Ok(Command::Help) => {
            let written = stdout
                .write_all(HELP.as_bytes())
                .and_then(|()| stdout.flush());
            return written.map_or_else(|err| write_error(&err, &mut report), |()| 0);
        }
        Ok(Command::Version) => {
            let written = writeln!(stdout, "rumorph {version}").and_then(|()| stdout.flush());
            return written.map_or_else(|err| write_error(&err, &mut report), |()| 0);
        }
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
        interactive,
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
            Ok(None) => {}
            Ok(Some(line)) => {
                report(format!(
                    "{name}: строка {line}: не UTF-8, неверные байты заменены на U+FFFD"
                ));
                code = 1;
            }
            Err(Failure::Read(err)) => {
                report(format!("{name}: {}", describe(&err)));
                code = 1;
            }
            Err(Failure::Encoding(line)) => {
                report(format!("{name}: файл не в UTF-8 (строка {line})"));
                code = 1;
            }
            Err(Failure::Input(line, message)) => {
                report(format!("{name}: строка {line}: {message}"));
                code = 1;
            }
            Err(Failure::Write(err)) => return write_error(&err, &mut report),
        }
    }
    match writer.output.flush() {
        Ok(()) => code,
        Err(err) => write_error(&err, &mut report).max(code),
    }
}

/// Код выхода после ошибки записи: закрытый канал (`rumorph ... | head`) - не ошибка
fn write_error(err: &io::Error, report: &mut impl FnMut(String)) -> u8 {
    if err.kind() == io::ErrorKind::BrokenPipe {
        return 0;
    }
    report(format!("ошибка записи: {err}"));
    1
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
        _ => err.to_string(),
    }
}

enum Failure {
    /// Ошибка чтения входа
    Read(io::Error),
    /// Файл в другой кодировке: UTF-16 или неверные байты раньше первой строки UTF-8; номер
    /// строки, на которой это видно
    Encoding(usize),
    /// Неверная строка входа: номер и что не так
    Input(usize, String),
    Write(io::Error),
}

/// Токен предложения: словоформа и есть ли после нее пробел в тексте
#[derive(Clone, Copy, Debug, PartialEq)]
struct Token<'a> {
    word: &'a str,
    space_after: bool,
}

/// Строка предложения CoNLL-U: комментарий или строка без разбора как есть либо слово -
/// столбцы
enum Line {
    Raw(String),
    Word(Vec<String>),
}

/// Предложение из токенов или CoNLL-U: текст и строки; токены - слова строк
#[derive(Default)]
struct Block {
    /// Текст предложения: комментарий `# text` CoNLL-U
    text: Option<String>,
    words: Vec<String>,
    lines: Vec<Line>,
}

struct Writer<'a> {
    dictionary: &'a Dictionary,
    format: Format,
    output: &'a mut dyn Write,
    /// Сбрасывать вывод после каждого предложения: в терминале разбор виден сразу
    interactive: bool,
    /// Номер последнего предложения: `# sent_id` CoNLL-U
    sentence: usize,
}

impl Writer<'_> {
    /// Читает вход построчно и пишет разбор каждого предложения; вывод - номер первой строки
    /// не в UTF-8, если такая была: неверные байты заменяются на U+FFFD
    fn process(
        &mut self,
        input: Input,
        reader: &mut dyn BufRead,
    ) -> Result<Option<usize>, Failure> {
        let mut bytes = Vec::new();
        let mut number = 0;
        let mut invalid = None;
        // Была ли строка UTF-8 с не-ASCII байтами: до нее неверные байты и NUL значат другую
        // кодировку файла (cp1251, UTF-16 без BOM), после - испорченную строку
        let mut utf8 = false;
        let mut block = Block::default();
        loop {
            bytes.clear();
            if reader
                .read_until(b'\n', &mut bytes)
                .map_err(Failure::Read)?
                == 0
            {
                break;
            }
            number += 1;
            let utf16 = bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]);
            if (number == 1 && utf16) || (!utf8 && bytes.contains(&0)) {
                return Err(Failure::Encoding(number));
            }
            let decoded = String::from_utf8_lossy(&bytes);
            match decoded {
                Cow::Owned(_) if !utf8 => return Err(Failure::Encoding(number)),
                Cow::Owned(_) => {
                    invalid.get_or_insert(number);
                }
                Cow::Borrowed(_) => utf8 |= !bytes.is_ascii(),
            }
            let mut text = decoded.trim_end_matches(['\n', '\r']);
            if number == 1 {
                text = text.strip_prefix('\u{feff}').unwrap_or(text);
            }
            match input {
                Input::Text => {
                    let tokens = Tokens::new(text).peekable();
                    self.write_tokens(Some(text), tokens, true)
                        .map_err(Failure::Write)?;
                }
                _ if text.trim().is_empty() => {
                    self.write_block(&block).map_err(Failure::Write)?;
                    block = Block::default();
                }
                Input::Tokens => {
                    // Токен - до табуляции: так читается и таблица с токеном в первом столбце
                    let token = text.split('\t').next().unwrap_or_default().trim();
                    if !token.is_empty() {
                        block.words.push(token.to_owned());
                    }
                }
                Input::Conllu => conllu_line(&mut block, text)
                    .map_err(|message| Failure::Input(number, message))?,
            }
        }
        self.write_block(&block).map_err(Failure::Write)?;
        Ok(invalid)
    }

    /// Предложение из токенов или CoNLL-U
    fn write_block(&mut self, block: &Block) -> io::Result<()> {
        if self.format == Format::Conllu && !block.lines.is_empty() {
            return self.write_conllu_lines(block);
        }
        let tokens = block.words.iter().map(|word| Token {
            word,
            space_after: true,
        });
        self.write_tokens(block.text.as_deref(), tokens.peekable(), false)
    }

    /// Предложение в формате вывода: токены пишутся по одному, без разборов всего предложения
    /// в памяти. Пустая строка текста (`keep_empty`) остается пустой строкой лемм
    fn write_tokens<'t>(
        &mut self,
        text: Option<&str>,
        mut tokens: Peekable<impl Iterator<Item = Token<'t>>>,
        keep_empty: bool,
    ) -> io::Result<()> {
        if tokens.peek().is_none() {
            if keep_empty && self.format == Format::Lemmas {
                writeln!(self.output)?;
                self.flush_interactive()?;
            }
            return Ok(());
        }
        let mut piece = String::new();
        match self.format {
            Format::Lemmas => {
                for (index, token) in tokens.enumerate() {
                    let analyses = self.dictionary.analyze(token.word);
                    let separator = if index > 0 { " " } else { "" };
                    write!(self.output, "{separator}{}", lemma(&analyses, token.word))?;
                }
                writeln!(self.output)?;
            }
            Format::Tsv => {
                for token in tokens {
                    let analyses = self.dictionary.analyze(token.word);
                    let (pos, feats, prob) = first(self.dictionary, &analyses);
                    let lemma = lemma(&analyses, token.word);
                    writeln!(
                        self.output,
                        "{}\t{lemma}\t{pos}\t{feats}\t{prob}",
                        token.word
                    )?;
                }
                writeln!(self.output)?;
            }
            Format::Conllu => {
                self.sentence += 1;
                writeln!(self.output, "# sent_id = {}", self.sentence)?;
                if let Some(text) = text {
                    writeln!(self.output, "# text = {text}")?;
                }
                for (index, token) in tokens.enumerate() {
                    let analyses = self.dictionary.analyze(token.word);
                    let (pos, feats, _) = first(self.dictionary, &analyses);
                    let misc = if token.space_after {
                        "_"
                    } else {
                        "SpaceAfter=No"
                    };
                    writeln!(
                        self.output,
                        "{}\t{}\t{}\t{pos}\t_\t{feats}\t_\t_\t_\t{misc}",
                        index + 1,
                        token.word,
                        lemma(&analyses, token.word)
                    )?;
                }
                writeln!(self.output)?;
            }
            Format::Jsonl => {
                piece.push('{');
                if let Some(text) = text {
                    piece.push_str("\"text\": ");
                    json_string(&mut piece, text);
                    piece.push_str(", ");
                }
                piece.push_str("\"tokens\": [");
                self.output.write_all(piece.as_bytes())?;
                for (index, token) in tokens.enumerate() {
                    piece.clear();
                    if index > 0 {
                        piece.push_str(", ");
                    }
                    piece.push_str("{\"word\": ");
                    json_string(&mut piece, token.word);
                    piece.push_str(", \"analyses\": [");
                    for (number, analysis) in self.dictionary.analyze(token.word).iter().enumerate()
                    {
                        if number > 0 {
                            piece.push_str(", ");
                        }
                        self.json_analysis(&mut piece, analysis);
                    }
                    piece.push_str("]}");
                    self.output.write_all(piece.as_bytes())?;
                }
                writeln!(self.output, "]}}")?;
            }
        }
        self.flush_interactive()
    }

    /// Предложение CoNLL-U с заполненными LEMMA, UPOS и FEATS
    fn write_conllu_lines(&mut self, block: &Block) -> io::Result<()> {
        for line in &block.lines {
            match line {
                Line::Raw(text) => writeln!(self.output, "{text}")?,
                Line::Word(columns) => {
                    let analyses = self.dictionary.analyze(&columns[1]);
                    let (pos, feats, _) = first(self.dictionary, &analyses);
                    let lemma = lemma(&analyses, &columns[1]);
                    let [id, form, _, _, xpos, _, rest @ ..] = &columns[..] else {
                        continue;
                    };
                    writeln!(
                        self.output,
                        "{id}\t{form}\t{lemma}\t{pos}\t{xpos}\t{feats}\t{}",
                        rest.join("\t")
                    )?;
                }
            }
        }
        writeln!(self.output)?;
        self.flush_interactive()
    }

    fn flush_interactive(&mut self) -> io::Result<()> {
        if self.interactive {
            self.output.flush()?;
        }
        Ok(())
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

/// Часть речи, признаки CoNLL-U и вероятность самого вероятного разбора
fn first<'d>(dictionary: &'d Dictionary, analyses: &[Analysis]) -> (&'d str, String, f64) {
    let Some(analysis) = analyses.first() else {
        return ("X", "_".into(), 0.0);
    };
    match dictionary.ud_tag(analysis.ud) {
        Some(tag) if tag.feats().len() > 0 => (tag.pos(), tag.feats_string(), analysis.prob),
        Some(tag) => (tag.pos(), "_".into(), analysis.prob),
        None => ("X", "_".into(), analysis.prob),
    }
}

/// Лемма самого вероятного разбора; без разборов - токен в нижнем регистре
fn lemma<'a>(analyses: &'a [Analysis], word: &str) -> Cow<'a, str> {
    match analyses.first() {
        Some(analysis) => Cow::Borrowed(&analysis.lemma),
        None => Cow::Owned(word.to_lowercase()),
    }
}

/// Строка CoNLL-U в предложение: комментарий, слово или многословный токен и пустой узел,
/// которые остаются как есть
fn conllu_line(block: &mut Block, line: &str) -> Result<(), String> {
    if let Some(comment) = line.strip_prefix('#') {
        if let Some(text) = comment.trim_start().strip_prefix("text = ") {
            block.text = Some(text.to_owned());
        }
        block.lines.push(Line::Raw(line.to_owned()));
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
        block.lines.push(Line::Raw(line.to_owned()));
        return Ok(());
    }
    block.words.push(columns[1].clone());
    block.lines.push(Line::Word(columns));
    Ok(())
}

/// Графема-буква или цифра: с ней идут знаки ударения и другие комбинирующие знаки
fn word(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(char::is_alphanumeric)
}

/// Разделитель токенов: графема с пробела или невидимого символа (пробел нулевой ширины,
/// соединитель слов, BOM, мягкий перенос вне слова), после которого нет пробела в тексте
fn separator(grapheme: &str) -> bool {
    grapheme.chars().next().is_some_and(|c| {
        c.is_whitespace() || matches!(c, '\u{200b}' | '\u{2060}' | '\u{feff}' | '\u{ad}')
    })
}

/// Токены строки текста по графемам: слова (с дефисами и апострофами между буквами, мягкими
/// переносами), числа (с точкой или запятой между цифрами) и знаки; одинаковые знаки подряд
/// («...», «!!!») - один токен, эмодзи с модификаторами и флаги - одна графема
struct Tokens<'a> {
    line: &'a str,
    graphemes: Peekable<GraphemeIndices<'a>>,
}

impl<'a> Tokens<'a> {
    fn new(line: &'a str) -> Self {
        Tokens {
            line,
            graphemes: line.grapheme_indices(true).peekable(),
        }
    }

    /// Соединяет ли графема `joiner` слово с графемой `next` после него
    fn joins(last: &str, joiner: &str, next: &str) -> bool {
        match joiner {
            "-" | "\u{2010}" | "\u{2011}" | "'" | "’" | "\u{2060}" | "\u{feff}" => word(next),
            "." | "," => {
                last.chars().last().is_some_and(|c| c.is_ascii_digit())
                    && next.chars().next().is_some_and(|c| c.is_ascii_digit())
            }
            _ => false,
        }
    }
}

impl<'a> Iterator for Tokens<'a> {
    type Item = Token<'a>;

    fn next(&mut self) -> Option<Token<'a>> {
        let (start, first) = loop {
            let (at, grapheme) = self.graphemes.next()?;
            if !separator(grapheme) {
                break (at, grapheme);
            }
        };
        let (mut end, mut last) = (start + first.len(), first);
        if word(first) {
            while let Some(&(at, grapheme)) = self.graphemes.peek() {
                if word(grapheme) || grapheme == "\u{ad}" {
                    self.graphemes.next();
                    (end, last) = (at + grapheme.len(), grapheme);
                    continue;
                }
                let mut ahead = self.graphemes.clone();
                ahead.next();
                match ahead.next() {
                    Some((at, next)) if Self::joins(last, grapheme, next) => {
                        self.graphemes.next();
                        self.graphemes.next();
                        (end, last) = (at + next.len(), next);
                    }
                    _ => break,
                }
            }
        } else {
            while let Some(&(at, grapheme)) = self.graphemes.peek() {
                if grapheme != first {
                    break;
                }
                self.graphemes.next();
                end = at + grapheme.len();
            }
        }
        let space_after = self
            .graphemes
            .peek()
            .is_none_or(|&(_, grapheme)| grapheme.chars().next().is_some_and(char::is_whitespace));
        Some(Token {
            word: &self.line[start..end],
            space_after,
        })
    }
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
        Tokens::new(line)
            .map(|token| (token.word.to_owned(), token.space_after))
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
                ("да".into(), true)
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
                ("-".into(), true)
            ]
        );
        // Эмодзи с селектором, тоном кожи, флаг и семья через ZWJ - по одному токену
        let tokens: Vec<String> = words("❤️ 👍🏽 🇷🇺 👨‍👩‍👧!!")
            .into_iter()
            .map(|(word, _)| word)
            .collect();
        assert_eq!(tokens, ["❤️", "👍🏽", "🇷🇺", "👨‍👩‍👧", "!!"]);
        // Знак после пробела и мягкий перенос вне слова - не токены
        assert_eq!(
            words("кот \u{301}пес \u{ad} да"),
            [
                ("кот".into(), true),
                ("пес".into(), true),
                ("да".into(), true)
            ]
        );
        // Невидимые символы разделяют токены, но пробелом не считаются; мягкий перенос
        // и соединитель слов - в слове
        assert_eq!(
            words("при\u{200b}вет \u{feff}раму при\u{ad}вет при\u{2060}вет"),
            [
                ("при".into(), false),
                ("вет".into(), true),
                ("раму".into(), true),
                ("при\u{ad}вет".into(), true),
                ("при\u{2060}вет".into(), true)
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
                let interactive = false;
                execute(
                    args,
                    "1.2.3",
                    Some(&self.0),
                    interactive,
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
        // Пробелы по краям и все после табуляции - не токен
        let (_, tsv, _) = dict.run(
            &["-i", "tokens", "-f", "tsv"],
            "  мыла \tVERB\n\tX\n".as_bytes(),
        );
        assert_eq!(
            tsv,
            "мыла\tмыть\tVERB\tGender=Fem|Number=Sing|Tense=Past\t1\n\n"
        );
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
        // Строка не в UTF-8 в файле UTF-8 разбирается с U+FFFD вместо неверных байт
        let mut input = "Раму\n".as_bytes().to_vec();
        input.extend_from_slice(b"\xd0\xff\n");
        input.extend_from_slice("раму\n".as_bytes());
        let (code, out, error) = dict.run(&[], &input);
        assert_eq!((code, out.as_str()), (1, "рама\n\u{fffd}\u{fffd}\nрама\n"));
        assert_eq!(
            error,
            "rumorph: -: строка 2: не UTF-8, неверные байты заменены на U+FFFD\n"
        );
        // Файл в другой кодировке не разбирается: cp1251, UTF-16 с BOM и без него
        for (input, line) in [
            (&b"abc\n\xd0\xe0\xec\xf3\n"[..], 2),
            (b"\xff\xfe\x20\x04", 1),
            (b"\x1c\x04\x30\x04\x20\x00", 1),
        ] {
            let (code, out, error) = dict.run(&[], input);
            assert_eq!(
                (code, error),
                (1, format!("rumorph: -: файл не в UTF-8 (строка {line})\n")),
                "{input:?}"
            );
            assert!(out.len() <= "abc\n".len(), "{out}");
        }
    }

    /// Вывод, который считает сбросы и может отказать при сбросе
    struct Output {
        data: Vec<u8>,
        flushes: usize,
        failing: bool,
    }

    impl Write for Output {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.data.extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            if self.failing {
                return Err(io::Error::other("диск полон"));
            }
            Ok(())
        }
    }

    fn run_with(dict: &TestDictionary, interactive: bool, output: &mut Output) -> (u8, String) {
        let mut stderr = Vec::new();
        let code = execute(
            Vec::new(),
            "",
            Some(&dict.0),
            interactive,
            &mut "раму\nраму\n".as_bytes(),
            output,
            &mut stderr,
        );
        (code, String::from_utf8(stderr).unwrap())
    }

    #[test]
    fn flushes_terminal_after_each_sentence() {
        let dict = TestDictionary::new();
        let mut output = Output {
            data: Vec::new(),
            flushes: 0,
            failing: false,
        };
        assert_eq!(run_with(&dict, true, &mut output), (0, String::new()));
        assert_eq!(output.data, "рама\nрама\n".as_bytes());
        // По сбросу на предложение и последний
        assert_eq!(output.flushes, 3);
        output.flushes = 0;
        assert_eq!(run_with(&dict, false, &mut output).0, 0);
        assert_eq!(output.flushes, 1);
    }

    #[test]
    fn reports_failed_final_write() {
        let dict = TestDictionary::new();
        let mut output = Output {
            data: Vec::new(),
            flushes: 0,
            failing: true,
        };
        assert_eq!(
            run_with(&dict, false, &mut output),
            (1, "rumorph: ошибка записи: диск полон\n".into())
        );
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
