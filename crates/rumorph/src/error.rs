use std::fmt;

/// Ошибка чтения или сборки словаря
#[derive(Debug)]
pub enum Error {
    /// Ошибка ввода-вывода
    Io(std::io::Error),
    /// Данные не являются словарем ruMorph или повреждены
    Format(String),
    /// Неверные данные для сборки словаря
    Build(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Format(msg) => write!(f, "неверный формат словаря: {msg}"),
            Error::Build(msg) => write!(f, "ошибка сборки словаря: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}
