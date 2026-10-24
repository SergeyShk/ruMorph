# Морфологический анализатор русского языка (ruMorph)
#
# Copyright (C) 2026
# Автор: Шкарин Сергей <kouki.sergey@gmail.com>
# URL: <https://github.com/SergeyShk/ruMorph>

"""
Компонент spaCy rumorph: часть речи, признаки и лемма токенов по разбору ruMorph с контекстом

Описание:
    Компонент заменяет в конвейере spaCy теггер, морфологизатор и лемматизатор: разбирает
    предложения документа контекстной моделью ruMorph и пишет в токены часть речи (`pos_`),
    признаки (`morph`) и лемму (`lemma_`) выбранного разбора. Предложения берутся из `Doc.sents`,
    если их разметил парсер или `sentencizer`, иначе документ делится после знаков конца
    предложения. Пробельные токены получают часть речи SPACE. Разбор ruMorph токена -
    `token._.rumorph`. После установки `pip install rumorph[spacy]` компонент подключается
    по имени, без импорта rumorph.

Пример использования:
    >>> import spacy
    >>> nlp = spacy.blank("ru")
    >>> _ = nlp.add_pipe("rumorph")
    >>> [(token.lemma_, token.pos_) for token in nlp("Мост сделан из стали.")]
    [('мост', 'NOUN'), ('сделать', 'VERB'), ('из', 'ADP'), ('сталь', 'NOUN'), ('.', 'PUNCT')]
"""

from collections.abc import Iterator

from spacy.language import Language
from spacy.tokens import Doc, MorphAnalysis, Token

from ._rumorph import Analysis, Analyzer

__all__ = ["Component"]

# Документ без разметки предложений делится, как в команде rumorph: после токена из знаков
# конца предложения и не реже чем через MAX_CHUNK токенов
SENTENCE_END = frozenset(".!?…")
MAX_CHUNK = 256


@Language.factory(
    "rumorph",
    default_config={"dictionary": None, "words": None, "context": True},
    assigns=["token.pos", "token.morph", "token.lemma"],
)
def create_component(
    nlp: Language,
    name: str,
    dictionary: str | None,
    words: dict[str, str | tuple[str, str]] | None,
    context: bool,
) -> "Component":
    """
    Компонент rumorph для конвейера spaCy: `nlp.add_pipe("rumorph", config={...})`

    Аргументы:
        nlp (Language): Конвейер
        name (str): Имя компонента в конвейере
        dictionary (str | None): Путь к словарю, None - словарь пакета
        words (dict[str, str | tuple[str, str]] | None): Свои слова, как у `Analyzer`
        context (bool): Выбирать разбор с учетом соседних слов; False - самый вероятный
            разбор слова без контекста, как у `Analyzer.analyze`

    Вывод:
        Component: Компонент
    """
    return Component(dictionary, words, context)


class Component:
    """
    Компонент spaCy: часть речи, признаки и лемма токенов по разбору ruMorph

    Аргументы:
        dictionary (str | None): Путь к словарю, None - словарь пакета
        words (dict[str, str | tuple[str, str]] | None): Свои слова, как у `Analyzer`
        context (bool): Выбирать разбор с учетом соседних слов
    """

    def __init__(
        self,
        dictionary: str | None = None,
        words: dict[str, str | tuple[str, str]] | None = None,
        context: bool = True,
    ) -> None:
        self.analyzer = Analyzer(dictionary, words)
        self.context = context
        Token.set_extension("rumorph", getter=self.analysis, force=True)

    def __call__(self, doc: Doc) -> Doc:
        """
        Разметка документа: часть речи, признаки и лемма каждого токена

        Аргументы:
            doc (Doc): Документ

        Вывод:
            Doc: Тот же документ
        """
        for sentence in sentences(doc):
            tokens = [token for token in sentence if not token.is_space]
            words = [token.text for token in tokens]
            if self.context:
                analyses = self.analyzer.analyze_sentence(words)
            else:
                analyses = self.analyzer.analyze_many(words)
            for token, candidates in zip(tokens, analyses, strict=True):
                if candidates:
                    token.pos_ = candidates[0].pos
                    token.morph = MorphAnalysis(doc.vocab, candidates[0].feats)
                    token.lemma_ = candidates[0].lemma
        for token in doc:
            if token.is_space:
                token.pos_ = "SPACE"
                token.lemma_ = token.text
        return doc

    def analysis(self, token: Token) -> Analysis | None:
        """
        Разбор ruMorph токена (`token._.rumorph`): разбор словоформы с леммой, частью речи
        и признаками токена; через него доступны формы слова. Вероятность - без контекста

        Аргументы:
            token (Token): Токен

        Вывод:
            Analysis | None: Разбор или None, если у словоформы нет такого разбора
        """
        feats = token.morph.to_dict()
        for analysis in self.analyzer.analyze(token.text):
            if (analysis.lemma, analysis.pos, analysis.feats) == (token.lemma_, token.pos_, feats):
                return analysis
        return None


def sentences(doc: Doc) -> Iterator[list[Token]]:
    """
    Предложения документа: размеченные или до знака конца предложения

    Аргументы:
        doc (Doc): Документ

    Вывод:
        Iterator[list[Token]]: Токены предложений
    """
    if doc.has_annotation("SENT_START"):
        for sentence in doc.sents:
            yield list(sentence)
        return
    chunk = []
    for token in doc:
        chunk.append(token)
        if len(chunk) >= MAX_CHUNK or (token.text and set(token.text) <= SENTENCE_END):
            yield chunk
            chunk = []
    if chunk:
        yield chunk
