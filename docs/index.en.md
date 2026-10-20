# ruMorph

**ruMorph** is a morphological analyzer of Russian: a Rust core with a Python API. For a word form the analyzer finds its analyses in the OpenCorpora dictionary - the normal form (lemma), the part of speech and the grammatical features - and orders them by probability.

!!! warning "Work in progress"
    So far the analyzer analyzes and inflects the word forms of the dictionary - the lemma, the UD part of speech and features, the probability, the forms of a word; numbers, punctuation, Latin words and abbreviations are analyzed by rules, unknown words are guessed by their ending; hyphenated and prefixed words come in the next versions. Versions `0.1.0.devN` are published on TestPyPI.

## Installation

Python 3.11 or newer is required.

``` bash
pip install rumorph
```

Prebuilt wheels and building from source are described on the [Installation](installation.md) page.

## Quick start

``` python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> morph.lemmatize(["Мы", "стали", "друзьями"])
['мы', 'стать', 'друг']
```

See the [Word analysis](analysis.md) page for more.
