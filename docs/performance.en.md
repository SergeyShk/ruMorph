# Comparison

ruMorph analyzes a word without context, like pymorphy3 and mystem, and it is the most accurate of them: on three UD test sets the first analysis gets both the part of speech and the lemma right for 90-94 % of words against 79-85 % for pymorphy3 and mystem. It is also the fastest: 684 thousand tokens per second, 29 times faster than pymorphy3 and 9 times faster than mystem, and it answers a short text together with loading in 5 ms. The context tagger Stanza is more accurate in features, but 230 times slower and answers a short text in 2 s.

## Quality { #quality }

The gold standard is the test parts of the [UD_Russian-SynTagRus](https://github.com/UniversalDependencies/UD_Russian-SynTagRus), [UD_Russian-Taiga](https://github.com/UniversalDependencies/UD_Russian-Taiga) and [UD_Russian-GSD](https://github.com/UniversalDependencies/UD_Russian-GSD) treebanks from the UD 2.18 release: every system gets their tokens, punctuation is not counted, and lemmas are compared ignoring case and with «е» instead of «ё». The treebanks follow different annotation conventions, so systems should be compared on the same set rather than sets against each other.

### Without context { #context-free }

The share of words whose first analysis has the right part of speech and lemma, in brackets - at least one of the analyses, %:

| System | SynTagRus | Taiga | GSD |
| :----- | :-------- | :---- | :-- |
| ruMorph | **93.7** (97.5) | **90.5** (94.9) | **92.1** (95.4) |
| pymorphy3 | 84.8 (90.6) | 82.9 (88.9) | 84.3 (87.9) |
| mystem | 81.9 (86.4) | 79.4 (83.8) | 79.9 (83.9) |

pymorphy3 and mystem annotate by their own rules, and their parts of speech are converted to UD by a simple table: a noun is `NOUN`, or `PROPN` with a grammeme of a name, a surname or a place, a pronoun-noun is `PRON`, a pronominal adjective is `DET` and so on. So some of their errors are differences of conventions rather than of analysis: «быть» is a verb for them but `AUX` in UD, and they do not split conjunctions into coordinating and subordinating (the conjunction «что» is `SCONJ` in UD). ruMorph follows the UD conventions. mystem does not analyze numbers written in digits.

### With context { #contextual }

Taggers see the sentence and choose one analysis; ruMorph in this table is the first analysis without context. The share of words with the right part of speech, lemma and UD features (case, number, gender, person, tense, aspect, mood, voice, verb form, degree, animacy), %:

=== "SynTagRus"

    | System | Part of speech | Lemma | Features |
    | :----- | :------------- | :---- | :------- |
    | ruMorph | 95.0 | 96.6 | 83.9 |
    | Stanza | **98.0** | **97.8** | **95.3** |
    | spaCy ru_core_news_sm | 94.1 | 90.7 | 81.0 |
    | Slovnet (Natasha) | 93.7 | - | 86.8 |

=== "Taiga"

    | System | Part of speech | Lemma | Features |
    | :----- | :------------- | :---- | :------- |
    | ruMorph | 92.1 | **95.5** | 81.6 |
    | Stanza | **94.5** | 94.9 | **90.3** |
    | spaCy ru_core_news_sm | 89.8 | 87.9 | 74.9 |
    | Slovnet (Natasha) | 88.2 | - | 81.1 |

=== "GSD"

    | System | Part of speech | Lemma | Features |
    | :----- | :------------- | :---- | :------- |
    | ruMorph | 93.8 | **96.3** | 72.8 |
    | Stanza | **96.0** | 95.9 | **83.7** |
    | spaCy ru_core_news_sm | 94.0 | 92.0 | 77.5 |
    | Slovnet (Natasha) | 93.5 | - | 79.8 |

The default Stanza model is trained on SynTagRus, so its test part is Stanza's own; spaCy ru_core_news_sm and Slovnet are trained on Nerus, news annotated automatically. Slovnet gives no lemmas: in Natasha they come from pymorphy2.

??? note "All metrics"

    The case and number of pymorphy3 and mystem are converted to UD by the same table; all features and "all together" (part of speech, lemma and features) only for systems with UD features.

    === "SynTagRus"

        | System | Lemma | Part of speech | Case | Number | Features | All together |
        | :----- | :---- | :------------- | :--- | :----- | :------- | :----------- |
        | ruMorph | 96.6 | 95.0 | 88.0 | 96.4 | 83.9 | 81.9 |
        | pymorphy3 | 94.1 | 87.0 | 87.0 | 94.8 | - | - |
        | mystem | 90.9 | 88.2 | 76.9 | 92.7 | - | - |
        | spaCy ru_core_news_sm | 90.7 | 94.1 | 95.6 | 97.3 | 81.0 | 76.5 |
        | Stanza | 97.8 | 98.0 | 98.4 | 99.2 | 95.3 | 93.5 |
        | Slovnet (Natasha) | - | 93.7 | 95.0 | 96.7 | 86.8 | - |

    === "Taiga"

        | System | Lemma | Part of speech | Case | Number | Features | All together |
        | :----- | :---- | :------------- | :--- | :----- | :------- | :----------- |
        | ruMorph | 95.5 | 92.1 | 86.8 | 95.4 | 81.6 | 77.6 |
        | pymorphy3 | 92.7 | 85.3 | 85.7 | 93.8 | - | - |
        | mystem | 90.5 | 85.2 | 77.0 | 91.7 | - | - |
        | spaCy ru_core_news_sm | 87.9 | 89.8 | 91.7 | 94.0 | 74.9 | 68.8 |
        | Stanza | 94.9 | 94.5 | 96.3 | 97.1 | 90.3 | 85.6 |
        | Slovnet (Natasha) | - | 88.2 | 90.6 | 93.0 | 81.1 | - |

    === "GSD"

        | System | Lemma | Part of speech | Case | Number | Features | All together |
        | :----- | :---- | :------------- | :--- | :----- | :------- | :----------- |
        | ruMorph | 96.3 | 93.8 | 80.8 | 93.2 | 72.8 | 71.2 |
        | pymorphy3 | 94.5 | 86.1 | 79.7 | 92.3 | - | - |
        | mystem | 91.2 | 85.5 | 66.3 | 88.1 | - | - |
        | spaCy ru_core_news_sm | 92.0 | 94.0 | 89.5 | 94.7 | 77.5 | 72.0 |
        | Stanza | 95.9 | 96.0 | 91.3 | 95.4 | 83.7 | 80.3 |
        | Slovnet (Natasha) | - | 93.5 | 89.0 | 94.3 | 79.8 | - |

## Speed { #speed }

### Text analysis { #text }

The time to analyze a text together with importing and loading the system in a new process, seconds, both axes on a logarithmic scale:

![Text analysis time with loading by text size](img/speed.en.svg)

On a short text almost all the time goes to loading: ruMorph answers in 5 ms, pymorphy3 and mystem in 20-40 ms, Slovnet in 0.2 s, spaCy in 1.2-1.4 s, Stanza in 2 s. On a long text the analysis speed decides, tokens per second without loading, more is better:

| System | Tokens per second |
| :----- | :---------------- |
| ruMorph | **684,151** |
| mystem | 79,046 |
| Slovnet (Natasha) | 44,323 |
| pymorphy3 | 23,843 |
| spaCy ru_core_news_sm | 8,878 |
| Stanza | 2,983 |

### Operations { #operations }

Thousands of words or analyses per second compared with pymorphy3, more is better:

| Operation | ruMorph | pymorphy3 | pymorphy3[fast] |
| :-------- | :------ | :-------- | :-------------- |
| Analysis of a word form: `analyze` in a loop, `parse` | 732 | 20 | 105 |
| Analysis of a list: `analyze_many` | 754 | - | - |
| Lemmatization: `lemmatize`, `parse(word)[0].normal_form` | 1,404 | 22 | 170 |
| Lemmatization in 4 threads: `lemmatize` | 4,275 | - | - |
| Form by features: `inflect(Case="Gen")`, `inflect({"gent"})` | 1,399 | 76 | 76 |
| Agreement with a number: `agree(5)`, `make_agree_with_number(5)` | 1,417 | 85 | 85 |
| Forms of the lexeme: `forms()`, `lexeme` | 101 | 32 | 32 |

Lists of words are processed without the GIL, so several threads sharing one analyzer run in parallel.

## Loading and memory { #memory }

| | ruMorph | pymorphy3 | pymorphy3[fast] |
| :- | :------ | :-------- | :-------------- |
| Import and dictionary loading | 4 ms | 30 ms | 31 ms |
| Growth of process memory | 16.0 MB | 34.5 MB | 25.5 MB |
| Every further process | 1.4 MB | 32.5 MB | 23.1 MB |

Analyzers with the same dictionary file in a process share the dictionary. The dictionary file is mapped into memory, so `multiprocessing` or joblib processes and web server workers share it too: every further process adds only the analyzer's own tables.

## How it was measured { #method }

- Quality and text analysis - the [`scripts/compare.py`](https://github.com/SergeyShk/ruMorph/blob/master/scripts/compare.py) script: `uv run --with rumorph==X.Y.Z scripts/compare.py build/compare --chart docs/img/speed.svg`, where `X.Y.Z` is the ruMorph version (without `--with`, the latest one from PyPI; a path to a wheel works instead of the version). It installs the systems into a separate environment with pinned versions, downloads the treebank test parts, the models and mystem with checksum verification before measuring and analyzes the gold tokens. The tables converting the parts of speech, case and number of pymorphy3 and mystem to UD are at the top of the script.
- Versions: Python 3.11, ruMorph 0.3.0.dev8 (a wheel built from the repository), pymorphy3 2.0.6 with the pymorphy3-dicts-ru 2.4.417150.4580142 dictionary, mystem 3.1 through pymystem3 0.2.0 without disambiguation (for quality, each word separately; the macOS binary is built for x86_64 and runs through Rosetta 2), spaCy 3.8.16 with the ru_core_news_sm 3.8.0 model, Stanza 1.15.0 with the default model on the CPU (torch 2.14.1), Slovnet 0.6.0 with the slovnet_morph_news_v1 and navec_news_v1 models. Computer - Apple M3 Pro, macOS 26.
- Text analysis speed - 157,706 tokens of the SynTagRus test part with punctuation in one thread: tokens per second - the best of two analyses of the whole part after loading, chart points - importing, loading and analyzing the first 10-100,000 tokens in a new process. Only the call of the system is measured: mystem gets a whole sentence, spaCy analyzes sentences with `nlp.pipe`.
- Operations and memory - ruMorph 0.2.0, pymorphy3 2.0.6, `pymorphy3[fast]` - the same with the C++ DAWG extension; 128,251 word forms of the SynTagRus test part without punctuation, the best of three runs in one thread, except for the row with threads. `inflect` - the genitive of the first analyses that have a case, `agree` - the first analyses of nouns with the number 5, `forms` - the first analyses of the first 20,000 words. Memory - the growth of the resident memory of the process (`ps -o rss`) after importing and creating the analyzer, time - the import, creating the analyzer and the first analysis; a further process - the growth of the memory the process does not share with others (USS), in each of four `multiprocessing` processes.
