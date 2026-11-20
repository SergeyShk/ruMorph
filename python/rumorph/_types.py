# Морфологический анализатор русского языка (ruMorph)
#
# Copyright (C) 2026
# Автор: Шкарин Сергей <kouki.sergey@gmail.com>
# URL: <https://github.com/SergeyShk/ruMorph>

"""Типы описания словаря"""

from typing import Required, TypedDict


class CorpusInfo(TypedDict):
    """Корпус OpenCorpora, на котором обучена контекстная модель: сайт и ревизия"""

    source: str
    revision: str


class ContextInfo(TypedDict, total=False):
    """Обучение контекстной модели: трибанки UD, релиз, части, корпус и лицензия"""

    treebanks: list[str]
    release: str
    parts: list[str]
    corpus: CorpusInfo
    license: str


class ParadigmsInfo(TypedDict, total=False):
    """Обучение модели класса парадигмы: ревизия словаря OpenCorpora и выгрузка Викисловаря, на словах
    которых она обучена, число слов и лицензия"""

    opencorpora_revision: str
    wiktionary_dump: str
    words: int
    license: str


class FrequenciesInfo(TypedDict, total=False):
    """Частоты словаря: трибанки UD, релиз и лицензия"""

    treebanks: list[str]
    release: str
    license: str


class WiktionaryInfo(TypedDict, total=False):
    """Лексемы Викисловаря: дата выгрузки, файл снимка, его SHA-256, число лексем и лицензия"""

    dump: str
    file: str
    sha256: str
    lexemes: int
    license: str


class DictionaryInfo(TypedDict, total=False):
    """Описание словаря - значение Analyzer.dictionary_info"""

    source: str
    opencorpora_revision: str
    corpus_revision: str
    wiktionary: WiktionaryInfo
    frequencies: FrequenciesInfo
    context: ContextInfo
    paradigms: ParadigmsInfo
    license: str
    word_forms: Required[int]
    checksum: Required[int]
