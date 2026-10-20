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


def test_yo_written_with_yo(morph):
    # Слово через «ё» угадывается по окончанию с «ё»
    assert [(a.word, a.lemma) for a in morph.analyze("кринжьё")][:1] == [("кринжьё", "кринжьё")]


def test_yo_written_as_e(morph):
    for word in ("пингвиненка", "пингвинёнка"):
        first = morph.analyze(word)[0]
        assert (first.word, first.lemma, first.feats["Case"]) == (
            "пингвинёнка",
            "пингвинёнок",
            "Gen",
        )
    assert [form.word for form in morph.analyze("пингвиненка")[0].forms()][:2] == [
        "пингвинёнок",
        "пингвинёнка",
    ]


def test_capitalized_prefixed_forms(morph):
    adjective = next(a for a in morph.analyze("Кринжовый") if a.pos == "ADJ")
    words = [form.word for form in adjective.forms()]
    assert "Покринжовее" in words
    assert not any(char.isupper() for word in words for char in word[1:])
    comparative = next(form for form in adjective.forms() if form.word == "Покринжовее")
    assert pickle.loads(pickle.dumps(comparative)) == comparative
    # Формы формы с приставкой сохраняют заглавную букву
    assert [form.word for form in comparative.forms()][:2] == ["Кринжовый", "Кринжового"]
    assert comparative.inflect(Degree="Pos", Case="Gen") == "Кринжового"


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
