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
    # Лексемы OpenCorpora одного слова объединены: переходный и непереходный «мешать»,
    # «брызгаю» и «брызжу» - одна лексема, с какой формы ни начать
    for words in (["мешать", "мешаемый", "мешал"], ["брызгать", "брызгаю", "брызжет"]):
        lexemes = {
            frozenset(form.word for form in analysis(morph, word).forms()) for word in words
        }
        assert len(lexemes) == 1


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
        # Переходный и непереходный «мешать» - одна лексема: причастие есть только у переходного
        ("мешать", {}, {"VerbForm": "Part", "Voice": "Pass", "Tense": "Pres"}, "мешаемый"),
        ("брызжет", {}, {"VerbForm": "Part", "Voice": "Pass", "Tense": "Pres"}, "брызгаемый"),
        (
            "мешать",
            {},
            {"VerbForm": "Part", "Voice": "Pass", "Tense": "Pres", "Variant": "Short"},
            "мешаем",
        ),
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
        # Существительные с адъективным склонением согласуются как прилагательные
        (
            "учёный",
            {},
            [
                "учёных",
                "учёный",
                "учёных",
                "учёных",
                "учёных",
                "учёных",
                "учёный",
                "учёных",
                "учёных",
                "учёных",
            ],
        ),
        (
            "учёного",
            {"Case": "Acc"},
            [
                "учёных",
                "учёного",
                "учёных",
                "учёных",
                "учёных",
                "учёных",
                "учёного",
                "учёных",
                "учёных",
                "учёных",
            ],
        ),
        (
            "столовая",
            {},
            [
                "столовых",
                "столовая",
                "столовые",
                "столовых",
                "столовых",
                "столовых",
                "столовая",
                "столовые",
                "столовых",
                "столовые",
            ],
        ),
        (
            "Толстой",
            {},
            [
                "толстых",
                "толстой",
                "толстых",
                "толстых",
                "толстых",
                "толстых",
                "толстой",
                "толстых",
                "толстых",
                "толстых",
            ],
        ),
        (
            "Иванов",
            {"Case": "Nom"},
            [
                "ивановых",
                "иванов",
                "иванова",
                "ивановых",
                "ивановых",
                "ивановых",
                "иванов",
                "иванова",
                "ивановых",
                "иванова",
            ],
        ),
        # Счетная форма родительного падежа множественного числа
        (
            "вольт",
            {"Case": "Nom"},
            [
                "вольт",
                "вольт",
                "вольта",
                "вольт",
                "вольт",
                "вольт",
                "вольт",
                "вольта",
                "вольт",
                "вольта",
            ],
        ),
        ("год", {}, ["лет", "год", "года", "лет", "лет", "лет", "год", "года", "лет", "года"]),
        (
            "человек",
            {},
            [
                "человек",
                "человек",
                "человека",
                "человек",
                "человек",
                "человек",
                "человек",
                "человека",
                "человек",
                "человека",
            ],
        ),
        (
            "человека",
            {"Case": "Acc"},
            [
                "человек",
                "человека",
                "человек",
                "человек",
                "человек",
                "человек",
                "человека",
                "человека",
                "человек",
                "человека",
            ],
        ),
    ],
)
def test_agree(morph, word, feats, expected):
    parsed = analysis(morph, word, **feats)
    assert [parsed.agree(number) for number in (0, 1, 2, 5, 11, 12, 21, 22, 112, 1002)] == expected


def test_agree_large_and_negative(morph):
    ruble = analysis(morph, "рубль")
    numbers = (-1, -2, 10**30 + 1, 10**30 + 12, 10**40 + 2, -(10**40) - 5, 2**127 - 1)
    assert [ruble.agree(number) for number in numbers] == [
        "рубль", "рубля", "рубль", "рублей", "рубля", "рублей", "рублей",
    ]  # fmt: skip
    with pytest.raises(FormNotFoundError, match=f"количеством {10**40 + 5}"):
        analysis(morph, "молоко").agree(10**40 + 5)


@pytest.mark.parametrize(
    ("word", "number"), [("молоко", 5), ("читать", 2), ("быстро", 2), ("он", 2)]
)
def test_agree_errors(morph, word, number):
    with pytest.raises(FormNotFoundError, match=f"не согласуется с количеством {number}"):
        morph.analyze(word)[0].agree(number)
    with pytest.raises(TypeError):
        morph.analyze(word)[0].agree(2.5)


@pytest.mark.parametrize("word", ["мешаемый", "передох", "брызжет", "кошкою", "лесу"])
def test_pickled_forms(morph, word):
    forms = morph.analyze(word)[0].forms()
    restored = pickle.loads(pickle.dumps(forms))
    assert restored == forms
    assert [[form.word for form in item.forms()] for item in restored] == [
        [form.word for form in item.forms()] for item in forms
    ]


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


@pytest.mark.parametrize(
    ("word", "feats", "form"),
    [
        # Повелительное без лица - 2-е лицо, а не инклюзив «сделаем»
        ("сделать", {"Mood": "Imp"}, "сделай"),
        ("сделать", {"Mood": "Imp", "Number": "Plur"}, "сделайте"),
        ("пойти", {"Mood": "Imp", "Person": "1", "Number": "Plur"}, "пойдёмте"),
        ("быть", {"Mood": "Imp"}, "будь"),
        # Без запроса одушевленности - неодушевленный винительный
        ("новый", {"Case": "Acc"}, "новый"),
        ("новые", {"Case": "Acc"}, "новые"),
        ("кот", {"Case": "Acc"}, "кота"),
        # Будущее «быть»: формы с пометой futr
        ("буду", {"Person": "3"}, "будет"),
        # Вариант написания исходной формы
        ("листья", {"Case": "Gen"}, "листьев"),
        ("листы", {"Case": "Gen"}, "листов"),
        ("человек", {"Case": "Dat", "Number": "Plur"}, "людям"),
        # От инфинитива без запроса формы глагола - личная форма, а не причастие
        ("достигнуть", {"Tense": "Past"}, "достиг"),
    ],
)
def test_inflect_prefers_requested(morph, word, feats, form):
    assert morph.analyze(word)[0].inflect(**feats) == form


def test_future_of_be(morph):
    future = next(a for a in morph.analyze("будет") if a.feats.get("Mood") == "Ind")
    assert "futr" in future.labels
    assert future.inflect(Number="Plur") == "будут"


def test_female_surname_agreement(morph):
    female = next(a for a in morph.analyze("Иванова") if a.feats.get("Gender") == "Fem")
    assert [female.agree(n) for n in (1, 2, 5, 21)] == [
        "иванова",
        "ивановы",
        "ивановых",
        "иванова",
    ]
