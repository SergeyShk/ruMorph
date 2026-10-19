"""Словарь собран из pymorphy3-dicts-ru: на словарных словах разборы OpenCorpora совпадают с pymorphy3"""

import math
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


def expected_order(pymorphy, word, parses):
    """Порядок и вероятности ruMorph по таблице частот pymorphy3: частоты написания из запроса,
    без них - написания разбора; вероятность тега делится между разборами с той же таблицей"""
    table = pymorphy.prob_estimator.p_t_given_w
    lower = word.lower()
    probs = [table.prob(lower, parse.tag) for parse in parses]
    own = any(probs)
    if not own:
        probs = [0 if p.word == lower else table.prob(p.word, p.tag) for p in parses]
    shares = [sum(q.tag == p.tag and (own or q.word == p.word) for q in parses) for p in parses]
    probs = [prob / share for prob, share in zip(probs, shares, strict=True)]
    total = sum(probs)
    if total == 0:
        return [(parse, 1 / len(parses)) for parse in parses]
    pairs = zip(parses, [prob / total for prob in probs], strict=True)
    return sorted(pairs, key=lambda pair: -pair[1])


def test_same_analyses(pymorphy, words):
    """Разборы OpenCorpora до перевода в UD: словоформа, тег, начальная форма и вероятность"""
    morph = Analyzer()
    checked = Counter()
    for word in words:
        parses = dictionary_parses(pymorphy, word)
        if parses is None:
            checked["not in dictionary"] += 1
            continue
        ours = morph._opencorpora(word)
        theirs = [(p.word, str(p.tag), p.normal_form) for p in parses]
        assert sorted(item[:3] for item in ours) == sorted(theirs), word
        checked["same analyses"] += 1
        if len({tag for _, tag, _, _ in ours}) == len(ours):
            expected = expected_order(pymorphy, word, parses)
            assert [item[:3] for item in ours] == [
                (p.word, str(p.tag), p.normal_form) for p, _ in expected
            ], word
            for (*_, prob), (_, expected_prob) in zip(ours, expected, strict=True):
                assert math.isclose(prob, expected_prob, rel_tol=1e-9), word
            checked["same order"] += 1
    assert checked["same analyses"] > 40_000
    assert checked["same order"] > 0.95 * checked["same analyses"]
