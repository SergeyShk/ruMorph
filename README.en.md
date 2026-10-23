<p align="center">
  <img src="https://raw.githubusercontent.com/SergeyShk/ruMorph/master/docs/img/rumorph.png" alt="ruMorph" width="480">
</p>

<h1 align="center">ruMorph</h1>

<p align="center">
  <b>Russian Morphology</b> - a fast Russian morphological analyzer written in Rust with a Python API
</p>

<p align="center">
  <a href="https://sergeyshk.github.io/ruMorph/en/">Documentation</a> ·
  <a href="https://pypi.org/project/rumorph/">PyPI</a> ·
  <a href="https://crates.io/crates/rumorph">crates.io</a> ·
  <a href="https://github.com/SergeyShk/ruMorph/blob/master/README.md">Русский</a>
</p>

<p align="center">
  <a href="https://pypi.org/project/rumorph/"><img src="https://img.shields.io/pypi/v/rumorph?logo=pypi&logoColor=FFE873" alt="Version"></a>
  <a href="https://pypi.org/project/rumorph/"><img src="https://img.shields.io/pypi/pyversions/rumorph.svg?logo=python&logoColor=FFE873" alt="Supported Python versions"></a>
  <a href="https://crates.io/crates/rumorph"><img src="https://img.shields.io/crates/v/rumorph?logo=rust" alt="Crate"></a>
  <a href="https://github.com/SergeyShk/ruMorph/actions/workflows/ci.yml"><img src="https://github.com/SergeyShk/ruMorph/actions/workflows/ci.yml/badge.svg" alt="Build"></a>
  <a href="https://github.com/astral-sh/ruff"><img src="https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json" alt="Ruff"></a>
  <a href="https://github.com/SergeyShk/ruMorph/blob/master/LICENSE.txt"><img src="https://img.shields.io/github/license/sergeyshk/rumorph.svg" alt="License"></a>
  <a href="https://doi.org/10.5281/zenodo.23520738"><img src="https://zenodo.org/badge/DOI/10.5281/zenodo.23520738.svg" alt="DOI"></a>
</p>

---

**ruMorph** is a morphological analyzer of Russian: a Rust core with a Python API. For a word form the analyzer finds its analyses in the [OpenCorpora](https://opencorpora.org) dictionary - the initial form (lemma), the part of speech and the [Universal Dependencies](https://universaldependencies.org/) features - orders them by probability and builds the forms of the word.

Analysis and lemmatization are 6-8 times faster than in pymorphy3 with the `pymorphy3[fast]` speedup, the form by features and agreement with a number are 16-19 times faster; lists of words are analyzed without the GIL. Parts of speech, features and lemmas follow the conventions of the SynTagRus treebank, so the results can be compared with the annotation of spaCy, Stanza and other tools built on UD.

* **[Analyzer](https://sergeyshk.github.io/ruMorph/en/analyzer/)** - analyses of a word form by descending probability, analysis of word lists and lemmatization without the GIL, a dictionary lookup of a word form
* **[Sentence analysis](https://sergeyshk.github.io/ruMorph/en/analyzer/#analyze_sentence)** - the context model picks the analysis from the neighbouring words: the part of speech, lemma and features are right for 81-93 % of words against 71-82 % without context
* **[UD parts of speech and features](https://sergeyshk.github.io/ruMorph/en/analysis/#ud)** - parts of speech, features and lemmas in the SynTagRus conventions; OpenCorpora labels that have no UD feature
* **[Tokens outside the dictionary](https://sergeyshk.github.io/ruMorph/en/analysis/#rules)** - numbers, signs, Latin script, initials and abbreviations by rules
* **[Unknown words](https://sergeyshk.github.io/ruMorph/en/analysis/#guess)** - analyses by the ending, hyphenated words and words with prefixes, forms of guessed words
* **[Inflection](https://sergeyshk.github.io/ruMorph/en/inflection/)** - forms of a lexeme, a form by UD features, agreement with a number
* **[The rumorph command](https://sergeyshk.github.io/ruMorph/en/cli/)** - analysis of files and streams: lemmas, CoNLL-U, TSV and JSON Lines
* **[Migrating from pymorphy](https://sergeyshk.github.io/ruMorph/en/migration/)** - pymorphy2 and pymorphy3 calls, parts of speech and grammemes and their counterparts
* **[Comparison](https://sergeyshk.github.io/ruMorph/en/performance/)** - accuracy and speed compared with pymorphy3, mystem, spaCy, Stanza and Slovnet, dictionary loading and memory

> [!NOTE]
> Early version: before 1.0 the API may change between 0.x versions, changes are described in the release notes.

## Installation

Python 3.11 or newer is required.

```bash
pip install rumorph
```

Or with [uv](https://docs.astral.sh/uv/):

```bash
uv add rumorph
```

Wheels are built for Linux (Alpine included), macOS and Windows, the free-threaded Python 3.14t included; installing them does not require Rust, and the dictionary is part of the package. On other platforms pip builds the package from source, which requires [Rust](https://www.rust-lang.org/tools/install) 1.88 or newer. See the [Installation](https://sergeyshk.github.io/ruMorph/en/installation/) page.

For Rust there is the [`rumorph`](https://crates.io/crates/rumorph) crate with the dictionary (`cargo add rumorph`, documentation on [docs.rs](https://docs.rs/rumorph)); the `rumorph` command without Python is installed with `cargo install rumorph --features cli`.

## Quick start

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> morph.lemmatize("Мы стали друзьями в ковиде".split())
['мы', 'стать', 'друг', 'в', 'ковид']
>>> [(a.lemma, a.pos, a.feats.get("Case"), round(a.prob, 3)) for a in morph.analyze("стали")][:2]
[('стать', 'VERB', None, 0.988), ('сталь', 'NOUN', 'Gen', 0.005)]
>>> morph.analyze("кошка")[0].inflect(Case="Gen", Number="Plur")
'кошек'
>>> morph.analyze("рубль")[0].agree(5)
'рублей'
```

Create the analyzer once: it loads the dictionary when created and can then be used throughout the program.

## Features

### Word analysis

The `analyze` method returns the analyses of a word form by descending probability - `Analysis` objects with the lemma, the part of speech, the UD features, the dictionary labels, the probability and the source of the analysis. Case does not matter for the lookup but changes the shares of proper nouns («шаров» is «шар», «Шаров» is a surname), and the letter «е» may stand for «ё». Analyses are immutable: they can be compared, put into sets and pickled.

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> morph.analyze("стали")[0]
Analysis(word='стали', lemma='стать', pos='VERB', feats='Aspect=Perf|Mood=Ind|Number=Plur|Tense=Past|VerbForm=Fin|Voice=Act', labels='intr', prob=0.9877829877829878, source='dictionary')
>>> [len(analyses) for analyses in morph.analyze_many(["мама", "мыла", "раму"])]
[1, 4, 2]
>>> morph.is_known("Ежи"), morph.is_known("вайб")
(True, False)
```

The `analyze_many` and `lemmatize` methods analyze lists of words without the GIL, so several threads sharing one analyzer run in parallel; on the free-threaded Python 3.14t `analyze`, `inflect` and `agree` run in parallel too. The analyzer and the analyses can be pickled and work with `multiprocessing` and joblib.

See the documentation: [analyzer](https://sergeyshk.github.io/ruMorph/en/analyzer/), [word analysis](https://sergeyshk.github.io/ruMorph/en/analysis/).

### Sentence analysis

The `analyze_sentence` method orders the analyses of the words of a sentence using the neighbouring words: the first one is the analysis chosen by the context model, and the probability is its estimate in this sentence. The model (a BiLSTM, 0.9 MB) is trained on the UD_Russian-Taiga and UD_Russian-GSD treebanks and ships with the package dictionary; the words are the tokens of one sentence.

```python
>>> words = ["Я", "читаю", "книги"]
>>> [analyses[0].feats.get("Case") for analyses in morph.analyze_many(words)]
['Nom', None, 'Gen']
>>> [analyses[0].feats.get("Case") for analyses in morph.analyze_sentence(words)]
['Nom', None, 'Acc']
>>> morph.analyze_sentence(["Мост", "сделан", "из", "стали"])[3][0].lemma
'сталь'
```

<details>
<summary><b>Universal Dependencies parts of speech and features</b></summary>

<br>

Parts of speech and features follow [Universal Dependencies](https://universaldependencies.org/u/pos/) in the conventions of the largest Russian treebank [UD_Russian-SynTagRus](https://github.com/UniversalDependencies/UD_Russian-SynTagRus). Differences from the OpenCorpora parts of speech:

*   «который», «этот», «свой», «весь», «такой» and other pronominal adjectives - `DET`
*   «быть» and «бы» - `AUX`
*   parenthetical words «однако», «например», «конечно» - `ADV`
*   «можно», «надо», «нельзя» - `VERB` without features
*   prepositions - `ADP`, coordinating conjunctions - `CCONJ`, subordinating ones - `SCONJ`, particles - `PART`

Features: case, number, gender, animacy, person, tense, aspect, mood, voice, verb form, degree, short form, proper name type, abbreviation, possessiveness, numeral type and form, pronoun type, reflexivity, indeclinability, negation. Lemmas follow SynTagRus too: «Ивановой» - «иванова», «важнейшей» - «важнейший», «денег» - «деньги». OpenCorpora grammemes that have no UD feature - style, transitivity, the second locative and others - are in the `labels`.

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> words = ["который", "был", "конечно", "можно", "Ивановой", "денег"]
>>> [(analyses[0].lemma, analyses[0].pos) for analyses in morph.analyze_many(words)]
[('который', 'DET'), ('быть', 'AUX'), ('конечно', 'ADV'), ('можно', 'VERB'), ('иванова', 'PROPN'), ('деньги', 'NOUN')]
>>> lesu = morph.analyze("лесу")[0]
>>> lesu.feats["Case"], lesu.labels
('Loc', frozenset({'loc2'}))
```

See the [documentation](https://sergeyshk.github.io/ruMorph/en/analysis/#ud); OpenCorpora grammemes and their UD features are listed on the [Migrating from pymorphy](https://sergeyshk.github.io/ruMorph/en/migration/#grammemes) page.

</details>

<details>
<summary><b>Tokens outside the dictionary</b></summary>

<br>

Numbers, signs, Latin script and abbreviations are analyzed by rules, their `source` is `"rule"`:

*   punctuation - `PUNCT`, the symbols `%`, `$`, `№` and emoji - `SYM`
*   numbers in digits - `NUM`; an integer can also be an ordinal `ADJ`, which comes first for years
*   numbers with an ending «2005-го», «5-ти» - ordinal `ADJ` with a case or cardinal `NUM`, Roman numerals - ordinal `ADJ`
*   words in Latin script - `X` with `Foreign=Yes`
*   initials «А.» - a first name, a patronymic or a surname with `Abbr=Yes`
*   abbreviations with a dot «т.», «им.» - the analyses of the dictionary abbreviation, uppercase acronyms «НКО» - `PROPN` or `NOUN` with `Abbr=Yes`
*   adverbs «по-бурятски», «по-молодому» - `ADV`
*   other tokens «км/ч», «100%-ный» - `X` without features, the lemma is the token in lowercase

The shares of ambiguous analyses come from the frequencies in the UD Taiga and GSD treebanks, the shares of proper nouns take the case into account: «Земли» is first of all the planet (PROPN), «земли» is NOUN.

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> [(a.lemma, a.pos, a.feats["NumType"], round(a.prob, 2)) for a in morph.analyze("2026")]
[('2026', 'ADJ', 'Ord', 0.98), ('2026', 'NUM', 'Card', 0.02)]
>>> [(a.lemma, a.pos, a.feats["Case"]) for a in morph.analyze("2005-го")][:1]
[('2005-й', 'ADJ', 'Gen')]
>>> [(a.pos, a.feats, a.source) for a in morph.analyze("iPhone")]
[('X', {'Foreign': 'Yes'}, 'rule')]
```

See the [documentation](https://sergeyshk.github.io/ruMorph/en/analysis/#rules).

</details>

<details>
<summary><b>Unknown words</b></summary>

<br>

For a word in Cyrillic that is not in the dictionary, the analyzer guesses the analyses by its ending: it takes the forms of dictionary words with the same ending - their part of speech, features and inflection. The `source` of such analyses is `"guess"`.

*   A lowercase word is guessed as a common noun or another common word, a capitalized word mostly as a proper name: a surname, a first name, a place
*   A hyphenated word is analyzed by its last part: «крэш-тестов» - «крэш-тест», «Мак-Киннона» - «мак-киннон», and a word with a drawl or a stutter by the dictionary word without them: «не-е-ет» - «нет», «с-спасибо» - «спасибо»
*   A word with a known prefix - «эко», «анти», «супер», «микро» and others - is analyzed by the dictionary word without it: «экодомов» - «экодом»
*   A guessed analysis has forms, and they keep the capital letters: «Мак-Киннона» in the dative - «Мак-Киннону»
*   A word that must be analyzed exactly can be added after a sample - a dictionary word that inflects the same way: `Analyzer(words={"зумер": "геймер"})`

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> [(a.lemma, a.pos, a.feats["Case"], a.source) for a in morph.analyze("чатботу")][:1]
[('чатбот', 'NOUN', 'Dat', 'guess')]
>>> guess = morph.analyze("Мартова")[0]
>>> guess.lemma, guess.pos, guess.feats["NameType"], guess.inflect(Case="Dat")
('мартов', 'PROPN', 'Sur', 'Мартову')
>>> morph.analyze("крэш-тестов")[0].lemma, morph.analyze("экодомов")[0].lemma
('крэш-тест', 'экодом')
>>> zoomers = Analyzer(words={"зумер": "геймер"})
>>> zoomers.analyze("зумерами")[0].lemma, zoomers.is_known("зумеры")
('зумер', True)
```

See the [documentation](https://sergeyshk.github.io/ruMorph/en/analysis/#guess).

</details>

<details>
<summary><b>Inflection</b></summary>

<br>

Every analysis knows its lexeme - all the forms of the word with the same lemma:

*   `forms` - the forms of the lexeme in dictionary order
*   `inflect` - the form with the given UD features; features that are not given are kept where possible
*   `agree` - the form of a noun, an adjective or a participle that agrees with a number: «1 рубль», «2 рубля», «5 рублей», «2 суток»

If the lexeme has no such form, `inflect` and `agree` raise `FormNotFoundError`, and on a misspelled feature `inflect` raises `ValueError` listing the valid values.

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> cat = morph.analyze("кошка")[0]
>>> [form.word for form in cat.forms()][:5]
['кошка', 'кошки', 'кошке', 'кошку', 'кошкой']
>>> cat.inflect(Case="Gen"), cat.inflect(Case="Gen", Number="Plur")
('кошки', 'кошек')
>>> verb = morph.analyze("читал")[0]
>>> verb.inflect(Gender="Fem"), verb.inflect(Tense="Pres", Person="3", Number="Plur")
('читала', 'читают')
>>> ruble = morph.analyze("рубль")[0]
>>> [f"{n} {ruble.agree(n)}" for n in (1, 2, 5, 21)]
['1 рубль', '2 рубля', '5 рублей', '21 рубль']
```

See the [documentation](https://sergeyshk.github.io/ruMorph/en/inflection/).

</details>

<details>
<summary><b>The rumorph command</b></summary>

<br>

The command is installed with the package and analyzes text from files or the standard input: lemmas, CoNLL-U, TSV or JSON Lines with all analyses. The input is text, one token per line or CoNLL-U with the LEMMA, UPOS and FEATS columns to fill.

```bash
$ echo "Мы стали друзьями в ковиде." | rumorph
мы стать друг в ковид .
$ rumorph --format conllu text.txt > text.conllu
```

See the [documentation](https://sergeyshk.github.io/ruMorph/en/cli/) for the options, formats and exit codes.

</details>

<details>
<summary><b>Migrating from pymorphy</b></summary>

<br>

ruMorph works with the same OpenCorpora dictionary as pymorphy2 and pymorphy3, but has its own API: UD parts of speech and features instead of OpenCorpora grammemes, a probability instead of `score`, word forms as strings, exceptions instead of `None`. Here `morph` is the analyzer, `p` is a pymorphy analysis and `a` is a ruMorph analysis:

| pymorphy2, pymorphy3 | ruMorph |
| :------------------- | :------ |
| `MorphAnalyzer()` | `Analyzer()` |
| `morph.parse(word)` | `morph.analyze(word)` |
| `[morph.parse(w)[0].normal_form for w in words]` | `morph.lemmatize(words)` |
| `morph.word_is_known(word)` | `morph.is_known(word)` |
| `p.normal_form`, `p.tag.POS`, `p.score` | `a.lemma`, `a.pos`, `a.prob` |
| `p.tag.case`, `p.tag.number` | `a.feats.get("Case")`, `a.feats.get("Number")` |
| `p.lexeme` | `a.forms()` |
| `p.inflect({"gent", "plur"}).word` | `a.inflect(Case="Gen", Number="Plur")` |
| `p.make_agree_with_number(n).word` | `a.agree(n)` |

See the [documentation](https://sergeyshk.github.io/ruMorph/en/migration/) for all the calls, parts of speech and grammemes.

</details>

<details>
<summary><b>Comparison</b></summary>

<br>

The share of words whose first analysis has the right part of speech and lemma, % (test parts of the UD 2.18 treebanks):

| System | SynTagRus | Taiga | GSD | Tokens per second |
| :----- | :-------- | :---- | :-- | :---------------- |
| ruMorph | **93.9** | **90.6** | **91.8** | **693,383** |
| pymorphy3 | 84.8 | 82.9 | 84.3 | 23,484 |
| mystem | 81.9 | 79.4 | 79.9 | 82,191 |

In context (`analyze_sentence`) ruMorph is more accurate than all the taggers in the part of speech, lemma and features together on Taiga and GSD (90.0 and 81.3 % of words against 85.6 and 80.3 % for Stanza), and on SynTagRus it is behind only Stanza, by 0.2 points, although Stanza is trained on SynTagRus. Meanwhile it handles 108 thousand tokens per second against 2,738 for Stanza. ruMorph answers a short text together with loading in 6 ms against 0.2-2 s for the taggers, processes share the dictionary, and every further one adds 1.2 MB. Measured on Apple M3 Pro, Python 3.11.

See the [documentation](https://sergeyshk.github.io/ruMorph/en/performance/) for context taggers, the chart of speed by text size, operations compared with pymorphy3, memory and how it was measured.

</details>

## Development

The project uses [uv](https://docs.astral.sh/uv/) to manage dependencies, [maturin](https://www.maturin.rs/) to build the Rust module and [ruff](https://docs.astral.sh/ruff/) for linting and formatting. [Rust](https://www.rust-lang.org/tools/install) 1.88 or newer is required.

```bash
git clone https://github.com/SergeyShk/ruMorph.git
cd ruMorph

make deps        # create the environment, install the dependencies and build the Rust module
make dictionary  # build the dictionary from pymorphy3-dicts-ru and UD treebanks
make test        # run the Python tests, the docstring examples (doctest) and the Rust tests
make lint        # ruff, mypy, rustfmt and clippy
```

The full list of commands is `make help`. The documentation is bilingual: Russian pages `docs/*.md` with English `docs/*.en.md` next to them; update both versions when editing a page.

Before submitting changes, install the hooks that run the linters on commit and the tests on push:

```bash
uv run pre-commit install
```

## Contributing

Bug reports, ideas and pull requests are welcome - the [issues](https://github.com/SergeyShk/ruMorph/issues) are open. The workflow and the checks to run before submitting a pull request are in [CONTRIBUTING.md](https://github.com/SergeyShk/ruMorph/blob/master/CONTRIBUTING.md); the rules of conduct are in the [code of conduct](https://github.com/SergeyShk/ruMorph/blob/master/CODE_OF_CONDUCT.md).

<details>
<summary><b>Project structure</b></summary>

<br>

*   **crates/rumorph** - the analyzer core in Rust: the dictionary format, building, analysis, the context model and inflection
*   **crates/rumorph-build** - building the dictionary from an export of pymorphy3-dicts-ru, converting the OpenCorpora tags to UD and frequencies of analyses from UD treebanks
*   **crates/rumorph-dict-ru** - the crate with the dictionary for `rumorph` on crates.io
*   **crates/rumorph-py** - the Python module `rumorph._rumorph` built with PyO3
*   **python/rumorph** - the Python package
*   **scripts** - exporting the pymorphy3-dicts-ru dictionary and the UD treebank counts, training the context model, checking the installed package before publishing and comparing with other analyzers
*   **data** - the context model weights that the builder embeds into the dictionary
*   **docs** - MkDocs documentation in Russian and English
*   **tests** - tests of the Python package

</details>

## Authors

*   Sergey Shkarin (kouki.sergey@gmail.com)

## License

The code is distributed under the [MIT](https://github.com/SergeyShk/ruMorph/blob/master/LICENSE.txt) license. The dictionary in the package is built from the [OpenCorpora](https://opencorpora.org) dictionary with frequencies and a context model from the [UD_Russian-Taiga](https://github.com/UniversalDependencies/UD_Russian-Taiga) and [UD_Russian-GSD](https://github.com/UniversalDependencies/UD_Russian-GSD) treebanks and is distributed under the CC BY-SA 4.0 license ([LICENSE-DICTIONARY.txt](https://github.com/SergeyShk/ruMorph/blob/master/LICENSE-DICTIONARY.txt)).

## Citation

If you use **ruMorph** in research or software, please cite it with the following BibTeX entry - citations help the development and support of the project. The same metadata is in [CITATION.cff](https://github.com/SergeyShk/ruMorph/blob/master/CITATION.cff), which GitHub shows under the "Cite this repository" button. The Zenodo concept DOI [10.5281/zenodo.23520738](https://doi.org/10.5281/zenodo.23520738) resolves to all versions of the library; the DOI of a specific version is on its release page.

```bibtex
@software{ruMorph,
  author = {Sergey Shkarin},
  title = {{ruMorph, a fast morphological analyzer for Russian}},
  year = 2026,
  doi = {10.5281/zenodo.23520738},
  url = {https://github.com/SergeyShk/ruMorph}
}
```
