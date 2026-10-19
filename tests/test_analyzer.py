import copy
import errno
import json
import math
import pickle
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

import rumorph
from rumorph import Analysis, Analyzer


@pytest.fixture(scope="module")
def morph():
    return Analyzer()


def test_default_dictionary(morph):
    assert morph.path == Path(rumorph.__file__).parent / "data" / "ru.dict"
    meta = json.loads(morph._meta())
    assert meta["source"] == "pymorphy3-dicts-ru"
    assert meta["opencorpora_revision"] == "417150"


def test_path_argument(morph):
    assert Analyzer(str(morph.path)).path == morph.path
    assert Analyzer(path=morph.path).analyze("стали") == morph.analyze("стали")


def test_missing_dictionary(tmp_path):
    path = tmp_path / "нет.dict"
    with pytest.raises(FileNotFoundError, match="Нет файла словаря") as error:
        Analyzer(path)
    assert error.value.errno == errno.ENOENT
    assert error.value.filename == str(path)


def test_directory_instead_of_dictionary(tmp_path):
    with pytest.raises((IsADirectoryError, PermissionError)) as error:
        Analyzer(tmp_path)
    assert error.value.filename == str(tmp_path)


@pytest.mark.parametrize("content", [b"", b"not a dictionary", b"RUMORPH\x00\x01\x00\x00\x00"])
def test_broken_dictionary(tmp_path, content):
    path = tmp_path / "broken.dict"
    path.write_bytes(content)
    with pytest.raises(ValueError, match="неверный формат словаря"):
        Analyzer(path)


def test_analyze(morph):
    analyses = morph.analyze("Стали")
    assert [analysis.lemma for analysis in analyses] == ["стать"] + ["сталь"] * 5
    assert {analysis.word for analysis in analyses} == {"стали"}
    assert {analysis.source for analysis in analyses} == {"dictionary"}
    probs = [analysis.prob for analysis in analyses]
    assert probs == sorted(probs, reverse=True)
    assert math.isclose(sum(probs), 1.0)
    assert probs[0] == pytest.approx(0.975, abs=1e-3)


@pytest.mark.parametrize("word", ["еж", "ёж", "ЕЖ", "Ёж"])
def test_yo_and_case(morph, word):
    assert {(analysis.word, analysis.lemma) for analysis in morph.analyze(word)} == {("ёж", "ёж")}


def test_e_matches_both_spellings(morph):
    assert {analysis.word for analysis in morph.analyze("все")} == {"все", "всё"}
    assert {analysis.word for analysis in morph.analyze("всё")} == {"всё"}


@pytest.mark.parametrize(
    ("word", "spelling", "lemma"),
    [("еще", "ещё", "ещё"), ("гнет", "гнёт", "гнуть"), ("замерзла", "замёрзла", "замёрзнуть")],
)
def test_frequencies_of_yo_spelling(morph, word, spelling, lemma):
    probs = [analysis.prob for analysis in morph.analyze(spelling)]
    assert [analysis.prob for analysis in morph.analyze(word)] == probs
    assert morph.lemmatize([word]) == [lemma]


def test_uniform_without_frequencies(morph):
    probs = [analysis.prob for analysis in morph.analyze("ёж")]
    assert probs == [1 / len(probs)] * len(probs)


@pytest.mark.parametrize("word", ["вайб", "", "ежж", "hello"])
def test_unknown_word(morph, word):
    assert morph.analyze(word) == []
    assert not morph.is_known(word)


def test_is_known(morph):
    assert morph.is_known("Ежи")
    assert morph.is_known("СТАЛИ")


def test_analyze_many(morph):
    words = ["мама", "мыла", "раму", "вайб", "Ёж"]
    assert morph.analyze_many(words) == [morph.analyze(word) for word in words]
    assert morph.analyze_many(word for word in words) == morph.analyze_many(tuple(words))
    assert morph.analyze_many([]) == []


def test_lemmatize(morph):
    words = ["Мы", "стали", "друзьями", "в", "ВАЙБЕ"]
    assert morph.lemmatize(words) == ["мы", "стать", "друг", "в", "вайбе"]
    assert morph.lemmatize(iter(words)) == morph.lemmatize(words)


@pytest.mark.parametrize("method", ["analyze_many", "lemmatize"])
def test_words_must_be_strings(morph, method):
    with pytest.raises(TypeError, match="не строка"):
        getattr(morph, method)("стали")
    with pytest.raises(TypeError, match="элемент 1 - int"):
        getattr(morph, method)(["стали", 5])
    with pytest.raises(TypeError, match="элемент 0 - bytes"):
        getattr(morph, method)([b"stali"])
    with pytest.raises(UnicodeEncodeError):
        getattr(morph, method)(["\udcff"])


def test_threads(morph):
    chunks = [["мама", "мыла", "раму"] * 1000, ["стали", "все", "ёж"] * 1000] * 4
    with ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(morph.lemmatize, chunks))
    assert results == [morph.lemmatize(chunk) for chunk in chunks]


def test_analysis(morph):
    first, second = morph.analyze("ёж")[:2]
    assert first == morph.analyze("еж")[0]
    assert first != second
    assert hash(first) == hash(morph.analyze("еж")[0])
    assert len(set(morph.analyze("стали"))) == 6
    assert repr(first) == (
        "Analysis(word='ёж', lemma='ёж', pos='NOUN', "
        "feats='Animacy=Anim|Case=Nom|Gender=Masc|Number=Sing', labels='', "
        f"prob={first.prob!r}, source='dictionary')"
    )
    assert repr(morph.analyze("мама")[0]).endswith("prob=1.0, source='dictionary')")
    assert repr(morph) == f"Analyzer(path={str(morph.path)!r})"
    with pytest.raises(AttributeError):
        first.lemma = "еж"
    with pytest.raises(TypeError):
        Analysis()


def test_ud_fields(morph):
    analysis = morph.analyze("стали")[0]
    assert analysis.pos == "VERB"
    assert analysis.feats == {
        "Aspect": "Perf",
        "Mood": "Ind",
        "Number": "Plur",
        "Tense": "Past",
        "VerbForm": "Fin",
        "Voice": "Mid",
    }
    assert analysis.labels == frozenset({"intr"})
    assert list(analysis.feats) == sorted(analysis.feats)
    feats = analysis.feats
    feats["Tense"] = "Pres"
    assert analysis.feats["Tense"] == "Past"
    assert json.loads(json.dumps(analysis.feats)) == analysis.feats
    assert copy.deepcopy(analysis) == analysis
    assert pickle.loads(pickle.dumps(analysis.feats)) == analysis.feats


@pytest.mark.parametrize(
    ("word", "lemma", "pos", "feats"),
    [
        ("который", "который", "DET", {"Case": "Nom"}),
        ("бы", "бы", "AUX", {"Mood": "Cnd"}),
        ("будет", "быть", "AUX", {"Tense": "Pres"}),
        ("однако", "однако", "ADV", {"Degree": "Pos"}),
        ("можно", "можно", "VERB", {}),
        ("кто", "кто", "PRON", {"Animacy": "Anim"}),
        ("это", "это", "PRON", {"Animacy": "Inan", "Case": "Nom"}),
        ("Ивановой", "иванова", "PROPN", {"NameType": "Sur", "Gender": "Fem"}),
        ("важнейшей", "важнейший", "ADJ", {"Degree": "Sup"}),
        ("об", "об", "ADP", {}),
        ("денег", "деньги", "NOUN", {"Number": "Plur"}),
        ("РФ", "рф", "PROPN", {"Abbr": "Yes", "NameType": "Geo"}),
    ],
)
def test_syntagrus_conventions(morph, word, lemma, pos, feats):
    analysis = morph.analyze(word)[0]
    assert (analysis.lemma, analysis.pos) == (lemma, pos)
    assert feats.items() <= analysis.feats.items()


def test_merged_analyses(morph):
    analyses = morph.analyze("так")
    keys = [(a.word, a.lemma, a.pos, dict(a.feats)) for a in analyses]
    assert len(keys) == len({repr(key) for key in keys})
    assert analyses[0].pos == "ADV"
    assert math.isclose(sum(a.prob for a in analyses), 1.0)


def test_pickle(morph):
    analyses = morph.analyze("стали")
    restored_analyses = pickle.loads(pickle.dumps(analyses))
    assert restored_analyses == analyses
    assert [a.feats for a in restored_analyses] == [a.feats for a in analyses]
    assert [a.labels for a in restored_analyses] == [a.labels for a in analyses]
    restored = pickle.loads(pickle.dumps(morph))
    assert restored.path == morph.path
    assert restored.lemmatize(["стали"]) == ["стать"]
    assert pickle.loads(pickle.dumps(Analyzer(str(morph.path)))).path == morph.path
    state = ("ёж", "ёж", "NOUN", "Case=Nom", "", 1.0, "dictionary")
    assert rumorph._rumorph._analysis(state).feats == {"Case": "Nom"}
    with pytest.raises(ValueError, match="Неизвестный источник"):
        rumorph._rumorph._analysis((*state[:-1], "guess"))
    with pytest.raises(ValueError, match="Неверные признаки"):
        rumorph._rumorph._analysis(("ёж", "ёж", "NOUN", "Case", "", 1.0, "dictionary"))
