# Analyzer

``` {.python .signature}
Analyzer(path=None)
```

Morphological analyzer: for a word form it finds all [analyses](analysis.md) in the OpenCorpora dictionary - the initial form, the part of speech and the [Universal Dependencies](https://universaldependencies.org/) features - and orders them by probability. The dictionary is loaded when the analyzer is created, so it is enough to create the analyzer once and use it across the program.

Exceptions:

- `FileNotFoundError` - the dictionary file does not exist;
- `OSError` - the dictionary file cannot be read: no access, the path points to a directory;
- `ValueError` - the file is not a ruMorph dictionary or is damaged;
- `TypeError` - a path that is neither a string nor `PathLike`.

=== "Code"

    ``` python
    from rumorph import Analyzer

    morph = Analyzer()
    morph.analyze("стали")[0]
    ```

=== "Result"

    ```
    Analysis(word='стали', lemma='стать', pos='VERB', feats='Aspect=Perf|Mood=Ind|Number=Plur|Tense=Past|VerbForm=Fin|Voice=Mid', labels='intr', prob=0.9753459013836056, source='dictionary')
    ```

## Parameters

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `path` | `str` / `PathLike` | `None` | Path to the dictionary file; by default, the dictionary from the package |

The object keeps the dictionary path in `path` of type `Path`, read-only.

## Methods

<div class="metric-card" markdown>

### Analyses of a word form { #analyze }

`analyze` · `list[Analysis]`
{ .metric-meta }

``` {.python .signature}
analyze(word)
```

Analyses of the word form in descending order of probability; the probabilities sum to 1. Case does not matter, and the letter «е» may stand for «ё»: `analyze("ЕЖ")` returns the analyses of the word form «ёж». A token that is not in the dictionary - a number, a symbol, a Latin word, an abbreviation - is analyzed [by rules](analysis.md#rules), and the analyses of an unknown Cyrillic word are [guessed](analysis.md#guess) from its ending and parts.

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `word` | `str` | - | Word form |

Exceptions:

- `TypeError` - a word that is not a string.

=== "Code"

    ``` python
    [(a.lemma, a.pos, a.feats.get("Case"), round(a.prob, 3)) for a in morph.analyze("стали")]
    [(a.lemma, a.source) for a in morph.analyze("чатботу")][:2]
    ```

=== "Result"

    ```
    [('стать', 'VERB', None, 0.975),
     ('сталь', 'NOUN', 'Gen', 0.011),
     ('сталь', 'NOUN', 'Nom', 0.005),
     ('сталь', 'NOUN', 'Dat', 0.003),
     ('сталь', 'NOUN', 'Loc', 0.003),
     ('сталь', 'NOUN', 'Acc', 0.003)]
    [('чатбот', 'guess'), ('чатбота', 'guess')]
    ```

</div>

<div class="metric-card" markdown>

### Analyses of a list of word forms { #analyze_many }

`analyze_many` · `list[list[Analysis]]`
{ .metric-meta }

``` {.python .signature}
analyze_many(words)
```

Analyses of each word form, as with [`analyze`](#analyze); on a list the method is faster than `analyze` in a loop.

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `words` | `Iterable[str]` | - | Word forms: a list, a tuple, a generator |

Exceptions:

- `TypeError` - a string instead of a collection of words, a word that is not a string.

=== "Code"

    ``` python
    [len(analyses) for analyses in morph.analyze_many(["мама", "мыла", "раму"])]
    ```

=== "Result"

    ```
    [1, 4, 2]
    ```

</div>

<div class="metric-card" markdown>

### Lemmatization { #lemmatize }

`lemmatize` · `list[str]`
{ .metric-meta }

``` {.python .signature}
lemmatize(words)
```

Lemmas of the most probable analyses of the word forms; a token without analyses is returned in lowercase.

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `words` | `Iterable[str]` | - | Word forms: a list, a tuple, a generator |

Exceptions:

- `TypeError` - a string instead of a collection of words, a word that is not a string.

=== "Code"

    ``` python
    morph.lemmatize("Мы стали друзьями в ковиде".split())
    ```

=== "Result"

    ```
    ['мы', 'стать', 'друг', 'в', 'ковид']
    ```

</div>

<div class="metric-card" markdown>

### Word form in the dictionary { #is_known }

`is_known` · `bool`
{ .metric-meta }

``` {.python .signature}
is_known(word)
```

Whether the word form is in the dictionary; as with `analyze`, case and «е» instead of «ё» do not matter. Words analyzed by rules or guessed are not in the dictionary.

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `word` | `str` | - | Word form |

Exceptions:

- `TypeError` - a word that is not a string.

=== "Code"

    ``` python
    morph.is_known("Ежи"), morph.is_known("вайб")
    ```

=== "Result"

    ```
    (True, False)
    ```

</div>

## Dictionary { #dictionary }

The package dictionary is built from the [OpenCorpora](https://opencorpora.org) dictionary: about 3.1 million word forms. The probabilities of analyses come from the OpenCorpora corpus: a word written with «е» instead of «ё» gets the frequencies of the spelling with «ё», and word forms missing from the corpus get equally probable analyses. The dictionary is distributed under the [CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/) license.

## Threads and processes { #parallel }

One analyzer can be used from several threads: `analyze_many` and `lemmatize` release the GIL and process lists in parallel. The analyzer and analyses are pickled, so they work with `multiprocessing` and joblib: a process loads the dictionary once, however many analyses it receives.

=== "Code"

    ``` python
    from concurrent.futures import ThreadPoolExecutor

    texts = [["мама", "мыла", "раму"], ["кошки", "ловят", "мышей"]]
    with ThreadPoolExecutor(2) as pool:
        lemmas = list(pool.map(morph.lemmatize, texts))
    lemmas
    ```

=== "Result"

    ```
    [['мама', 'мыло', 'рама'], ['кошка', 'ловить', 'мышь']]
    ```
