# Word analysis

The analyzer finds every analysis of a word form in the OpenCorpora dictionary and orders them by probability.

## Analyzer { #analyzer }

``` python
>>> from rumorph import Analyzer
>>> morph = Analyzer()
```

The dictionary loads in a few milliseconds when the analyzer is created. One analyzer is enough for the whole program, including several threads. By default the dictionary from the package is used; another dictionary file is passed as the `path` argument.

## Analyses of a word form { #analyze }

`analyze` returns the analyses of a word form in order of decreasing probability:

``` python
>>> for analysis in morph.analyze("стали")[:2]:
...     print(analysis.word, analysis.lemma, round(analysis.prob, 3))
стали стать 0.975
стали сталь 0.011
```

| Field | Meaning |
|---|---|
| `word` | the word form as spelled in the dictionary: with the letter «ё» if the dictionary has it |
| `lemma` | the normal form |
| `prob` | the probability of the analysis; it sums to 1 over all analyses of the word form |
| `source` | where the analysis comes from: `"dictionary"` - the word form is in the dictionary |

Case does not matter, and the letter «е» may stand for «ё»: `analyze("ЕЖ")` returns the analyses of the word form «ёж». For a word that is not in the dictionary the list is empty.

## Lists of words { #lists }

`analyze_many` analyzes a collection of word forms, and `lemmatize` returns the lemmas of their most probable analyses:

``` python
>>> morph.lemmatize(["Мы", "стали", "друзьями"])
['мы', 'стать', 'друг']
```

`lemmatize` returns a word that is not in the dictionary in lower case. Both methods accept any collection of words - a list, a tuple, a generator - and analyze it without the GIL: several threads sharing one analyzer run in parallel.

## Dictionary { #dictionary }

`is_known` checks whether a word form is in the dictionary. The dictionary is OpenCorpora, about 3.1 million word forms; the probabilities of the analyses come from the OpenCorpora corpus, and the analyses of word forms missing from the corpus are equally probable. The dictionary is distributed under the [CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/) license.
