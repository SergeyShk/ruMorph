# Speed and memory

Analysis and lemmatization in ruMorph are 6-8 times faster than in pymorphy3 with the `pymorphy3[fast]` speedup and 31-63 times faster than in pymorphy3 without it; `inflect` and `agree` are 15-18 times faster. Lists of words are processed without the GIL, so several threads sharing one analyzer run in parallel. The dictionary loads in 5 ms, 6 times faster than in pymorphy3, and takes less memory than with `pymorphy3[fast]`; processes share the dictionary, so every further process with an analyzer takes about 1.5 MB.

## Speed { #speed }

Thousands of words or analyses per second, more is better:

| Operation | ruMorph | pymorphy3 | pymorphy3[fast] |
| :-------- | :------ | :-------- | :-------------- |
| Analysis of a word form: `analyze` in a loop, `parse` | 629 | 20 | 105 |
| Analysis of a list: `analyze_many` | 641 | - | - |
| Lemmatization: `lemmatize`, `parse(word)[0].normal_form` | 1,391 | 22 | 170 |
| Lemmatization in 4 threads: `lemmatize` | 4,210 | - | - |
| Form by features: `inflect(Case="Gen")`, `inflect({"gent"})` | 1,338 | 76 | 76 |
| Agreement with a number: `agree(5)`, `make_agree_with_number(5)` | 1,284 | 85 | 85 |
| Forms of the lexeme: `forms()`, `lexeme` | 62 | 32 | 32 |

## Loading and memory { #memory }

| | ruMorph | pymorphy3 | pymorphy3[fast] |
| :- | :------ | :-------- | :-------------- |
| Import and dictionary loading | 5 ms | 30 ms | 31 ms |
| Growth of process memory | 16.2 MB | 34.5 MB | 25.5 MB |
| Every further process | 1.4 MB | 34.5 MB | 25.5 MB |

Analyzers with the same dictionary file in a process share the dictionary. The dictionary file is mapped into memory, so `multiprocessing` or joblib processes and web server workers share it too: every further process adds only the analyzer's own tables.

## How it was measured { #method }

- Computer - Apple M3 Pro, macOS 26, Python 3.11; ruMorph 0.2.0.dev1, pymorphy3 2.0.6 with the pymorphy3-dicts-ru 2.4.417150.4580142 dictionary; `pymorphy3[fast]` - the same with the C++ DAWG extension.
- Words - 128,251 word forms of the test part of [UD_Russian-SynTagRus](https://github.com/UniversalDependencies/UD_Russian-SynTagRus) without punctuation, in text order. Time - the best of three runs in one thread, except for the row with threads.
- `inflect` - the genitive of the first analyses that have a case, `agree` - the first analyses of nouns with the number 5, `forms` - the first analyses of the first 20,000 words.
- Memory - the growth of the resident memory of the process (`ps -o rss`) after importing the library and creating the analyzer; time - the import, creating the analyzer and the first analysis. A further process - the growth of the memory the process does not share with others (USS), in each of four `multiprocessing` processes with an analyzer.
