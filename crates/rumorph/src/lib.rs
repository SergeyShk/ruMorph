//! Морфологический анализатор русского языка: словоформы словаря хранятся в конечном автомате,
//! разборы восстанавливаются по парадигмам

mod builder;
mod dictionary;
mod error;
mod format;

pub use builder::{Builder, Entry, GuessIndex};
pub use dictionary::{Analysis, Dictionary, Source, UdTag};
pub use error::Error;
