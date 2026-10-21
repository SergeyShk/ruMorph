//! Морфологический анализатор русского языка: разборы словоформ (лемма, часть речи и признаки
//! Universal Dependencies, вероятность), формы лексемы, подбор формы по признакам и согласование
//! с количеством. Словоформы словаря хранятся в конечном автомате, разборы восстанавливаются
//! по парадигмам
//!
//! ```
//! # #[cfg(feature = "bundled")] {
//! use rumorph::Dictionary;
//!
//! let dictionary = Dictionary::bundled()?;
//! let analysis = &dictionary.analyze("стали")[0];
//! assert_eq!(analysis.lemma, "стать");
//! assert_eq!(dictionary.ud_tag(analysis.ud).unwrap().pos(), "VERB");
//! # }
//! # Ok::<(), rumorph::Error>(())
//! ```
//!
//! Features: `bundled` (по умолчанию) - словарь OpenCorpora из крейта `rumorph-dict-ru`,
//! `build` - сборка своих словарей (`Builder`)
#![cfg_attr(docsrs, feature(doc_cfg))]

mod alphabet;
#[cfg(any(test, feature = "build"))]
mod builder;
#[cfg(feature = "cli")]
#[cfg_attr(docsrs, doc(cfg(feature = "cli")))]
pub mod cli;
mod dictionary;
mod error;
mod format;

#[cfg(feature = "build")]
#[cfg_attr(docsrs, doc(cfg(feature = "build")))]
pub use builder::Builder;
#[cfg(feature = "build")]
#[cfg_attr(docsrs, doc(cfg(feature = "build")))]
pub use dictionary::GuessIndex;
pub use dictionary::{Analysis, Dictionary, Entry, Source, UdTag};
pub use error::Error;

/// Примеры README крейта - doc-тесты
#[cfg(all(doctest, feature = "bundled"))]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
