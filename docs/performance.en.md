# Speed and memory

Analysis and lemmatization in ruMorph are about 5 times faster than in pymorphy3 with the `pymorphy3[fast]` speedup and 25-40 times faster than in pymorphy3 without it; `inflect` and `agree` are 15-18 times faster. Lists of words are processed without the GIL, so several threads sharing one analyzer run in parallel. The dictionary loads in 24 ms and takes as much memory as with `pymorphy3[fast]`.

## Speed { #speed }

Thousands of words or analyses per second, more is better:

| Operation | ruMorph | pymorphy3 | pymorphy3[fast] |
| :-------- | :------ | :-------- | :-------------- |
| Analysis of a word form: `analyze` in a loop, `parse` | 483 | 20 | 106 |
| Analysis of a list: `analyze_many` | 505 | - | - |
| Lemmatization: `lemmatize`, `parse(word)[0].normal_form` | 889 | 21 | 172 |
| Lemmatization in 4 threads: `lemmatize` | 2,229 | - | - |
| Form by features: `inflect(Case="Gen")`, `inflect({"gent"})` | 1,354 | 76 | 76 |
| Agreement with a number: `agree(5)`, `make_agree_with_number(5)` | 1,270 | 85 | 86 |
| Forms of the lexeme: `forms()`, `lexeme` | 56 | 32 | 32 |

`lemmatize` is faster than `analyze` because it does not create analysis objects, and in 4 threads lemmatization is 2.5 times faster than in one.

## Loading and memory { #memory }

| | ruMorph | pymorphy3 | pymorphy3[fast] |
| :- | :------ | :-------- | :-------------- |
| Import and dictionary loading | 24 ms | 31 ms | 31 ms |
| Growth of process memory | 26 MB | 35 MB | 26 MB |

It is enough to create the analyzer once: every new analyzer loads the dictionary again. `multiprocessing` and joblib processes each load the dictionary into their own memory.

## How it was measured { #method }

- Computer - Apple M3 Pro, macOS 26, Python 3.11; ruMorph 0.1.0.dev9, pymorphy3 2.0.6 with the pymorphy3-dicts-ru 2.4.417150.4580142 dictionary; `pymorphy3[fast]` - the same with the C++ DAWG extension.
- Words - 128,251 word forms of the test part of [UD_Russian-SynTagRus](https://github.com/UniversalDependencies/UD_Russian-SynTagRus) without punctuation, in text order. Time - the best of three runs in one thread, except for the row with threads.
- `inflect` - the genitive of the first analyses that have a case, `agree` - the first analyses of nouns with the number 5, `forms` - the first analyses of the first 20,000 words.
- Memory - the growth of the resident memory of the process (`ps -o rss`) after importing the library and creating the analyzer; time - the import, creating the analyzer and the first analysis.
