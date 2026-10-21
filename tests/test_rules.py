import math
import pickle

import pytest

from rumorph import Analyzer, FormNotFoundError


@pytest.fixture(scope="module")
def morph():
    return Analyzer()


def summary(morph, word):
    return [(a.word, a.lemma, a.pos, dict(a.feats)) for a in morph.analyze(word)]


@pytest.mark.parametrize(
    ("word", "pos"),
    [
        (",", "PUNCT"),
        ("«", "PUNCT"),
        ("...", "PUNCT"),
        ("—", "PUNCT"),
        ("<", "PUNCT"),
        ("%", "SYM"),
        ("№", "SYM"),
        ("$", "SYM"),
        ("−", "SYM"),
        ("♥", "SYM"),
        ("😀", "SYM"),
        ("❤\ufe0f", "SYM"),
        ("🏴\U000e0067\U000e0062\U000e0073\U000e0063\U000e0074\U000e007f", "SYM"),
    ],
)
def test_punctuation_and_symbols(morph, word, pos):
    assert summary(morph, word) == [(word, word, pos, {})]
    assert morph.analyze(word)[0].source == "rule"
    assert not morph.is_known(word)


@pytest.mark.parametrize(
    ("word", "first"),
    [
        ("2026", "ADJ"),
        ("1812", "ADJ"),
        ("1900", "ADJ"),
        ("2000", "ADJ"),
        ("1500", "NUM"),
        ("2100", "NUM"),
        ("15", "NUM"),
        ("123456", "NUM"),
        ("3,14", "NUM"),
        ("1.5", "NUM"),
        ("-5", "NUM"),
        ("10 000", "NUM"),
        ("10\u00a0000", "NUM"),
        ("10\u2009000", "NUM"),
        ("10\u2007000", "NUM"),
        ("17:04", "NUM"),
        ("2/3", "NUM"),
        ("01.02.2003", "NUM"),
    ],
)
def test_numbers(morph, word, first):
    analyses = morph.analyze(word)
    assert analyses[0].pos == first
    assert {(a.word, a.lemma) for a in analyses} == {(word, word)}
    assert math.isclose(sum(a.prob for a in analyses), 1.0)
    cardinal = next(a for a in analyses if a.pos == "NUM")
    assert cardinal.feats == {"NumForm": "Digit", "NumType": "Card"}
    if word.isdigit():
        ordinal = next(a for a in analyses if a.pos == "ADJ")
        assert ordinal.feats == {"NumForm": "Digit", "NumType": "Ord"}
    else:
        assert len(analyses) == 1


@pytest.mark.parametrize(("word", "feats"), [
    ("2005-го", {"Case": "Gen", "Gender": "Masc", "Number": "Sing"}),
    ("2005-ГО", {"Case": "Gen", "Gender": "Masc", "Number": "Sing"}),
    ("2005-м", {"Case": "Loc", "Gender": "Masc", "Number": "Sing"}),
    ("2010-е", {"Animacy": "Inan", "Case": "Acc", "Number": "Plur"}),
    ("90-х", {"Case": "Gen", "Number": "Plur"}),
    ("90-е", {"Animacy": "Inan", "Case": "Acc", "Number": "Plur"}),
    ("1-го", {"Case": "Gen", "Gender": "Masc", "Number": "Sing"}),
])  # fmt: skip
def test_numeral_with_ending_first(morph, word, feats):
    first = morph.analyze(word)[0]
    assert (first.word, first.pos) == (word, "ADJ")
    assert first.feats == {**feats, "NumForm": "Combi", "NumType": "Ord"}


def test_numeral_with_ending(morph):
    analyses = morph.analyze("2005-го")
    assert {(a.lemma, a.pos, a.feats["NumForm"], a.feats["NumType"]) for a in analyses} == {
        ("2005-й", "ADJ", "Combi", "Ord")
    }
    assert "Gen" in {a.feats["Case"] for a in analyses}
    assert [form.word for form in analyses[0].forms()][:3] == ["2005-й", "2005-го", "2005-му"]
    assert analyses[0].inflect(Case="Loc", Gender="Masc") == "2005-м"
    # Окончание заглавными: формы строятся в нижнем регистре и пишутся капсом, как слово
    upper = morph.analyze("2005-ГО")[0]
    assert [form.word for form in upper.forms()][:2] == ["2005-Й", "2005-ГО"]
    assert (upper.inflect(Case="Dat"), upper.inflect()) == ("2005-МУ", "2005-ГО")
    assert {a.lemma for a in morph.analyze("1990-х")} == {"1990-й"}
    # Короткое окончание без образца - количественное
    assert summary(morph, "5-ти") == [
        ("5-ти", "5", "NUM", {"NumForm": "Digit", "NumType": "Card"})
    ]
    assert summary(morph, "5-ТИ") == [
        ("5-ТИ", "5", "NUM", {"NumForm": "Digit", "NumType": "Card"})
    ]


def test_roman_numerals(morph):
    assert summary(morph, "XX") == [
        ("XX", "XX", "ADJ", {"NumForm": "Roman", "NumType": "Ord"}),
        ("XX", "xx", "X", {"Foreign": "Yes"}),
    ]
    assert morph.analyze("I")[1].pos == "X"
    # Одиночные «C» и «L» чаще латиница, «X» и «V» - римское число
    assert [morph.analyze(letter)[0].pos for letter in "CLXV"] == ["X", "X", "ADJ", "ADJ"]
    # Не римское число - латиница
    assert summary(morph, "IIII") == [("IIII", "iiii", "X", {"Foreign": "Yes"})]
    # Строчные с «c» и «l» - латинские слова
    assert [morph.analyze(word)[0].pos for word in ("ci", "cl", "xix")] == ["X", "X", "ADJ"]
    # Латинская буква со словом через дефис - не римское порядковое
    assert all(a.feats.get("NumForm") != "Roman" for a in morph.analyze("X-хромосома"))
    assert morph.analyze("XIX-го")[0].feats["NumForm"] == "Roman"


@pytest.mark.parametrize("word", ["iPhone", "and", "TW600", "e-mail", "O'Neil"])
def test_latin(morph, word):
    assert summary(morph, word) == [(word, word.lower(), "X", {"Foreign": "Yes"})]


def test_initials(morph):
    analyses = morph.analyze("А.")
    assert [(a.word, a.lemma, a.feats["NameType"]) for a in analyses] == [
        ("А.", "а", "Giv"),
        ("А.", "а", "Pat"),
        ("А.", "а", "Sur"),
    ]
    assert {a.pos for a in analyses} == {"PROPN"}
    assert {a.feats["Abbr"] for a in analyses} == {"Yes"}
    assert [a.pos for a in morph.analyze("Ъ.")] == ["X"]
    # Заглавная буква без точки - тоже инициал
    initial = morph.analyze("Х")[0]
    assert (initial.lemma, initial.feats["NameType"]) == ("х", "Giv")


def test_abbreviation_with_dot(morph):
    assert {(a.word, a.lemma, a.pos) for a in morph.analyze("им.")} == {("им.", "имя", "NOUN")}
    assert {(a.word, a.lemma) for a in morph.analyze("Им.")} == {("Им.", "имя")}
    analyses = morph.analyze("т.")
    assert analyses[0].lemma == "так"
    assert math.isclose(sum(a.prob for a in analyses), 1.0)
    assert all(a.feats["Abbr"] == "Yes" for a in analyses)
    assert [a.pos for a in morph.analyze("вайб.")] == ["X"]


def test_capitalized_abbreviation(morph):
    assert summary(morph, "НКО") == [
        ("НКО", "нко", "PROPN", {"Abbr": "Yes", "NameType": "Com"}),
        ("НКО", "нко", "NOUN", {"Abbr": "Yes"}),
    ]
    # Слово словаря капсом разбирается по словарю
    assert morph.analyze("МАМА")[0].source == "dictionary"


@pytest.mark.parametrize(
    "word", [" ", "\n", "\u00a0", "\u200b", "\ufeff", "\x00", "\u200d", "а б", "\u00ad"]
)
def test_not_tokens(morph, word):
    assert morph.analyze(word) == []


def test_rule_analyses_without_lexeme(morph):
    number = morph.analyze("2026")[0]
    assert [(form.word, form.pos, form.prob) for form in number.forms()] == [("2026", "ADJ", 1.0)]
    assert number.inflect(NumType="Ord") == "2026"
    with pytest.raises(FormNotFoundError):
        number.inflect(NumType="Card")
    with pytest.raises(FormNotFoundError):
        number.agree(5)


def test_pickled_rule_analyses(morph):
    analyses = [
        a
        for word in ("2026", "2005-го", "XX", "iPhone", "А.", "им.", "НКО", "%")
        for a in morph.analyze(word)
    ]
    analyses += morph.analyze("2005-го")[0].forms()
    assert pickle.loads(pickle.dumps(analyses)) == analyses


def test_lemmatize(morph):
    assert morph.lemmatize(["В", "2008", "г.", "Apple", "выпустила", "iPhone", ",", "а", "НКО", "—", "нет"]) == [
        "в", "2008", "г", "apple", "выпустить", "iphone", ",", "а", "нко", "—", "нет",
    ]  # fmt: skip


@pytest.mark.parametrize(
    ("word", "lemma", "pos", "feats"),
    [
        ("1990-2000", "1990-2000", "NUM", {"NumType": "Card"}),
        ("37–38", "37–38", "NUM", {"NumType": "Card"}),
        ("10-15-го", "10-15-й", "ADJ", {"NumType": "Ord", "Case": "Gen"}),
        ("20-30-е", "20-30-й", "ADJ", {"NumType": "Ord"}),
        ("1840–1860-х", "1840–1860-й", "ADJ", {"NumType": "Ord", "Number": "Plur"}),
        ("5-6-ти", "5-6", "NUM", {"NumType": "Card"}),
        ("XIX-го", "XIX", "ADJ", {"NumForm": "Roman"}),
        ("II-III", "II-III", "ADJ", {"NumForm": "Roman"}),
        ("xix", "xix", "ADJ", {"NumForm": "Roman"}),
    ],
)
def test_ranges_and_roman(morph, word, lemma, pos, feats):
    analysis = morph.analyze(word)[0]
    assert (analysis.lemma, analysis.pos) == (lemma, pos)
    assert feats.items() <= analysis.feats.items()


def test_two_letter_roman(morph):
    # «CV», «XL» - чаще латинские сокращения, «XIV» - римское число
    assert morph.analyze("CV")[0].pos == "X"
    assert morph.analyze("XIV")[0].pos == "ADJ"
