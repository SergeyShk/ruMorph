"""Компонент spaCy rumorph: часть речи, признаки и лемма токенов по разбору ruMorph"""

import importlib.metadata
import importlib.resources
import pickle
import subprocess
import sys

import pytest

import rumorph

spacy = pytest.importorskip("spacy")


@pytest.fixture(scope="module")
def nlp():
    nlp = spacy.blank("ru")
    nlp.add_pipe("rumorph")
    return nlp


def annotation(doc):
    return [(token.lemma_, token.pos_, token.morph.to_dict()) for token in doc]


def expected(*sentences):
    morph = rumorph.Analyzer()
    return [
        (analyses[0].lemma, analyses[0].pos, analyses[0].feats)
        for words in sentences
        for analyses in morph.analyze_sentence(words)
    ]


def test_entry_point():
    points = importlib.metadata.entry_points(group="spacy_factories", name="rumorph")
    assert [point.value for point in points] == ["rumorph.spacy:create_component"]


def test_context(nlp):
    doc = nlp("Мост сделан из стали. Я читаю книги")
    assert [(token.lemma_, token.pos_) for token in doc[:4]] == [
        ("мост", "NOUN"),
        ("сделать", "VERB"),
        ("из", "ADP"),
        ("сталь", "NOUN"),
    ]
    assert str(doc[3].morph) == "Animacy=Inan|Case=Gen|Gender=Fem|Number=Sing"
    assert doc[7].morph.get("Case") == ["Acc"]


def test_sentences_by_punctuation(nlp):
    doc = nlp("Мы стали друзьями. Мост сделан из стали!")
    assert annotation(doc) == expected(
        ["Мы", "стали", "друзьями", "."], ["Мост", "сделан", "из", "стали", "!"]
    )


def test_marked_sentences():
    nlp = spacy.blank("ru")
    nlp.add_pipe("sentencizer", config={"punct_chars": ["."]})
    nlp.add_pipe("rumorph")
    doc = nlp("Мы стали! Друзьями стали.")
    assert annotation(doc) == expected(["Мы", "стали", "!", "Друзьями", "стали", "."])


def test_long_text_chunks(nlp):
    words = ["Мы", "стали", "друзьями"] * 100
    doc = nlp(" ".join(words))
    assert annotation(doc) == expected(words[:256], words[256:])


def test_long_marked_sentence_chunks():
    nlp = spacy.blank("ru")
    nlp.add_pipe("sentencizer")
    nlp.add_pipe("rumorph")
    words = ["Мы", "стали", "друзьями"] * 100
    assert annotation(nlp(" ".join(words))) == expected(words[:256], words[256:])


def test_spaces_and_invisible_tokens(nlp):
    tokens = list(nlp("Мы\n\nстали ⁠ друзьями"))
    assert [(token.pos_, token.lemma_) for token in tokens[1::2]] == [("SPACE", "\n\n"), ("", "")]
    analyses = rumorph.Analyzer().analyze_sentence(["Мы", "стали", "⁠", "друзьями"])
    assert analyses[2] == []
    assert annotation(tokens[::2]) == [
        (a[0].lemma, a[0].pos, a[0].feats) for a in (analyses[0], analyses[1], analyses[3])
    ]


def test_without_context():
    nlp = spacy.blank("ru")
    nlp.add_pipe("rumorph", config={"context": False})
    doc = nlp("Я читаю книги")
    assert doc[2].morph.get("Case") == ["Gen"]


def test_config(tmp_path):
    path = importlib.resources.files("rumorph") / "data" / "ru.dict"
    words = {"зумер": "геймер", "зечь": ["печь", "VERB"]}
    nlp = spacy.blank("ru")
    nlp.add_pipe("rumorph", config={"dictionary": str(path), "words": words})
    lemmas = ["зумер", "зечь", "нравиться"]
    assert [token.lemma_ for token in nlp("Зумерам зечет нравится")] == lemmas
    assert nlp.get_pipe_meta("rumorph").assigns == ["token.pos", "token.morph", "token.lemma"]
    nlp.to_disk(tmp_path)
    assert [token.lemma_ for token in spacy.load(tmp_path)("Зумерам зечет нравится")] == lemmas


def test_extension(nlp):
    doc = nlp("Мы стали друзьями")
    analysis = doc[2]._.rumorph
    assert isinstance(analysis, rumorph.Analysis)
    assert (analysis.lemma, analysis.inflect(Case="Dat")) == ("друг", "друзьям")
    doc[2].lemma_ = "дружба"
    assert doc[2]._.rumorph is None


def test_extension_uses_component_settings(nlp):
    own = spacy.blank("ru")
    own.add_pipe("rumorph", config={"words": {"зумер": "геймер"}})
    doc = own("Зумеры стали друзьями")
    nlp("Мы стали друзьями")
    assert doc[0]._.rumorph.source == "dictionary"
    assert nlp("Зумеры")[0]._.rumorph.source == "guess"


def test_serialization(nlp):
    own = spacy.blank("ru")
    own.add_pipe("rumorph", config={"words": {"зумер": "геймер"}})
    doc = own("Зумеры стали друзьями")
    restored = spacy.tokens.Doc(own.vocab).from_bytes(doc.to_bytes())
    assert annotation(restored) == annotation(doc)
    assert restored[0]._.rumorph.source == "dictionary"
    stored = spacy.tokens.DocBin(docs=[nlp("Мы стали друзьями")]).get_docs(nlp.vocab)
    assert next(stored)[2]._.rumorph.lemma == "друг"
    assert annotation(pickle.loads(pickle.dumps(own))("Зумеры")) == annotation(doc[:1])


def test_pickle_in_new_process():
    nlp = spacy.blank("ru")
    nlp.add_pipe("rumorph", config={"words": {"зумер": "геймер"}})
    code = (
        "import pickle, sys; doc = pickle.load(sys.stdin.buffer)('Зумеры стали'); "
        "print(doc[0].lemma_, doc[0]._.rumorph.source)"
    )
    result = subprocess.run(
        [sys.executable, "-c", code], input=pickle.dumps(nlp), capture_output=True, check=True
    )
    assert result.stdout.decode().split() == ["зумер", "dictionary"]
