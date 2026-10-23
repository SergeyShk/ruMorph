# Migrating from pymorphy

pymorphy2 and pymorphy3 share one API: pymorphy3 continues pymorphy2, which does not work on Python 3.11 and newer. ruMorph uses the same OpenCorpora dictionary but has its own API: [Universal Dependencies](https://universaldependencies.org/) parts of speech and features instead of OpenCorpora grammemes, a probability instead of `score`, word forms as strings, exceptions instead of `None`.

``` bash
pip install rumorph
```

The dictionary is included in the package, no separate dictionary package is needed.

=== "pymorphy3"

    ``` python
    >>> import pymorphy3
    >>> morph = pymorphy3.MorphAnalyzer()
    >>> p = morph.parse("стали")[0]
    >>> p.normal_form, p.tag.POS, p.tag.number, round(p.score, 3)
    ('стать', 'VERB', 'plur', 0.975)
    >>> cat = morph.parse("кошка")[0]
    >>> cat.inflect({"gent", "plur"}).word
    'кошек'
    >>> morph.parse("рубль")[0].make_agree_with_number(5).word
    'рублей'
    ```

=== "ruMorph"

    ``` python
    >>> import rumorph
    >>> morph = rumorph.Analyzer()
    >>> a = morph.analyze("стали")[0]
    >>> a.lemma, a.pos, a.feats["Number"], round(a.prob, 3)
    ('стать', 'VERB', 'Plur', 0.988)
    >>> cat = morph.analyze("кошка")[0]
    >>> cat.inflect(Case="Gen", Number="Plur")
    'кошек'
    >>> morph.analyze("рубль")[0].agree(5)
    'рублей'
    ```

## Call correspondence { #calls }

Here `morph` is the analyzer, `p` is a pymorphy analysis, `a` is a ruMorph analysis.

| pymorphy2, pymorphy3 | ruMorph | Differences |
| :------------------- | :------ | :---------- |
| `MorphAnalyzer()` | [`Analyzer()`](analyzer.md) | |
| `MorphAnalyzer(path)` | `Analyzer(path)` | the path to a ruMorph dictionary file; pymorphy dictionaries do not fit |
| `morph.parse(word)` | [`morph.analyze(word)`](analyzer.md#analyze) | analyses that coincide in UD are merged |
| `[morph.parse(w) for w in words]` | [`morph.analyze_many(words)`](analyzer.md#analyze_many) | runs without the GIL |
| `[morph.parse(w)[0].normal_form for w in words]` | [`morph.lemmatize(words)`](analyzer.md#lemmatize) | runs without the GIL |
| `morph.normal_forms(word)` | `list(dict.fromkeys(a.lemma for a in morph.analyze(word)))` | |
| `morph.tag(word)` | `[(a.pos, a.feats) for a in morph.analyze(word)]` | |
| `morph.word_is_known(word)` | [`morph.is_known(word)`](analyzer.md#is_known) | |
| `p.word` | `a.word` | analyses by rules and guessed ones keep the case of the text |
| `p.normal_form` | `a.lemma` | lemmas follow the UD conventions |
| `p.tag.POS` | `a.pos` | UD parts of speech, see [below](#pos) |
| `p.tag.case`, `p.tag.number` and others | `a.feats.get("Case")`, `a.feats.get("Number")` and others | UD features, see [below](#grammemes) |
| `"Infr" in p.tag` | `"Infr" in a.labels` | `labels` holds the grammemes that have no UD feature |
| `p.score` | `a.prob` | |
| `p.methods_stack` | `a.source` | `"dictionary"`, `"rule"` or `"guess"` |
| `p.lexeme` | [`a.forms()`](inflection.md#forms) | |
| `p.inflect({"gent", "plur"}).word` | [`a.inflect(Case="Gen", Number="Plur")`](inflection.md#inflect) | a string; no form - `FormNotFoundError` instead of `None` |
| `p.make_agree_with_number(n).word` | [`a.agree(n)`](inflection.md#agree) | a string; no form - `FormNotFoundError` instead of `None` |

There is no counterpart for the Russian names of grammemes (`cyr_repr`, `lat2cyr`), the analysis of the initial form (`p.normalized`), iterating over the dictionary and configuring the predictor of unknown words.

## Parts of speech { #pos }

| OpenCorpora | UD |
| :---------- | :- |
| `NOUN` | `NOUN`; given names, surnames, patronymics, places, organizations and trademarks - `PROPN` |
| `ADJF` | `ADJ`; pronominal adjectives - `DET` |
| `ADJS` | `ADJ` with `Variant=Short` |
| `COMP` | `ADJ` with `Degree=Cmp` |
| `VERB` | `VERB` with `VerbForm=Fin`; «быть» - `AUX` |
| `INFN` | `VERB` with `VerbForm=Inf` |
| `PRTF` | `VERB` with `VerbForm=Part` |
| `PRTS` | `VERB` with `VerbForm=Part` and `Variant=Short` |
| `GRND` | `VERB` with `VerbForm=Conv` |
| `NUMR` | `NUM` with `NumType=Card` |
| `ADVB` | `ADV` |
| `NPRO` | `PRON` |
| `PRED` | `VERB` without features |
| `PREP` | `ADP` |
| `CONJ` | `CCONJ`; subordinating conjunctions - `SCONJ` |
| `PRCL` | `PART` |
| `INTJ` | `INTJ` |

Some words get their part of speech by the SynTagRus conventions: the parenthetical «однако», «например» are `ADV`, «бы» is `AUX` and so on (see [Parts of speech and features](analysis.md#ud)). Tokens outside the dictionary: `NUMB` - `NUM` with `NumForm=Digit`, `ROMN` - `ADJ` with `NumForm=Roman`, `LATN` - `X` with `Foreign=Yes`, `PNCT` - `PUNCT` or `SYM`, `UNKN` - `X` without features (see [Tokens outside the dictionary](analysis.md#rules)).

## Grammemes { #grammemes }

| OpenCorpora | UD |
| :---------- | :- |
| `nomn`, `gent`, `datv`, `accs`, `ablt`, `loct`, `voct` | `Case`: `Nom`, `Gen`, `Dat`, `Acc`, `Ins`, `Loc`, `Voc` |
| `gen1`, `acc2`, `loc1` | `Case`: `Gen`, `Acc`, `Loc` |
| `gen2` («стакан чаю») | `Case=Par` |
| `loc2` («в лесу») | `Case=Loc` with the `loc2` label |
| `sing`, `plur` | `Number`: `Sing`, `Plur` |
| `masc`, `femn`, `neut` | `Gender`: `Masc`, `Fem`, `Neut` |
| `ms-f` («сирота») | `Gender=Masc` |
| `anim`, `inan` | `Animacy`: `Anim`, `Inan` |
| `1per`, `2per`, `3per` | `Person`: `1`, `2`, `3` |
| `pres`, `past`, `futr` | `Tense`: `Pres`, `Past`, `Fut` |
| `perf`, `impf` | `Aspect`: `Perf`, `Imp` |
| `indc`, `impr` | `Mood`: `Ind`, `Imp` |
| `actv`, `pssv` | `Voice`: `Act`, `Pass`; reflexive verbs - `Mid` |
| `Name`, `Surn`, `Patr`, `Geox`, `Orgn`, `Trad` | `NameType`: `Giv`, `Sur`, `Pat`, `Geo`, `Com`, `Pro`; `labels` for adjectives |
| `Supr` | `Degree=Sup` |
| `Abbr` | `Abbr=Yes`; a label in `labels` for declinable abbreviations («вуз») |
| `Abbr` and `Anum` together («1-го», «90-х») | `NumType=Ord`, `NumForm=Combi`, without `Abbr` |
| `Poss` | `Poss=Yes`; a label in `labels` for adjectives |
| `Fixd` | `InflClass=Ind` for nouns except abbreviations, a label in `labels` for other words |
| `Apro` | the `DET` part of speech |
| `Dmns`, `Ques` («там», «где») | `PronType`: `Dem`, `Rel`, `Int` |
| `Anum` on words («первый»), `GNdr` | none |
| the rest: `Infr`, `Slng`, `Arch`, `Qual`, `tran`, `intr`, `V-oy` and others | `labels` |

The `Voice` feature is present on all verb forms and `Degree` on all adverbs and on adjectives except ordinals written in Arabic or Roman numerals («1-го», «2026», «XX»), although in OpenCorpora only participles have voice grammemes and only superlatives and comparatives have degree.

## Differences in behavior { #differences }

- **Lemmas** follow SynTagRus: «Ивановой» - «иванова», not «иванов», «важнейшей» - «важнейший», not «важный», «денег» - «деньги», not «деньга».
- **Analyses** that coincide in UD (the conjunction and the adverb «так») are merged into one with the sum of their probabilities, so there may be fewer analyses than in pymorphy.
- **Forms** from `inflect` and `agree` are strings, not analyses; when there is no form, they raise [`FormNotFoundError`](exceptions.md), a subclass of `LookupError`.
- **Tokens outside the dictionary** are [analyzed by rules](analysis.md#rules) with UD parts of speech and features: «2005-го» is an ordinal adjective in the genitive with forms («2005-му»), not an indeclinable noun as in pymorphy3.
- **Speed.** Analysis and lemmatization are 6-8 times faster than with `pymorphy3[fast]` and 34-62 times faster than with pymorphy3 without it, and lists of words are processed without the GIL, see [Comparison](performance.md).
