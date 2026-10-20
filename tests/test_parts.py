import math
import pickle

import pytest

from rumorph import Analyzer


@pytest.fixture(scope="module")
def morph():
    return Analyzer()


@pytest.mark.parametrize(
    ("word", "lemma", "pos", "case"),
    [
        ("крэш-тестов", "крэш-тест", "NOUN", "Gen"),
        ("пин-кодов", "пин-код", "NOUN", "Gen"),
        ("Мак-Киннона", "мак-киннон", "PROPN", "Gen"),
        ("Азяш-Уфимского", "азяш-уфимский", "ADJ", "Gen"),
        ("10-летний", "10-летний", "ADJ", "Nom"),
    ],
)
def test_hyphenated(morph, word, lemma, pos, case):
    first = morph.analyze(word)[0]
    assert (first.word, first.lemma, first.pos, first.feats["Case"], first.source) == (
        word,
        lemma,
        pos,
        case,
        "guess",
    )
    assert math.isclose(sum(a.prob for a in morph.analyze(word)), 1.0)


@pytest.mark.parametrize(
    ("word", "spelled", "lemma"),
    [
        ("грязно-желтый", "грязно-жёлтый", "грязно-жёлтый"),
        ("бело-зеленого", "бело-зелёного", "бело-зелёный"),
        ("бизнес-счет", "бизнес-счёт", "бизнес-счёт"),
        ("экс-королем", "экс-королём", "экс-король"),
    ],
)
def test_yo_in_dictionary_part(morph, word, spelled, lemma):
    first = morph.analyze(word)[0]
    assert (first.word, first.lemma) == (spelled, lemma)
    assert first.forms()


def test_parts_probabilities(morph):
    # У словарных разборов части нет частот: доли поровну, а не нули
    for word in ("суперсем", "квази-совершенного"):
        assert math.isclose(sum(a.prob for a in morph.analyze(word)), 1.0)


def test_aspect_prefixes(morph):
    # Глагол с «про» и «недо» меняет вид, поэтому угадывается по окончанию, а не по слову без приставки
    for word in ("проплатил", "прозреют"):
        assert morph.analyze(word)[0].feats["Aspect"] == "Perf"


@pytest.mark.parametrize("word", ["5-Б", "10-А", "9-В", "кто-то-то", "К-за", "iPhone-ы"])
def test_no_parts(morph, word):
    assert [(a.pos, a.source) for a in morph.analyze(word)] == [("X", "rule")]


def test_particle_chain(morph):
    # Длинная цепочка частиц не разбирается и не уводит в глубокую рекурсию
    assert [a.pos for a in morph.analyze("кто" + "-то" * 12000)] == ["X"]


def test_capitalized_forms(morph):
    pin = morph.analyze("ПИН-КОДОВ")[0]
    assert pin.inflect(Case="Dat") == "ПИН-КОДАМ"


def test_particle_forms(morph):
    # Формы слова с частицей - формы первой части с той же частицей
    particle = morph.analyze("этакому-то")[0]
    assert particle.lemma == "этакий-то"
    assert {"этакий-то", "этакого-то", "этакому-то"} <= {form.word for form in particle.forms()}
    assert morph.analyze("смотри-ка")[0].inflect(Number="Plur") == "смотрите-ка"
    assert morph.analyze("Смотри-КА")[0].inflect(Number="Plur") == "Смотрите-КА"
    # Повторы разборов слова с частицей сливаются
    assert len(morph.analyze("орешь-то")) == 1


def test_hyphenated_forms(morph):
    tests = morph.analyze("крэш-тестов")[0]
    assert [form.word for form in tests.forms()][:3] == ["крэш-тест", "крэш-теста", "крэш-тесту"]
    assert morph.analyze("пин-код")[0].agree(5) == "пин-кодов"
    assert morph.analyze("Мак-Киннона")[0].inflect(Case="Dat") == "Мак-Киннону"


def test_hyphenated_abbreviation(morph):
    first = morph.analyze("ГУ-ВШЭ")[0]
    assert (first.word, first.lemma, first.pos, first.feats["Abbr"]) == (
        "ГУ-ВШЭ",
        "гу-вшэ",
        "PROPN",
        "Yes",
    )


def test_particles(morph):
    assert [(a.lemma, a.pos) for a in morph.analyze("сколь-нибудь")][:1] == [
        ("сколь-нибудь", "ADV")
    ]
    assert morph.analyze("смотри-ка")[0].pos == "VERB"


@pytest.mark.parametrize("word", ["по-бурятски", "по-молодому", "по-лисьи"])
def test_po_adverbs(morph, word):
    assert [(a.word, a.lemma, a.pos, dict(a.feats), a.source) for a in morph.analyze(word)] == [
        (word, word, "ADV", {"Degree": "Pos"}, "rule")
    ]


@pytest.mark.parametrize(
    ("word", "lemma", "pos"),
    [
        ("экодомов", "экодом", "NOUN"),
        ("микротромбов", "микротромб", "NOUN"),
        ("антиковидный", "антиковидный", "ADJ"),
    ],
)
def test_prefixed(morph, word, lemma, pos):
    first = morph.analyze(word)[0]
    assert (first.lemma, first.pos, first.source) == (lemma, pos, "guess")
    assert all(form.word.startswith(word[:3]) for form in first.forms())


@pytest.mark.parametrize("word", ["-", "-а", "а-", "а--б", "a-б", "кот-1"])
def test_not_parts(morph, word):
    assert all(a.source != "guess" or "-" not in a.word for a in morph.analyze(word))


def test_pickled_parts(morph):
    analyses = (
        morph.analyze("крэш-тестов") + morph.analyze("Мак-Киннона") + morph.analyze("экодомов")
    )
    analyses += morph.analyze("сколь-нибудь") + morph.analyze("ГУ-ВШЭ")
    analyses += analyses[0].forms()
    assert pickle.loads(pickle.dumps(analyses)) == analyses


@pytest.mark.parametrize(
    ("word", "lemma", "pos"),
    [
        ("IT-компания", "it-компания", "NOUN"),
        ("PR-менеджера", "pr-менеджер", "NOUN"),
        # Обе части изменяются: лемма из лемм частей
        ("дивана-кровати", "диван-кровать", "NOUN"),
        ("человека-паука", "человек-паук", "NOUN"),
        ("думал-думал", "думать-думать", "VERB"),
        # Первая часть в начальной форме не меняет лемму
        ("фитнес-тренера", "фитнес-тренер", "NOUN"),
        # Протяжка и заикание
        ("Что-о", "что", "SCONJ"),
        ("Н-нет", "нет", "VERB"),
    ],
)
def test_hyphenated_lemmas(morph, word, lemma, pos):
    analysis = morph.analyze(word)[0]
    assert (analysis.lemma, analysis.pos, analysis.word) == (lemma, pos, word)


def test_both_parts_inflected_have_no_forms(morph):
    sofa = morph.analyze("дивана-кровати")[0]
    assert [form.word for form in sofa.forms()] == ["дивана-кровати"]


@pytest.mark.parametrize(
    ("word", "forms"),
    [("кот-то", ["кот-то", "кота-то", "коту-то"]), ("это-то", ["это-то", "этого-то", "этому-то"])],
)
def test_particle_forms_of_dictionary_words(morph, word, forms):
    # Частица после словарного слова: формы строятся по первой части, а не по целому слову
    assert [form.word for form in morph.analyze(word)[0].forms()][:3] == forms
