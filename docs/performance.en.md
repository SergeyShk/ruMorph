# Speed and memory

Analysis and lemmatization in ruMorph are about 5 times faster than in pymorphy3 with the `pymorphy3[fast]` speedup and 24-37 times faster than in pymorphy3 without it; `inflect` and `agree` are 15-18 times faster. Lists of words are processed without the GIL, so several threads sharing one analyzer run in parallel. The dictionary loads in 14 ms, twice as fast as in pymorphy3, and takes less memory than with `pymorphy3[fast]`.

## Speed { #speed }

Thousands of words or analyses per second, more is better:

| Operation | ruMorph | pymorphy3 | pymorphy3[fast] |
| :-------- | :------ | :-------- | :-------------- |
| Analysis of a word form: `analyze` in a loop, `parse` | 474 | 20 | 105 |
| Analysis of a list: `analyze_many` | 493 | - | - |
| Lemmatization: `lemmatize`, `parse(word)[0].normal_form` | 810 | 22 | 170 |
| Lemmatization in 4 threads: `lemmatize` | 2,345 | - | - |
| Form by features: `inflect(Case="Gen")`, `inflect({"gent"})` | 1,400 | 76 | 76 |
| Agreement with a number: `agree(5)`, `make_agree_with_number(5)` | 1,307 | 85 | 85 |
| Forms of the lexeme: `forms()`, `lexeme` | 57 | 32 | 32 |

## Loading and memory { #memory }

| | ruMorph | pymorphy3 | pymorphy3[fast] |
| :- | :------ | :-------- | :-------------- |
| Import and dictionary loading | 14 ms | 30 ms | 31 ms |
| Growth of process memory | 21.5 MB | 34.5 MB | 25.5 MB |

Analyzers with the same dictionary file in a process share the dictionary, while `multiprocessing` and joblib processes each load it into their own memory.

## How it was measured { #method }

- Computer - Apple M3 Pro, macOS 26, Python 3.11; ruMorph 0.1.0.dev14, pymorphy3 2.0.6 with the pymorphy3-dicts-ru 2.4.417150.4580142 dictionary; `pymorphy3[fast]` - the same with the C++ DAWG extension.
- Words - 128,251 word forms of the test part of [UD_Russian-SynTagRus](https://github.com/UniversalDependencies/UD_Russian-SynTagRus) without punctuation, in text order. Time - the best of three runs in one thread, except for the row with threads.
- `inflect` - the genitive of the first analyses that have a case, `agree` - the first analyses of nouns with the number 5, `forms` - the first analyses of the first 20,000 words.
- Memory - the growth of the resident memory of the process (`ps -o rss`) after importing the library and creating the analyzer; time - the import, creating the analyzer and the first analysis.
