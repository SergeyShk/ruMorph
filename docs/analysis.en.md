# Word analysis

An analysis is an `Analysis` object returned by the `analyze` and `analyze_many` methods of the [analyzer](analyzer.md): the lemma, the part of speech and the [Universal Dependencies](https://universaldependencies.org/) features, the probability and the source of the analysis. Analyses are immutable; they can be compared (the probability takes no part in the comparison: an analysis equals its form from `forms()`), put into sets and pickled. The forms of a word are given by the `forms`, `inflect` and `agree` methods - see [Inflection](inflection.md).

=== "Code"

    ``` python
    from rumorph import Analyzer

    morph = Analyzer()
    analysis = morph.analyze("стали")[0]
    analysis.lemma, analysis.pos, analysis.feats, analysis.labels
    ```

=== "Result"

    ```
    ('стать',
     'VERB',
     {'Aspect': 'Perf',
      'Mood': 'Ind',
      'Number': 'Plur',
      'Tense': 'Past',
      'VerbForm': 'Fin',
      'Voice': 'Mid'},
     frozenset({'intr'}))
    ```

## Attributes { #attributes }

| Attribute | Type | Description |
| :-------- | :--- | :---------- |
| `word` | `str` | The word form as spelled in the dictionary: with «ё» if the dictionary has it; for analyses by rules and guessed ones - as in the text, but guessed ones take «ё» from the ending of the dictionary form («пингвиненка» - «пингвинёнка») |
| `lemma` | `str` | Initial form |
| `pos` | `str` | UD part of speech: `NOUN`, `VERB`, `ADJ`, `DET`, `ADP` and so on |
| `feats` | `dict[str, str]` | UD features: `{"Case": "Gen", "Number": "Sing"}`; a new dictionary each time, changing it does not change the analysis |
| `labels` | `frozenset[str]` | Dictionary labels that have no UD feature: `Infr` (informal) and others |
| `prob` | `float` | Probability of the analysis; it sums to 1 over all analyses of the word form |
| `source` | `str` | Where the analysis comes from: `"dictionary"` - the word form is in the dictionary, `"rule"` - an [analysis by rule](#rules), `"guess"` - a [guessed analysis](#guess) of an unknown word |

## Parts of speech and features { #ud }

Parts of speech and features follow [Universal Dependencies](https://universaldependencies.org/u/pos/) as annotated in the largest Russian treebank, [UD_Russian-SynTagRus](https://github.com/UniversalDependencies/UD_Russian-SynTagRus), so the output can be compared with spaCy, Stanza and other UD tools. Differences from the OpenCorpora parts of speech:

- «который», «этот», «свой», «весь», «такой» and the other pronominal adjectives are `DET`;
- «быть» and «бы» are `AUX`;
- the parenthetical words «однако», «например», «конечно» are `ADV`;
- «можно», «надо», «нельзя» are `VERB` without features;
- prepositions are `ADP`, coordinating conjunctions `CCONJ`, subordinating ones `SCONJ`, particles `PART`.

Features: case, number, gender, animacy, person, tense, aspect, mood, voice, verb form, degree, the short form (`Variant=Short`), the type of a proper name (`NameType`), abbreviation (`Abbr`), possessiveness (`Poss`), the type of a numeral (`NumType`) and how it is written (`NumForm`), the type of a pronoun or pronominal adverb (`PronType`: «там» - `Dem`, «где» - `Rel`, «никогда» - `Neg`), reflexivity (`Reflex`), indeclinability (`InflClass=Ind`), negation (`Polarity=Neg`).

Lemmas follow SynTagRus too: feminine surnames and patronymics keep the feminine lemma («Ивановой» - «иванова»), superlatives stay superlative («важнейшей» - «важнейший»), «денег» - «деньги», abbreviations get the full word as the lemma («млн» - «миллион», «г.» - «год»). Nouns that have only the plural get the gender of their declension, as in SynTagRus: «выборы» is masculine, «сутки» feminine, «ворота» neuter. When several OpenCorpora analyses coincide in UD (the conjunction and the adverb «так»), they are merged into one with the sum of their probabilities.

The `labels` are the [OpenCorpora](https://opencorpora.org/dict.php?act=gram) grammemes that have no UD feature: style (`Infr`, `Slng`, `Arch`), qualitative adjective (`Qual`), parenthetical word (`Prnt`), transitivity (`tran`, `intr`), whether the speaker is included in an imperative (`incl` - «пойдемте», `excl` - «иди»), the second locative (`loc2` - «в лесу»), the future of «быть» (`futr` - «будет»: SynTagRus gives it `Tense=Pres`) and others.

## Tokens outside the dictionary { #rules }

Numbers, punctuation, Latin words and abbreviations are analyzed by rules, their `source` is `"rule"`:

| Token | Analysis |
| :---- | :------- |
| punctuation; symbols `%`, `$`, `№`, `−`, emoji | `PUNCT`; `SYM` |
| numbers in digits and ranges: «15», «3,14», «10 000», «17:04», «1990-2000» | `NUM` with `NumForm=Digit`, `NumType=Card`; an integer may also be an ordinal `ADJ` with `NumType=Ord` - for 1000-2100 (years), except round hundreds, this is the first analysis, and 1900 and 2000 are years too |
| numbers and ranges with an ending: «2005-го», «10-15-го», «5-ти» | an ordinal `ADJ` with the case and forms of «1-го», or a cardinal `NUM` |
| Roman numerals and ranges: «XX», «XIX-го», «II-III» | an ordinal `ADJ` with `NumForm=Roman` |
| Latin words: «iPhone» | `X` with `Foreign=Yes` |
| initials: «А.», «А» | `PROPN` with `Abbr=Yes`: a given name, a patronymic or a surname |
| abbreviations with a dot: «т.», «им.» | the analyses of the dictionary abbreviation without the dot |
| capitalized abbreviations: «НКО» | `PROPN` or `NOUN` with `Abbr=Yes`; a long word in capitals («ЧАТБОТАМИ») is also guessed as an ordinary word |
| adverbs «по-бурятски», «по-молодому» | `ADV` with `Degree=Pos` |
| other tokens: «км/ч», «РосПил» | `X` without features, the lemma is the token in lowercase |

The shares of ambiguous analyses come from the frequencies in the UD Taiga and GSD treebanks. The letters «й» and «ё» in decomposed Unicode and the non-breaking hyphen are converted to the usual ones, and stress marks (also in the composed letters «ѐ», «ѝ»), soft hyphens and invisible word joiners (U+2060, U+FEFF) are removed: «Москва́» is analyzed as «Москва». Latin letters that look like Cyrillic ones in a Cyrillic word are read as Cyrillic («cтали» with a Latin «c» is «стали»), and Cyrillic ones in a Latin word as Latin.

=== "Code"

    ``` python
    [(a.lemma, a.pos, a.feats["NumType"], round(a.prob, 2)) for a in morph.analyze("2026")]
    [(a.lemma, a.pos, a.feats["Case"]) for a in morph.analyze("2005-го")][:1]
    ```

=== "Result"

    ```
    [('2026', 'ADJ', 'Ord', 0.98), ('2026', 'NUM', 'Card', 0.02)]
    [('2005-й', 'ADJ', 'Gen')]
    ```

## Unknown words { #guess }

The analyses of a Cyrillic word that is not in the dictionary are guessed by its ending: the analyzer takes the forms of dictionary words with the same ending - their part of speech, features and inflection. Their `source` is `"guess"`. A lowercase word is guessed as a common word, a capitalized word mostly as a proper noun: a surname, a given name, a place, and a word in «-ский» mostly as an adjective. An abbreviation with an ending («МЭРТа») is guessed like a capitalized word.

A hyphenated word is analyzed by its last part: «крэш-тестов» - «крэш-тест», «IT-компании» - «it-компания»; if the last part is a particle («сколь-нибудь»), by the first one. If both parts inflect, the lemma is made of the lemmas of the parts: «дивана-кровати» - «диван-кровать», and such an analysis has no forms. A word with a known prefix - «эко», «анти», «супер», «микро» and others - is analyzed by the dictionary word without it: «экодомов» - «экодом». These analyses are guessed too.

A guessed analysis has forms, and `forms`, `inflect` and `agree` work with it; the forms keep the capital letters: «Мак-Киннона» in the dative is «Мак-Киннону», «ПИН-КОДОВ» is «ПИН-КОДАМ». A word with a particle inflects by its first part: «смотри-ка» in the plural is «смотрите-ка».

=== "Code"

    ``` python
    [(a.lemma, a.pos, a.feats["Case"], round(a.prob, 2)) for a in morph.analyze("чатботу")][:2]
    guess = morph.analyze("Мартова")[0]
    guess.lemma, guess.pos, guess.feats["NameType"], guess.inflect(Case="Dat")
    morph.analyze("крэш-тестов")[0].lemma, morph.analyze("экодомов")[0].lemma
    ```

=== "Result"

    ```
    [('чатбот', 'NOUN', 'Dat', 0.44), ('чатбота', 'NOUN', 'Acc', 0.4)]
    ('мартов', 'PROPN', 'Sur', 'Мартову')
    ('крэш-тест', 'экодом')
    ```
