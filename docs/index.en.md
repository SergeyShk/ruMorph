# ruMorph

**ruMorph** is a morphological analyzer of Russian: a Rust core with a Python API. For a word form the analyzer finds its analyses in the OpenCorpora dictionary - the initial form (lemma), the part of speech and the [Universal Dependencies](https://universaldependencies.org/) features - orders them by probability and builds the forms of the word.

!!! note "Early version"
    Before 1.0 the API may change between 0.x versions, changes are described in the release notes.

## Features

<div class="grid cards" markdown>

-   :material-magnify:{ .lg .middle } __Analysis__

    ---

    Lemma, UD part of speech and features, probability of the analysis; lists of words and lemmatization without the GIL

    [:octicons-arrow-right-24: Analyzer](analyzer.md)

-   :material-tag-text-outline:{ .lg .middle } __Universal Dependencies__

    ---

    Parts of speech, features and lemmas in the SynTagRus conventions; numbers, punctuation and Latin words by rules, unknown words by their ending

    [:octicons-arrow-right-24: Word analysis](analysis.md)

-   :material-format-letter-case:{ .lg .middle } __Inflection__

    ---

    Forms of the lexeme, the form by UD features, agreement with a number

    [:octicons-arrow-right-24: Inflection](inflection.md)

-   :material-swap-horizontal:{ .lg .middle } __Migrating from pymorphy__

    ---

    Correspondence of pymorphy2 and pymorphy3 calls, parts of speech and grammemes

    [:octicons-arrow-right-24: Migrating from pymorphy](migration.md)

-   :material-speedometer:{ .lg .middle } __Speed and memory__

    ---

    Analysis, lemmatization and inflection compared with pymorphy3, dictionary loading and memory

    [:octicons-arrow-right-24: Speed and memory](performance.md)

-   :material-alert-circle-outline:{ .lg .middle } __Exceptions__

    ---

    Built-in Python exceptions and `FormNotFoundError` for missing forms

    [:octicons-arrow-right-24: Exceptions](exceptions.md)

</div>

## Installation

Python 3.11 or newer is required.

``` bash
pip install rumorph
```

Prebuilt wheels and building from source are described on the [Installation](installation.md) page.

## Quick start

=== "Code"

    ``` python
    from rumorph import Analyzer

    morph = Analyzer()
    morph.lemmatize(["Мы", "стали", "друзьями"])
    [(a.lemma, a.pos, a.feats.get("Case"), round(a.prob, 3)) for a in morph.analyze("стали")][:2]
    cat = morph.analyze("кошка")[0]
    cat.inflect(Case="Gen", Number="Plur"), morph.analyze("рубль")[0].agree(5)
    ```

=== "Result"

    ```
    ['мы', 'стать', 'друг']
    [('стать', 'VERB', None, 0.975), ('сталь', 'NOUN', 'Gen', 0.011)]
    ('кошек', 'рублей')
    ```

??? note "Project structure"

    *   **crates/rumorph** - the Rust core of the analyzer: dictionary format, building, analysis and inflection
    *   **crates/rumorph-build** - building the dictionary from the pymorphy3-dicts-ru export and converting OpenCorpora tags to UD
    *   **crates/rumorph-py** - the Python module `rumorph._rumorph` on PyO3
    *   **python/rumorph** - the Python package
    *   **scripts** - exporting the pymorphy3-dicts-ru dictionary
    *   **docs** - MkDocs documentation in Russian and English
    *   **tests** - tests of the Python package
