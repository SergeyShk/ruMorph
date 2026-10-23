# Analyzer

``` {.python .signature}
Analyzer(path=None, words=None)
```

Morphological analyzer: for a word form it finds all [analyses](analysis.md) in the OpenCorpora dictionary - the initial form, the part of speech and the [Universal Dependencies](https://universaldependencies.org/) features - and orders them by probability. The dictionary is loaded when the first analyzer is created: analyzers with the same dictionary file in a process share it, so a new analyzer is created instantly. The last loaded dictionary stays in the process even without analyzers, so process pool tasks and an analyzer created for a single call do not load the dictionary again.

Exceptions:

- `FileNotFoundError` - the dictionary file does not exist;
- `OSError` - the dictionary file cannot be read: no access, the path points to a directory;
- `ValueError` - the file is not a ruMorph dictionary or is damaged, or an [own word](#words) has no sample in the dictionary, its sample is ambiguous, the word does not inflect like it or contains a space, a control or an invisible character;
- `TypeError` - a path that is neither a string nor `PathLike`, or own words that are not a mapping of strings.

=== "Code"

    ``` python
    from rumorph import Analyzer

    morph = Analyzer()
    morph.analyze("стали")[0]
    ```

=== "Result"

    ```
    Analysis(word='стали', lemma='стать', pos='VERB', feats='Aspect=Perf|Mood=Ind|Number=Plur|Tense=Past|VerbForm=Fin|Voice=Act', labels='intr', prob=0.9877829877829878, source='dictionary')
    ```

## Parameters

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `path` | `str` / `PathLike` | `None` | Path to the dictionary file; by default, the dictionary from the package |
| `words` | `Mapping[str, str \| tuple[str, str]]` | `None` | [Own words](#words) and their samples |

The object keeps the dictionary path in `path` of type `Path`, read-only. The dictionary file is mapped into memory: processes with the same file share its memory, so every further process with an analyzer takes about 1.2 MB. While an analyzer exists, the file must not be overwritten in place, but it can be replaced with a new file; on Windows an open file can be neither replaced nor deleted. The last loaded dictionary stays open even after the analyzers are deleted - until a dictionary from another file is loaded or the process exits: to release the file of your own dictionary, for example before deleting it on Windows, delete its analyzers and analyses and create an analyzer with the package dictionary: `Analyzer()`. A dictionary is trusted data, like code: loading rejects a damaged file but not every deliberately crafted one, so dictionaries from untrusted sources should not be loaded.

## Own words { #words }

A word that is not in the dictionary can be added after a sample - a dictionary word that inflects the same way: «зумер» like «геймер», «кринж» like «пляж». The word and the sample are written in the same form, usually the initial one; stress marks, soft hyphens and look-alike Latin letters in them are normalized as in a query. The own word gets all forms of the sample with its own stem and then behaves like a dictionary word: `source` is `"dictionary"`, `is_known` finds it, `forms`, `inflect` and `agree` work. If the sample has lexemes of different parts of speech («печь» is a noun and a verb), the sample is a pair with the UD part of speech: `{"зечь": ("печь", "VERB")}`; otherwise the analyzer is not created. An own word listed earlier can be a sample too: `{"зумер": "геймер", "бумер": "зумер"}`. Own words belong to this analyzer only, and pickle saves them with it.

Own words are kept in a copy of the dictionary, about 3.5 KB per word (35 MB for 10 thousand words). Processes share the dictionary mapped into memory, but each process builds the copy itself. The last built copy stays in memory after the analyzers are deleted until another one is built or another dictionary is loaded: tasks of a process pool do not build it again.

=== "Code"

    ``` python
    morph = Analyzer(words={"зумер": "геймер", "кринж": "пляж"})
    zoomer = morph.analyze("зумерами")[0]
    zoomer.lemma, zoomer.inflect(Case="Dat", Number="Sing"), morph.lemmatize(["кринжем"])
    ```

=== "Result"

    ```
    ('зумер', 'зумеру', ['кринж'])
    ```

## Methods

<div class="metric-card" markdown>

### Analyses of a word form { #analyze }

`analyze` · `list[Analysis]`
{ .metric-meta }

``` {.python .signature}
analyze(word)
```

Analyses of the word form in descending order of probability; the probabilities sum to 1. Case does not matter for the lookup, and the letter «е» may stand for «ё»: `analyze("ЕЖ")` returns the analyses of the word form «ёж». The probabilities take the case into account: a lowercase word gets proper nouns last («шаров» - «шар»), and a capitalized word gets them a larger share («Шаров» - a surname). A token that is not in the dictionary - a number, a symbol, a Latin word, an abbreviation - is analyzed [by rules](analysis.md#rules), and the analyses of an unknown Cyrillic word are [guessed](analysis.md#guess) from its ending and parts. A token without spaces and invisible characters gets at least one analysis: if nothing fits, `X` without features.

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
    [('стать', 'VERB', None, 0.988),
     ('сталь', 'NOUN', 'Gen', 0.005),
     ('сталь', 'NOUN', 'Nom', 0.003),
     ('сталь', 'NOUN', 'Dat', 0.001),
     ('сталь', 'NOUN', 'Loc', 0.001),
     ('сталь', 'NOUN', 'Acc', 0.001)]
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

### Sentence analysis { #analyze_sentence }

`analyze_sentence` · `list[list[Analysis]]`
{ .metric-meta }

``` {.python .signature}
analyze_sentence(words)
```

Analyses of the words of a sentence in context: the same analyses as with [`analyze`](#analyze) are ordered by the context model - the first one is the analysis it chose from the neighbouring words, and `prob` is its estimate in this sentence. Without context «книги» is in the genitive (it is more frequent), in «Я читаю книги» the model picks the accusative, and «стали» in «Мост сделан из стали» is the noun «сталь», not the verb «стать». With context the part of speech, lemma and features of the first analysis are right for 81-93 % of words against 71-82 % without it ([Comparison](performance.md)). The words are the tokens of one sentence in order: ruMorph does not split text into sentences and tokens. A word without neighbours is analyzed as with `analyze`: without context the model chooses worse than the dictionary frequencies. The model is trained on the [UD_Russian-Taiga](https://github.com/UniversalDependencies/UD_Russian-Taiga) and [UD_Russian-GSD](https://github.com/UniversalDependencies/UD_Russian-GSD) treebanks and the [OpenCorpora](https://opencorpora.org) corpus and ships with the package dictionary.

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `words` | `Iterable[str]` | - | Tokens of a sentence: a list, a tuple, a generator |

Exceptions:

- `TypeError` - a string instead of a collection of words, a word that is not a string;
- `ValueError` - the dictionary has no context model (a dictionary built without it).

=== "Code"

    ``` python
    words = ["Я", "читаю", "книги"]
    print([analyses[0].feats.get("Case") for analyses in morph.analyze_many(words)])
    print([analyses[0].feats.get("Case") for analyses in morph.analyze_sentence(words)])
    print(morph.analyze_sentence(["Мост", "сделан", "из", "стали"])[3][0].lemma)
    ```

=== "Result"

    ```
    ['Nom', None, 'Gen']
    ['Nom', None, 'Acc']
    сталь
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

The package dictionary is built from the [OpenCorpora](https://opencorpora.org) dictionary: about 3.1 million word forms. The probabilities of analyses come from the OpenCorpora corpus and are refined by the annotation of the Universal Dependencies treebanks [UD_Russian-Taiga](https://github.com/UniversalDependencies/UD_Russian-Taiga) and [UD_Russian-GSD](https://github.com/UniversalDependencies/UD_Russian-GSD): this way «ничего» is more often a pronoun than an adverb. Word forms missing from every corpus get equally probable analyses. The dictionary is distributed under the [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) license.

## Threads and processes { #parallel }

One analyzer can be used from several threads: `analyze_many` and `lemmatize` release the GIL and process lists in parallel, and on the free-threaded Python 3.14t `analyze`, `is_known`, `inflect` and `agree` run in parallel too. A shared analyzer is not slower than a separate one per thread, so there is no need for an analyzer per thread. Without the GIL threads are limited by deleting the analysis objects: `forms` hardly speeds up in threads and `analyze_many` on long lists speeds up weakly, so such lists are better analyzed in chunks of a thousand or two words. The analyzer and analyses are pickled, so they work with `multiprocessing` and joblib: processes share the dictionary mapped into memory, however many analyses they receive (each process builds the copy with [own words](#words) itself). A pickle keeps the dictionary path: unpickling takes the analyzer with the same path whose analyses are in the thread, otherwise loads the dictionary from the path or, if the file is gone, takes the dictionary loaded from it in the process. An analysis pickled with another version of the dictionary is found by its spelling, lemma and tag; if the dictionary has no such analysis, it is restored as saved, but without other forms, and if the dictionary has no such tag either, the analysis of the word with new features added to the saved ones is taken (the gender of «Нидерландов»). Python 3.14 subinterpreters (`InterpreterPoolExecutor`) are not supported: use threads or processes. Passing analyses between processes costs more than the analysis itself, so in processes it is better to get lemmas and analysis fields, and analyses in threads.

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
    [['мама', 'мыть', 'рама'], ['кошка', 'ловить', 'мышь']]
    ```
