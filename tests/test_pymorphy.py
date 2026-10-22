"""Словарь собран из pymorphy3-dicts-ru: на словарных словах разборы OpenCorpora совпадают с pymorphy3"""

import math
from collections import Counter

import pymorphy3
import pytest
from pymorphy3.units.by_lookup import DictionaryAnalyzer

from rumorph import Analyzer

# Существительные, у форм единственного числа которых сборщик добавляет разбор PROPN
# (`ud::PROPER_COPIES`)
PROPER_COPIES = {
    "вселенная",
    "дума",
    "земля",
    "кремль",
    "луна",
    "солнце",
    "христос",
}


@pytest.fixture(scope="module")
def pymorphy():
    return pymorphy3.MorphAnalyzer()


@pytest.fixture(scope="module")
def unsorted():
    """pymorphy3 без оценки вероятностей: разборы в порядке словаря"""
    return pymorphy3.MorphAnalyzer(probability_estimator_cls=None)


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


def changed_by_build(parses):
    """Словоформы, разборы которых сборщик намеренно меняет: вводный союз при другом разборе,
    предикатив при кратком прилагательном среднего рода, местоимения «то», «всё», «все» поверх
    «тот» и «весь», варианты «б», «ж», «чтоб» в лексемах основных слов, имена собственные
    поверх нарицательных («Земля»)"""
    tags = [set(str(parse.tag).replace(" ", ",").split(",")) for parse in parses]
    return (
        (any({"CONJ", "Prnt"} <= tag for tag in tags) and len(parses) > 1)
        or (
            any("PRED" in tag for tag in tags)
            and any({"ADJS", "neut", "sing"} <= tag for tag in tags)
        )
        or any(parse.normal_form in ("тот", "весь") and "Apro" in parse.tag for parse in parses)
        or any(parse.normal_form in ("б", "ж", "чтоб") for parse in parses)
        or any(
            parse.normal_form in PROPER_COPIES and {"NOUN", "sing"} <= tag
            for parse, tag in zip(parses, tags, strict=True)
        )
    )


def dictionary_order(parses):
    """Порядок словаря: в записи каждого написания разборы одной леммы - группой на месте
    первого из них, несовершенный вид раньше совершенного"""
    imperfective = {(p.word, p.normal_form) for p in parses if "impf" in p.tag}
    first = {}
    for number, parse in enumerate(parses):
        first.setdefault(parse.word, number)
        first.setdefault((parse.word, parse.normal_form), number)
    return sorted(
        parses,
        key=lambda p: (
            first[p.word],
            first[(p.word, p.normal_form)],
            "perf" in p.tag and (p.word, p.normal_form) in imperfective,
        ),
    )


def expected_order(pymorphy, word, parses):
    """Порядок и вероятности ruMorph по таблице частот pymorphy3: частоты написания из запроса,
    без них - написания разбора; вероятность тега делится между разборами с той же таблицей.
    Если частоты есть только у части написаний, разбор написания без частот получает среднюю
    долю разборов с частотами"""
    table = pymorphy.prob_estimator.p_t_given_w
    lower = word.lower()
    parses = dictionary_order(parses)
    probs = [table.prob(lower, parse.tag) for parse in parses]
    own = any(probs)
    known = [own or (p.word != lower and bool(table.keys(f"{p.word}:"))) for p in parses]
    if not own:
        probs = [
            table.prob(p.word, p.tag) if has else 0 for p, has in zip(parses, known, strict=True)
        ]
    shares = [sum(q.tag == p.tag and (own or q.word == p.word) for q in parses) for p in parses]
    probs = [prob / share for prob, share in zip(probs, shares, strict=True)]
    if any(known) and not all(known):
        mean = sum(prob for prob, has in zip(probs, known, strict=True) if has) / sum(known)
        probs = [prob if has else mean for prob, has in zip(probs, known, strict=True)]
    total = sum(probs)
    if total == 0:
        return [(parse, 1 / len(parses)) for parse in parses]
    pairs = zip(parses, [prob / total for prob in probs], strict=True)
    return sorted(pairs, key=lambda pair: -pair[1])


def test_same_analyses(pymorphy, unsorted, words):
    """Разборы OpenCorpora до перевода в UD: словоформа, тег, начальная форма и вероятность"""
    morph = Analyzer()
    checked = Counter()
    for word in words:
        parses = dictionary_parses(pymorphy, word)
        if parses is None:
            checked["not in dictionary"] += 1
            continue
        if changed_by_build(parses):
            checked["changed by build"] += 1
            continue
        ours = morph._opencorpora(word)
        theirs = [(p.word, str(p.tag), p.normal_form) for p in parses]
        assert sorted(item[:3] for item in ours) == sorted(theirs), word
        checked["same analyses"] += 1
        # Доли по частотам корпуса UD с pymorphy3 не совпадают намеренно
        if morph._table_frequencies(word):
            checked["UD frequencies"] += 1
            continue
        if len({tag for _, tag, _, _ in ours}) == len(ours):
            expected = expected_order(pymorphy, word, unsorted.parse(word))
            assert [item[:3] for item in ours] == [
                (p.word, str(p.tag), p.normal_form) for p, _ in expected
            ], word
            for (*_, prob), (_, expected_prob) in zip(ours, expected, strict=True):
                assert math.isclose(prob, expected_prob, rel_tol=1e-9), word
            checked["same order"] += 1
    assert checked["same analyses"] > 40_000
    assert checked["same order"] > 0.95 * (checked["same analyses"] - checked["UD frequencies"])


def test_proper_copies():
    morph = Analyzer()
    for lemma in PROPER_COPIES:
        analyses = morph.analyze(lemma.capitalize())
        assert any(a.pos == "PROPN" and a.lemma == lemma for a in analyses), lemma
