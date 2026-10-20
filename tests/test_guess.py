import math
import pickle

import pytest

from rumorph import Analyzer


@pytest.fixture(scope="module")
def morph():
    return Analyzer()


@pytest.mark.parametrize(
    ("word", "lemma", "pos"),
    [
        ("зумерами", "зумер", "NOUN"),
        ("ковидом", "ковид", "NOUN"),
        ("чатботу", "чатбот", "NOUN"),
        ("загуглил", "загуглить", "VERB"),
        ("хайповый", "хайповый", "ADJ"),
    ],
)
def test_lowercase_guess(morph, word, lemma, pos):
    analyses = morph.analyze(word)
    assert (analyses[0].lemma, analyses[0].pos) == (lemma, pos)
    assert {a.source for a in analyses} == {"guess"}
    assert {a.word for a in analyses} == {word}
    assert math.isclose(sum(a.prob for a in analyses), 1.0)
    assert not morph.is_known(word)


def test_capitalized_guess(morph):
    analyses = morph.analyze("Мартова")
    first = analyses[0]
    assert (first.word, first.lemma, first.pos, first.feats["NameType"]) == (
        "Мартова",
        "мартов",
        "PROPN",
        "Sur",
    )
    assert sum(a.prob for a in analyses if a.pos == "PROPN") > 0.8
    assert [form.word for form in first.forms()][:3] == ["Мартов", "Мартова", "Мартову"]
    assert first.inflect(Case="Dat") == "Мартову"
    # Нарицательные разборы тоже есть
    assert any(a.pos != "PROPN" for a in analyses)


def test_guessed_inflection(morph):
    assert morph.analyze("зумер")[0].agree(5) == "зумеров"
    assert morph.analyze("загуглить")[0].inflect(Tense="Past", Gender="Fem") == "загуглила"
    assert {form.lemma for form in morph.analyze("чатботу")[0].forms()} == {"чатбот"}


@pytest.mark.parametrize("word", ["ВаЙб", "ЗуМер", "вайб1"])
def test_not_guessed(morph, word):
    assert morph.analyze(word) == []


def test_order_of_sources(morph):
    assert morph.analyze("мама")[0].source == "dictionary"
    assert morph.analyze("НКО")[0].source == "rule"
    assert morph.analyze("2026")[0].source == "rule"
    assert morph.analyze("Зумер")[0].source == "guess"


def test_pickled_guesses(morph):
    analyses = morph.analyze("Мартова") + morph.analyze("зумерами")
    analyses += analyses[0].forms()
    restored = pickle.loads(pickle.dumps(analyses))
    assert restored == analyses
    assert [form.word for form in restored[0].forms()] == [
        form.word for form in analyses[0].forms()
    ]
