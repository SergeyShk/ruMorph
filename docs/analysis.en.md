# Word analysis

The analyzer finds every analysis of a word form in the OpenCorpora dictionary - the normal form, the part of speech and the [Universal Dependencies](https://universaldependencies.org/) features - and orders them by probability.

## Analyzer { #analyzer }

``` python
>>> from rumorph import Analyzer
>>> morph = Analyzer()
```

The dictionary loads in a few milliseconds when the analyzer is created. One analyzer is enough for the whole program, including several threads. By default the dictionary from the package is used; another dictionary file is passed as the `path` argument. The analyzer and the analyses can be pickled, so they work with multiprocessing and joblib: another process loads the dictionary once, however many analyses it receives.

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
| `word` | the word form as spelled in the dictionary: with the letter «ё» if the dictionary has it; for rule and guessed analyses - as in the text, but a guessed one takes «ё» from the ending of the dictionary form («пингвиненка» → «пингвинёнка») |
| `lemma` | the normal form |
| `pos` | the UD part of speech: `NOUN`, `VERB`, `ADJ`, `DET`, `ADP` and so on |
| `feats` | the UD features as a dict `{"Case": "Gen", "Number": "Sing"}`; a new one each time, changing it does not change the analysis |
| `labels` | dictionary labels without a UD feature, as a `frozenset`: `{"Infr"}` (informal) |
| `prob` | the probability of the analysis; it sums to 1 over all analyses of the word form |
| `source` | where the analysis comes from: `"dictionary"` - the word form is in the dictionary, `"rule"` - the token is not in the dictionary and is analyzed by a rule (see [below](#rules)), `"guess"` - the analysis of an unknown word is guessed (see [below](#guess)) |

Case does not matter, and the letter «е» may stand for «ё»: `analyze("ЕЖ")` returns the analyses of the word form «ёж».

## Parts of speech and features { #ud }

Parts of speech and features follow [Universal Dependencies](https://universaldependencies.org/u/pos/) as annotated in the largest Russian treebank, [UD_Russian-SynTagRus](https://github.com/UniversalDependencies/UD_Russian-SynTagRus), so the output can be compared with spaCy, Stanza and other UD tools. Differences from the OpenCorpora parts of speech:

- «который», «этот», «свой», «весь», «такой» and the other pronominal adjectives are `DET`;
- «быть» and «бы» are `AUX`;
- the parenthetical words «однако», «например», «конечно» are `ADV`;
- «можно», «надо», «нельзя» are `VERB` without features;
- prepositions are `ADP`, coordinating conjunctions `CCONJ`, subordinating ones `SCONJ`, particles `PART`.

Features: case, number, gender, animacy, person, tense, aspect, mood, voice, verb form, degree, the short form (`Variant=Short`), the type of a proper name (`NameType`), abbreviation (`Abbr`), possessiveness (`Poss`), the type of a numeral (`NumType`) and how it is written (`NumForm`), the type of a pronoun or pronominal adverb (`PronType`: «там» - `Dem`, «где» - `Rel`, «никогда» - `Neg`), reflexivity (`Reflex`), indeclinability (`InflClass=Ind`), negation (`Polarity=Neg`).

Lemmas follow SynTagRus too: feminine surnames and patronymics keep the feminine lemma («Ивановой» - «иванова»), superlatives stay superlative («важнейшей» - «важнейший»), «денег» - «деньги». When several OpenCorpora analyses coincide in UD (the conjunction and the adverb «так»), they are merged into one with the sum of their probabilities. The forms of a word are given by the `forms`, `inflect` and `agree` methods of the analysis - see [Inflection](inflection.md).

The `labels` are the [OpenCorpora](https://opencorpora.org/dict.php?act=gram) grammemes that have no UD feature: style (`Infr`, `Slng`, `Arch`), qualitative adjective (`Qual`), parenthetical word (`Prnt`), transitivity (`tran`, `intr`), whether the speaker is included in an imperative (`incl` - «пойдемте», `excl` - «иди»), the second locative (`loc2` - «в лесу») and others.

## Tokens outside the dictionary { #rules }

Numbers, punctuation, Latin words and abbreviations are analyzed by rules, their `source` is `"rule"`:

``` python
>>> [(a.lemma, a.pos, a.feats["NumType"], round(a.prob, 2)) for a in morph.analyze("2026")]
[('2026', 'ADJ', 'Ord', 0.94), ('2026', 'NUM', 'Card', 0.06)]
>>> [(a.lemma, a.pos, a.feats["Case"]) for a in morph.analyze("2005-го")][:1]
[('2005-й', 'ADJ', 'Gen')]
```

| Token | Analysis |
|---|---|
| punctuation; symbols `%`, `$`, `№`, `−`, emoji | `PUNCT`; `SYM` |
| numbers in digits: «15», «3,14», «10 000», «17:04» | `NUM` with `NumForm=Digit`, `NumType=Card`; an integer may also be an ordinal `ADJ` with `NumType=Ord` - for 1000-2100 (years), except round hundreds, this is the first analysis |
| numbers with an ending: «2005-го», «5-ти» | an ordinal `ADJ` with the case and forms of «1-го», or a cardinal `NUM` |
| Roman numerals: «XX» | an ordinal `ADJ` with `NumForm=Roman` |
| Latin words: «iPhone» | `X` with `Foreign=Yes` |
| initials: «А.» | `PROPN` with `Abbr=Yes`: a given name, a patronymic or a surname |
| abbreviations with a dot: «т.», «им.» | the analyses of the dictionary abbreviation without the dot |
| capitalized abbreviations: «НКО» | `PROPN` or `NOUN` with `Abbr=Yes` |
| adverbs «по-бурятски», «по-молодому» | `ADV` with `Degree=Pos` |

The shares of ambiguous analyses come from the frequencies in SynTagRus. A token with whitespace (except a number), a control or an invisible character gets no analyses.

## Unknown words { #guess }

The analyses of a Cyrillic word that is not in the dictionary are guessed by its ending: the analyzer takes the forms of dictionary words with the same ending - their part of speech, features and inflection. Their `source` is `"guess"`:

``` python
>>> [(a.lemma, a.pos, a.feats["Case"], round(a.prob, 2)) for a in morph.analyze("чатботу")][:2]
[('чатбот', 'NOUN', 'Dat', 0.44), ('чатбота', 'NOUN', 'Acc', 0.4)]
>>> guess = morph.analyze("Мартова")[0]
>>> guess.lemma, guess.pos, guess.feats["NameType"], guess.inflect(Case="Dat")
('мартов', 'PROPN', 'Sur', 'Мартову')
```

A lowercase word is guessed as a common word, a capitalized word mostly as a proper noun: a surname, a given name, a place. Words in mixed case («ЗуМер») are not guessed.

A hyphenated word is analyzed by its last part: «крэш-тестов» - «крэш-тест», «Мак-Киннона» - «мак-киннон»; if the last part is a particle («сколь-нибудь»), by the first one. A word with a known prefix - «эко», «анти», «супер», «микро» and others - is analyzed by the dictionary word without it: «экодомов» - «экодом». These analyses are guessed too.

A guessed analysis has forms, and `forms`, `inflect` and `agree` work with it; the forms keep the capital letters: «Мак-Киннона» in the dative is «Мак-Киннону», «ПИН-КОДОВ» is «ПИН-КОДАМ». The exception is an analysis with a particle («сколь-нибудь»): it has no forms, `forms` gives only the analysis itself.

## Lists of words { #lists }

`analyze_many` analyzes a collection of word forms, and `lemmatize` returns the lemmas of their most probable analyses:

``` python
>>> morph.lemmatize(["Мы", "стали", "друзьями"])
['мы', 'стать', 'друг']
```

`lemmatize` returns a token without analyses in lower case. Both methods accept any collection of words - a list, a tuple, a generator - and analyze it without the GIL: several threads sharing one analyzer run in parallel.

## Dictionary { #dictionary }

`is_known` checks whether a word form is in the dictionary. The dictionary is OpenCorpora, about 3.1 million word forms; the probabilities of the analyses come from the OpenCorpora corpus: a word written with «е» instead of «ё» gets the frequencies of the spelling with «ё», and the analyses of word forms missing from the corpus are equally probable. The dictionary is distributed under the [CC BY-SA 3.0](https://creativecommons.org/licenses/by-sa/3.0/) license.
