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


# Короткие слова в смешанном регистре - чаще единицы измерения: их не угадать по окончанию
@pytest.mark.parametrize("word", ["вайб1", "Б" + "е" * 200, "МГц", "кГц", "дБ", "мВ"])
def test_not_guessed(morph, word):
    assert [(a.pos, a.source) for a in morph.analyze(word)] == [("X", "rule")]


@pytest.mark.parametrize(
    ("word", "lemma", "pos"),
    [
        # Заглавные внутри строчного слова и аббревиатура с суффиксом - как строчное слово
        ("прихАдите", "прихадить", "VERB"),
        ("КВНщиками", "квнщик", "NOUN"),
        ("ЗОЖное", "зожный", "ADJ"),
        ("НКФин", "нкфин", "NOUN"),
        # Слитное название - как слово с заглавной, аббревиатура с окончанием - по основе
        ("ЛайфНьюс", "лайфньюс", "PROPN"),
        ("РосПил", "роспил", "PROPN"),
        # Название, которое кончается заглавной буквой, сохраняет конец в лемме
        ("РусАЛ", "русал", "PROPN"),
        ("МЭРТа", "мэрт", "PROPN"),
    ],
)
def test_mixed_case(morph, word, lemma, pos):
    first = morph.analyze(word)[0]
    assert (first.lemma, first.pos, first.source) == (lemma, pos, "guess")


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


@pytest.mark.parametrize("word", ["Шатов", "Версилов"])
def test_surnames_singular(morph, word):
    # Угаданное имя собственное - чаще единственное число, чем множественное другого имени
    analysis = morph.analyze(word)[0]
    assert (analysis.lemma, analysis.pos, analysis.feats["Number"]) == (
        word.lower(),
        "PROPN",
        "Sing",
    )


def test_capitalized_adjective(morph):
    assert morph.analyze("Стэнфордская")[0].pos == "ADJ"


def test_long_capitals(morph):
    # Длинное слово капсом угадывается как слово, короткое - аббревиатура
    analysis = morph.analyze("ЧАТБОТАМИ")[0]
    assert (analysis.word, analysis.lemma, analysis.feats["Case"]) == (
        "ЧАТБОТАМИ",
        "чатбот",
        "Ins",
    )
    assert analysis.inflect(Case="Nom", Number="Sing") == "ЧАТБОТ"
    assert morph.analyze("НКО")[0].feats["Abbr"] == "Yes"


@pytest.mark.parametrize(
    ("word", "lemma"), [("МЭРТа", "мэрт"), ("КВНу", "квн"), ("ГАПов", "гап"), ("МУПы", "муп")]
)
def test_abbreviation_with_ending(morph, word, lemma):
    # Лемма - заглавная часть, даже без гласной
    assert morph.analyze(word)[0].lemma == lemma
    analysis = morph.analyze("МЭРТа")[0]
    assert (analysis.pos, analysis.feats["Case"]) == ("PROPN", "Gen")


def test_no_vowelless_lemma(morph):
    assert all(any(c in "аеёиоуыэюя" for c in a.lemma) for a in morph.analyze("Лем"))
