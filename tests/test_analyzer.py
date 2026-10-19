import json
import math
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
    with pytest.raises(FileNotFoundError, match=r"нет\.dict"):
        Analyzer(path)


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
    with pytest.raises(TypeError, match="должно быть строкой"):
        getattr(morph, method)(["стали", 5])


def test_threads(morph):
    chunks = [["мама", "мыла", "раму"] * 1000, ["стали", "все", "ёж"] * 1000] * 4
    with ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(morph.lemmatize, chunks))
    assert results == [morph.lemmatize(chunk) for chunk in chunks]


def test_analysis(morph):
    first, second = morph.analyze("ёж")[:2]
    assert first == morph.analyze("еж")[0]
    assert first != second
    assert (
        repr(first) == f"Analysis(word='ёж', lemma='ёж', prob={first.prob}, source='dictionary')"
    )
    assert repr(morph) == f"Analyzer(path={str(morph.path)!r})"
    with pytest.raises(AttributeError):
        first.lemma = "еж"
    with pytest.raises(TypeError):
        Analysis()
