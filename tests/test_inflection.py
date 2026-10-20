import pickle
from concurrent.futures import ThreadPoolExecutor

import pytest

from rumorph import Analysis, Analyzer, FormNotFoundError


@pytest.fixture(scope="module")
def morph():
    return Analyzer()


def analysis(morph: Analyzer, word: str, **feats: str) -> Analysis:
    """Самый вероятный разбор словоформы с признаками feats"""
    return next(a for a in morph.analyze(word) if feats.items() <= a.feats.items())


def test_forms(morph):
    cat = analysis(morph, "кошкам")
    forms = cat.forms()
    assert [form.word for form in forms] == [
        "кошка", "кошки", "кошке", "кошку", "кошкой", "кошкою", "кошке",
        "кошки", "кошек", "кошкам", "кошек", "кошками", "кошках",
    ]  # fmt: skip
    assert {(form.lemma, form.pos, form.prob, form.source) for form in forms} == {
        ("кошка", "NOUN", 1.0, "dictionary")
    }
    assert forms[5].labels == {"V-oy"}
    assert len(analysis(morph, "читать").forms()) == 132


@pytest.mark.parametrize(
    ("word", "lemma", "forms"),
    [
        # Лексема - формы с той же леммой: женская фамилия отдельно от мужской
        (
            "Ивановой",
            "иванова",
            ["иванова", "ивановой", "ивановой", "иванову", "ивановой", "ивановой"],
        ),
        ("наибольшую", "наибольший", ["наибольший", "наибольшего", "наибольшему"]),
        ("ёж", "ёж", ["ёж", "ежа", "ежу"]),
    ],
)
def test_forms_share_lemma(morph, word, lemma, forms):
    words = [form.word for form in morph.analyze(word)[0].forms()]
    assert words[: len(forms)] == forms
    assert {form.lemma for form in morph.analyze(word)[0].forms()} == {lemma}


@pytest.mark.parametrize(
    "word", ["стали", "кошкою", "лесу", "читающего", "красивее", "ежи", "денег", "РФ"]
)
def test_forms_round_trip(morph, word):
    for parsed in morph.analyze(word):
        forms = parsed.forms()
        assert parsed.word in {form.word for form in forms}
        for form in forms:
            assert any(
                (other.lemma, other.pos, other.feats) == (form.lemma, form.pos, form.feats)
                for other in morph.analyze(form.word)
            ), form


@pytest.mark.parametrize(
    ("word", "source_feats", "feats", "expected"),
    [
        ("кошка", {}, {"Case": "Gen"}, "кошки"),
        ("кошка", {}, {"Case": "Gen", "Number": "Plur"}, "кошек"),
        ("кошка", {}, {"Case": "Ins"}, "кошкой"),
        ("кошкою", {}, {"Number": "Plur"}, "кошками"),
        ("лес", {}, {"Case": "Loc"}, "лесе"),
        ("лес", {}, {"Case": "Par"}, "лесу"),
        ("читал", {"VerbForm": "Fin"}, {"Gender": "Fem"}, "читала"),
        ("читал", {"VerbForm": "Fin"}, {"Tense": "Pres", "Person": "3"}, "читает"),
        ("читал", {"VerbForm": "Fin"}, {"VerbForm": "Inf"}, "читать"),
        ("читал", {"VerbForm": "Fin"}, {"VerbForm": "Conv"}, "читав"),
        ("читать", {}, {"VerbForm": "Part", "Tense": "Pres", "Case": "Dat"}, "читающему"),
        ("красивый", {}, {"Gender": "Fem"}, "красивая"),
        ("красивый", {}, {"Degree": "Cmp"}, "красивее"),
        ("красивый", {}, {"Variant": "Short", "Gender": "Fem"}, "красива"),
        ("еж", {}, {"Number": "Plur"}, "ежи"),
        ("кошка", {}, {}, "кошка"),
    ],
)
def test_inflect(morph, word, source_feats, feats, expected):
    assert analysis(morph, word, **source_feats).inflect(**feats) == expected


def test_second_locative(morph):
    forest = analysis(morph, "лес")
    locative = [
        form
        for form in forest.forms()
        if form.feats.get("Case") == "Loc" and form.feats["Number"] == "Sing"
    ]
    assert [(form.word, form.labels) for form in locative] == [
        ("лесе", frozenset()),
        ("лесу", {"loc2"}),
    ]
    assert analysis(morph, "лесу", Case="Loc").inflect(Number="Plur") == "лесах"


def test_inflect_errors(morph):
    cat = analysis(morph, "кошка")
    with pytest.raises(FormNotFoundError, match="У слова «кошка» нет формы Tense=Past") as error:
        cat.inflect(Tense="Past")
    assert isinstance(error.value, LookupError)
    with pytest.raises(FormNotFoundError, match="нет формы Number=Sing"):
        analysis(morph, "ножницы").inflect(Number="Sing")
    with pytest.raises(ValueError, match="Неизвестный признак UD: Cas; есть Abbr, Animacy"):
        cat.inflect(Cas="Gen")
    with pytest.raises(ValueError, match="Неизвестное значение признака Case: Genitive; есть Acc"):
        cat.inflect(Case="Genitive")
    with pytest.raises(TypeError, match="Значение признака Case должно быть строкой, а не int"):
        cat.inflect(Case=1)


@pytest.mark.parametrize(
    ("word", "feats", "expected"),
    [
        (
            "рубль",
            {},
            [
                "рублей",
                "рубль",
                "рубля",
                "рублей",
                "рублей",
                "рублей",
                "рубль",
                "рубля",
                "рублей",
                "рубля",
            ],
        ),
        (
            "рублях",
            {},
            [
                "рублях",
                "рубле",
                "рублях",
                "рублях",
                "рублях",
                "рублях",
                "рубле",
                "рублях",
                "рублях",
                "рублях",
            ],
        ),
        (
            "кота",
            {"Case": "Acc"},
            ["котов", "кота", "котов", "котов", "котов", "котов", "кота", "кота", "котов", "кота"],
        ),
        (
            "сутки",
            {},
            [
                "суток",
                "сутки",
                "суток",
                "суток",
                "суток",
                "суток",
                "сутки",
                "суток",
                "суток",
                "суток",
            ],
        ),
        (
            "новый",
            {},
            [
                "новых",
                "новый",
                "новых",
                "новых",
                "новых",
                "новых",
                "новый",
                "новых",
                "новых",
                "новых",
            ],
        ),
        (
            "новая",
            {},
            [
                "новых",
                "новая",
                "новые",
                "новых",
                "новых",
                "новых",
                "новая",
                "новые",
                "новых",
                "новые",
            ],
        ),
        ("кино", {}, ["кино"] * 10),
    ],
)
def test_agree(morph, word, feats, expected):
    parsed = analysis(morph, word, **feats)
    assert [parsed.agree(number) for number in (0, 1, 2, 5, 11, 12, 21, 22, 112, 1002)] == expected


def test_agree_large_and_negative(morph):
    ruble = analysis(morph, "рубль")
    assert [ruble.agree(number) for number in (-1, -2, 10**30 + 1, 10**30 + 12)] == [
        "рубль", "рубля", "рубль", "рублей",
    ]  # fmt: skip


@pytest.mark.parametrize(
    ("word", "number"), [("молоко", 5), ("читать", 2), ("быстро", 2), ("он", 2)]
)
def test_agree_errors(morph, word, number):
    with pytest.raises(FormNotFoundError, match=f"не согласуется с количеством {number}"):
        morph.analyze(word)[0].agree(number)
    with pytest.raises(TypeError):
        morph.analyze(word)[0].agree(2.5)


def test_pickled_analysis_inflects(morph):
    restored = pickle.loads(pickle.dumps(morph.analyze_many(["кошкою", "стали"])))
    assert restored[0][0].inflect(Number="Plur") == "кошками"
    assert restored[1] == morph.analyze("стали")
    assert [form.word for form in restored[1][0].forms()] == [
        form.word for form in morph.analyze("стали")[0].forms()
    ]


def test_threads(morph):
    words = ["кошка", "читать", "красивый", "рубль"] * 50
    with ThreadPoolExecutor(max_workers=4) as pool:
        results = list(
            pool.map(lambda word: [form.word for form in morph.analyze(word)[0].forms()], words)
        )
    assert results == [[form.word for form in morph.analyze(word)[0].forms()] for word in words]
