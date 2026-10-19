"""Словарь собран из pymorphy3-dicts-ru: на словарных словах разборы OpenCorpora совпадают с pymorphy3"""

from collections import Counter

import pymorphy3
import pytest
from pymorphy3.units.by_lookup import DictionaryAnalyzer

from rumorph import Analyzer


@pytest.fixture(scope="module")
def pymorphy():
    return pymorphy3.MorphAnalyzer()


@pytest.fixture(scope="module")
def words(pymorphy):
    """Частые неоднозначные словоформы с частотами разборов, их варианты с «е» вместо «ё» и с заглавной"""
    frequent = sorted(
        {key.split(":", 1)[0] for key in pymorphy.prob_estimator.p_t_given_w.iterkeys()}
    )
    return (
        frequent
        + [word.replace("ё", "е") for word in frequent if "ё" in word]
        + [word.capitalize() for word in frequent[::20]]
    )


def dictionary_parses(pymorphy, word):
    parses = pymorphy.parse(word)
    if all(
        len(parse.methods_stack) == 1 and isinstance(parse.methods_stack[0][0], DictionaryAnalyzer)
        for parse in parses
    ):
        return parses
    return None


def test_same_analyses(pymorphy, words):
    """Разборы OpenCorpora до перевода в UD: словоформа, тег и начальная форма парадигмы"""
    morph = Analyzer()
    checked = Counter()
    for word in words:
        parses = dictionary_parses(pymorphy, word)
        if parses is None:
            checked["not in dictionary"] += 1
            continue
        theirs = sorted((p.word, str(p.tag), p.normal_form) for p in parses)
        assert sorted(morph._opencorpora(word)) == theirs, word
        checked["same analyses"] += 1
    assert checked["same analyses"] > 40_000
