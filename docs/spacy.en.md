# spaCy

The `rumorph` component plugs ruMorph into a [spaCy](https://spacy.io/) pipeline: it annotates the tokens of a document with the part of speech (`pos_`), the [Universal Dependencies](https://universaldependencies.org/) features (`morph`) and the lemma (`lemma_`) of the [analysis in context](analyzer.md#analyze_sentence). In the pipeline it replaces the tagger, the morphologizer and the lemmatizer.

``` bash
pip install rumorph[spacy]
```

The component registers with spaCy when the package is installed and is added by name, there is no need to import rumorph. spaCy has no prebuilt wheels for Python 3.14t without the GIL.

=== "Code"

    ``` python
    import spacy

    nlp = spacy.blank("ru")
    nlp.add_pipe("rumorph")
    doc = nlp("Мост сделан из стали.")
    [(token.lemma_, token.pos_) for token in doc]
    str(doc[3].morph)
    ```

=== "Result"

    ```
    [('мост', 'NOUN'), ('сделать', 'VERB'), ('из', 'ADP'), ('сталь', 'NOUN'), ('.', 'PUNCT')]
    'Animacy=Inan|Case=Gen|Gender=Fem|Number=Sing'
    ```

## ru_core_news models { #models }

In the `ru_core_news_*` models the component replaces the morphologizer and the lemmatizer, while the parser and named entity recognition stay. Placed after the parser, the component takes its sentence boundaries, and the model's `attribute_ruler` copies the part of speech to `tag_`.

=== "Code"

    ``` python
    nlp = spacy.load("ru_core_news_sm", exclude=["morphologizer", "lemmatizer"])
    nlp.add_pipe("rumorph", after="parser")
    nlp.pipe_names
    ```

=== "Result"

    ```
    ['tok2vec', 'parser', 'rumorph', 'attribute_ruler', 'ner']
    ```

The share of words with the right part of speech, lemma and UD features on the test parts of the treebanks ([Comparison](performance.md)) and the speed of the pipeline, tokens per second:

| Pipeline | SynTagRus | Taiga | GSD | Speed |
| :------- | :-------- | :---- | :-- | :---- |
| ru_core_news_sm | 82.0 | 75.5 | 74.8 | 5,291 |
| ru_core_news_sm with rumorph | **93.6** | **90.1** | **81.6** | **8,460** |

## Sentences { #sentences }

The context model analyzes a whole sentence. The component takes sentences from the annotation of the document - the parser, `senter` or `sentencizer`, and without it splits the document after sentence-final marks («.», «!», «?», «…») and at least every 256 tokens, like the [rumorph command](cli.md). Whitespace tokens get the `SPACE` part of speech, as in spaCy models, and the token itself as the lemma.

## Parameters { #config }

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `dictionary` | `str` | `None` | Path to the dictionary file; by default - the dictionary from the package |
| `words` | `dict` | `None` | [Own words](analyzer.md#words) and their patterns |
| `context` | `bool` | `True` | Choose the analysis by the neighbouring words; `False` - the most probable analysis of the word form, as with [`analyze`](analyzer.md#analyze) |

=== "Code"

    ``` python
    nlp = spacy.blank("ru")
    nlp.add_pipe("rumorph", config={"words": {"зумер": "геймер"}})
    [token.lemma_ for token in nlp("Зумерам нравится")]
    ```

=== "Result"

    ```
    ['зумер', 'нравиться']
    ```

## ruMorph analysis { #extension }

`token._.rumorph` is the ruMorph [analysis](analysis.md) with the lemma, part of speech and features of the token: it gives the [forms of the word](inflection.md). Its probability does not take the context into account. If another component changed the annotation of the token and the word form has no such analysis, the value is `None`.

=== "Code"

    ``` python
    doc = nlp("Мы стали друзьями")
    analysis = doc[2]._.rumorph
    analysis.lemma, analysis.inflect(Case="Dat")
    ```

=== "Result"

    ```
    ('друг', 'друзьям')
    ```
