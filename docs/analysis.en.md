# Word analysis

The analyzer finds every analysis of a word form in the OpenCorpora dictionary - the normal form, the part of speech and the [Universal Dependencies](https://universaldependencies.org/) features - and orders them by probability.

## Analyzer { #analyzer }

``` python
>>> from rumorph import Analyzer
>>> morph = Analyzer()
```

The dictionary loads in a few milliseconds when the analyzer is created. One analyzer is enough for the whole program, including several threads. By default the dictionary from the package is used; another dictionary file is passed as the `path` argument. The analyzer and the analyses can be pickled, so they work with multiprocessing and joblib: another process loads the dictionary again.

## Analyses of a word form { #analyze }

`analyze` returns the analyses of a word form in order of decreasing probability:

``` python
>>> for analysis in morph.analyze("стали")[:2]:
...     print(analysis.lemma, analysis.pos, dict(analysis.feats), round(analysis.prob, 3))
стать VERB {'Aspect': 'Perf', 'Mood': 'Ind', 'Number': 'Plur', 'Tense': 'Past', 'VerbForm': 'Fin', 'Voice': 'Mid'} 0.975
сталь NOUN {'Animacy': 'Inan', 'Case': 'Gen', 'Gender': 'Fem', 'Number': 'Sing'} 0.011
```

| Field | Meaning |
|---|---|
| `word` | the word form as spelled in the dictionary: with the letter «ё» if the dictionary has it |
| `lemma` | the normal form |
| `pos` | the UD part of speech: `NOUN`, `VERB`, `ADJ`, `DET`, `ADP` and so on |
| `feats` | the UD features as a dict `{"Case": "Gen", "Number": "Sing"}`; a new one each time, changing it does not change the analysis |
| `labels` | dictionary labels without a UD feature, as a `frozenset`: `{"Infr"}` (informal) |
| `prob` | the probability of the analysis; it sums to 1 over all analyses of the word form |
| `source` | where the analysis comes from: `"dictionary"` - the word form is in the dictionary |

Case does not matter, and the letter «е» may stand for «ё»: `analyze("ЕЖ")` returns the analyses of the word form «ёж». For a word that is not in the dictionary the list is empty.

## Parts of speech and features { #ud }

Parts of speech and features follow [Universal Dependencies](https://universaldependencies.org/u/pos/) as annotated in the largest Russian treebank, [UD_Russian-SynTagRus](https://github.com/UniversalDependencies/UD_Russian-SynTagRus), so the output can be compared with spaCy, Stanza and other UD tools. Differences from the OpenCorpora parts of speech:

- «который», «этот», «свой», «весь», «такой» and the other pronominal adjectives are `DET`;
- «быть» and «бы» are `AUX`;
- the parenthetical words «однако», «например», «конечно» are `ADV`;
- «можно», «надо», «нельзя» are `VERB` without features;
- prepositions are `ADP`, coordinating conjunctions `CCONJ`, subordinating ones `SCONJ`, particles `PART`.

Features: case, number, gender, animacy, person, tense, aspect, mood, voice, verb form, degree, the short form (`Variant=Short`), the type of a proper name (`NameType`), abbreviation (`Abbr`), possessiveness (`Poss`), the type of a numeral (`NumType`) and how it is written (`NumForm`), the type of a pronoun or pronominal adverb (`PronType`: «там» - `Dem`, «где» - `Rel`, «никогда» - `Neg`), reflexivity (`Reflex`), indeclinability (`InflClass=Ind`), negation (`Polarity=Neg`).

Lemmas follow SynTagRus too: feminine surnames and patronymics keep the feminine lemma («Ивановой» - «иванова»), superlatives stay superlative («важнейшей» - «важнейший»), «денег» - «деньги». When several OpenCorpora analyses coincide in UD (the conjunction and the adverb «так»), they are merged into one with the sum of their probabilities.

The `labels` are the [OpenCorpora](https://opencorpora.org/dict.php?act=gram) grammemes that have no UD feature: style (`Infr`, `Slng`, `Arch`), qualitative adjective (`Qual`), parenthetical word (`Prnt`), transitivity (`tran`, `intr`), whether the speaker is included in an imperative (`incl` - «пойдемте», `excl` - «иди») and others.

## Lists of words { #lists }

`analyze_many` analyzes a collection of word forms, and `lemmatize` returns the lemmas of their most probable analyses:

``` python
>>> morph.lemmatize(["Мы", "стали", "друзьями"])
['мы', 'стать', 'друг']
```

`lemmatize` returns a word that is not in the dictionary in lower case. Both methods accept any collection of words - a list, a tuple, a generator - and analyze it without the GIL: several threads sharing one analyzer run in parallel.

## Dictionary { #dictionary }

`is_known` checks whether a word form is in the dictionary. The dictionary is OpenCorpora, about 3.1 million word forms; the probabilities of the analyses come from the OpenCorpora corpus: a word written with «е» instead of «ё» gets the frequencies of the spelling with «ё», and the analyses of word forms missing from the corpus are equally probable. The dictionary is distributed under the [CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/) license.
