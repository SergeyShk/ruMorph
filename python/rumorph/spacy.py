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

import json
from collections.abc import Iterator, Mapping
from typing import Any

from spacy.language import Language
from spacy.tokens import Doc, MorphAnalysis, Token

from ._rumorph import Analysis, Analyzer

__all__ = ["Component"]

# Предложение разбирается кусками не длиннее MAX_CHUNK токенов, а документ без разметки
# предложений делится после токена из знаков конца предложения - как в команде rumorph
SENTENCE_END = frozenset(".!?…")
MAX_CHUNK = 256
# Ключ своих настроек компонента в `Doc.user_data`: по ним `token._.rumorph` находит анализатор;
# настройки по умолчанию не пишутся - их теряют `Doc.from_docs`, `DocBin` и `Span.as_doc`
SETTINGS = "rumorph"
DEFAULT = {"dictionary": None, "words": None}
# Образец своего слова: слово или пара «слово, часть речи» (в конфиге spaCy пара - список)
Pattern = str | tuple[str, str] | list[str]

_analyzers: dict[str, Analyzer] = {}


@Language.factory(
    "rumorph",
    default_config={"dictionary": None, "words": None, "context": True},
    assigns=["token.pos", "token.morph", "token.lemma"],
)
def create_component(
    nlp: Language,
    name: str,
    dictionary: str | None,
    words: dict[str, Pattern] | None,
    context: bool,
) -> "Component":
    """
    Компонент rumorph для конвейера spaCy: `nlp.add_pipe("rumorph", config={...})`

    Аргументы:
        nlp (Language): Конвейер
        name (str): Имя компонента в конвейере
        dictionary (str | None): Путь к словарю, None - словарь пакета
        words (dict[str, Pattern] | None): Свои слова, как у `Analyzer`; пара может быть списком
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
        words (Mapping[str, Pattern] | None): Свои слова, как у `Analyzer`; пара может быть
            списком
        context (bool): Выбирать разбор с учетом соседних слов
    """

    def __init__(
        self,
        dictionary: str | None = None,
        words: Mapping[str, Pattern] | None = None,
        context: bool = True,
    ) -> None:
        patterns = None
        if words is not None:
            patterns = {word: p if isinstance(p, str) else list(p) for word, p in words.items()}
        self.settings: dict[str, Any] = {"dictionary": dictionary, "words": patterns}
        self.context = context
        self.analyzer = analyzer(self.settings)

    def __reduce__(self) -> tuple[type, tuple[Any, ...]]:
        # pickle хранит настройки, а не словарь
        return Component, (self.settings["dictionary"], self.settings["words"], self.context)

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
        if self.settings != DEFAULT:
            doc.user_data[SETTINGS] = self.settings
        else:
            doc.user_data.pop(SETTINGS, None)
        return doc


def analyzer(settings: Mapping[str, Any]) -> Analyzer:
    """
    Анализатор с настройками компонента: один на процесс для одинаковых настроек

    Аргументы:
        settings (Mapping[str, Any]): Словарь и свои слова

    Вывод:
        Analyzer: Анализатор
    """
    key = json.dumps([settings["dictionary"], settings["words"]], ensure_ascii=False)
    if key not in _analyzers:
        words = settings["words"]
        if words is not None:
            words = {word: p if isinstance(p, str) else tuple(p) for word, p in words.items()}
        _analyzers[key] = Analyzer(settings["dictionary"], words)
    return _analyzers[key]


def analysis(token: Token) -> Analysis | None:
    """
    Разбор ruMorph токена (`token._.rumorph`): разбор словоформы с леммой, частью речи
    и признаками токена по словарю компонента, разметившего документ; через него доступны
    формы слова. Вероятность - без контекста

    Аргументы:
        token (Token): Токен

    Вывод:
        Analysis | None: Разбор или None, если у словоформы нет такого разбора
    """
    settings = token.doc.user_data.get(SETTINGS, DEFAULT)
    feats = token.morph.to_dict()
    for candidate in analyzer(settings).analyze(token.text):
        if (candidate.lemma, candidate.pos, candidate.feats) == (token.lemma_, token.pos_, feats):
            return candidate
    return None


Token.set_extension("rumorph", getter=analysis, force=True)


def sentences(doc: Doc) -> Iterator[list[Token]]:
    """
    Куски предложений документа: размеченных или до знака конца предложения, не длиннее
    `MAX_CHUNK` токенов

    Аргументы:
        doc (Doc): Документ

    Вывод:
        Iterator[list[Token]]: Токены кусков
    """
    if doc.has_annotation("SENT_START"):
        for sentence in doc.sents:
            tokens = list(sentence)
            for start in range(0, len(tokens), MAX_CHUNK):
                yield tokens[start : start + MAX_CHUNK]
        return
    chunk = []
    for token in doc:
        chunk.append(token)
        if len(chunk) >= MAX_CHUNK or (token.text and set(token.text) <= SENTENCE_END):
            yield chunk
            chunk = []
    if chunk:
        yield chunk
