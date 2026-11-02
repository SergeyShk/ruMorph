//! Разбор документа сегментатора anySeg

use anyseg::{Column, Document};

use crate::{Dictionary, Error};

impl Dictionary {
    /// Заполняет столбцы LEMMA, UPOS и FEATS слов документа anySeg (крейт - `rumorph::anyseg`)
    /// самым вероятным разбором: с `context` - в контексте предложения, как у
    /// [`Dictionary::analyze_sentence`], без него - разбором словоформы по частотам словаря, как
    /// у [`Dictionary::analyze`], в разы быстрее и без контекстной модели. Слово без разборов
    /// получает лемму в нижнем регистре и часть речи X; прежние значения столбцов заменяются
    ///
    /// ```
    /// # #[cfg(feature = "bundled")] {
    /// use rumorph::anyseg::{Column, Segmenter};
    ///
    /// let dictionary = rumorph::Dictionary::bundled()?;
    /// let mut document = Segmenter::new("ru").unwrap().segment("Мы стали друзьями.");
    /// dictionary.annotate(&mut document, true)?;
    /// assert_eq!(document.column(Column::Lemma, 1), Some("стать"));
    /// # }
    /// # Ok::<(), rumorph::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Context`], если нужна контекстная модель, а ее в словаре нет;
    /// [`Error::Document`], если документ не принял столбцы
    pub fn annotate(&self, document: &mut Document, context: bool) -> Result<(), Error> {
        // Пустой документ тоже проверяет, есть ли в словаре модель
        if context {
            self.analyze_sentence::<&str>(&[])?;
        }
        let count = document.word_count();
        let (mut lemmas, mut parts, mut feats) = (
            Vec::with_capacity(count),
            Vec::with_capacity(count),
            Vec::with_capacity(count),
        );
        for index in 0..document.sentence_count() {
            let words: Vec<&str> = (document.sentence(index).words)
                .map(|word| document.form(word))
                .collect();
            for (lemma, pos, features) in self.sentence_fields(&words, context)? {
                lemmas.push(Some(lemma));
                parts.push(Some(pos.to_owned()));
                feats.push(features);
            }
        }
        let columns = vec![
            (Column::Lemma, lemmas),
            (Column::Upos, parts),
            (Column::Feats, feats),
        ];
        document
            .set_columns(columns, None)
            .map_err(|err| Error::Document(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use crate::Error;
    use crate::dictionary::tests::dictionary;

    #[test]
    fn needs_context_model() {
        let segmenter = anyseg::Segmenter::new("ru").unwrap();
        for text in ["", "Все стали."] {
            let result = dictionary().annotate(&mut segmenter.segment(text), true);
            assert!(
                matches!(result, Err(Error::Context(_))),
                "{text:?}: {result:?}"
            );
        }
        // Без контекста модель не нужна: разбор - самый вероятный по словарю
        let mut document = segmenter.segment("Все стали.");
        dictionary().annotate(&mut document, false).unwrap();
        let first = &dictionary().analyze("стали")[0];
        assert_eq!(
            document.column(anyseg::Column::Lemma, 1),
            Some(first.lemma.as_str())
        );
    }
}
