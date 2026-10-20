# Inflection

Every analysis knows its lexeme - all forms of the word with the same lemma. From an analysis you can get the list of forms, the form with given UD features or the form that agrees with a number.

## Forms of the lexeme { #forms }

`forms` returns the forms of the lexeme in dictionary order - they are analyses like those of `analyze`, with probability 1:

``` python
>>> from rumorph import Analyzer
>>> morph = Analyzer()
>>> cat = morph.analyze("кошка")[0]
>>> [form.word for form in cat.forms()]
['кошка', 'кошки', 'кошке', 'кошку', 'кошкой', 'кошкою', 'кошке', 'кошки', 'кошек', 'кошкам', 'кошек', 'кошками', 'кошках']
>>> form = cat.forms()[8]
>>> form.word, dict(form.feats)
('кошек', {'Animacy': 'Anim', 'Case': 'Gen', 'Gender': 'Fem', 'Number': 'Plur'})
```

The lexeme is defined by the lemma, so it follows the UD lemma conventions: the lexeme of «Ивановой» is the feminine forms of the surname «Иванова», the lexeme of «важнейшего» is the superlative forms. Form variants differ in labels: «кошкою» has `V-oy`, the second locative «в лесу» has `loc2` (its `Case=Loc` feature is the same as in «о лесе»).

## Form by features { #inflect }

`inflect` takes UD features as keyword arguments and returns the word form:

``` python
>>> cat.inflect(Case="Gen")
'кошки'
>>> cat.inflect(Case="Gen", Number="Plur")
'кошек'
>>> verb = morph.analyze("читал")[0]
>>> verb.inflect(Gender="Fem"), verb.inflect(Tense="Pres", Person="3", Number="Plur"), verb.inflect(VerbForm="Inf")
('читала', 'читают', 'читать')
>>> morph.analyze("новый")[0].inflect(Degree="Cmp")
'новее'
```

Features that are not given are kept where possible: «кошка» in the genitive is «кошки», not «кошек». The short form is chosen only when asked for (`Variant="Short"`). If the lexeme has no form with these features, `inflect` raises `FormNotFoundError`, a subclass of `LookupError`; if the feature or the value is not in the dictionary (a typo like `Case="Genitive"`), it raises `ValueError` with the list of valid values.

## Agreement with a number { #agree }

`agree` makes a noun, an adjective, a pronominal adjective or a participle agree with a number:

``` python
>>> ruble = morph.analyze("рубль")[0]
>>> [f"{n} {ruble.agree(n)}" for n in (1, 2, 5, 21)]
['1 рубль', '2 рубля', '5 рублей', '21 рубль']
```

The case of the analysis is kept: «рублях» with 2 is «рублях» («о 2 рублях»), the accusative of animate nouns gives «вижу двух котов». Adjectives and nouns declined like adjectives agree by gender: «две новые», «два новых», «два учёных», «две столовые». After a number the counting form is taken: «5 вольт», «5 лет», «5 человек». Words without a singular take the plural: «1 сутки», «2 суток». If the needed form does not exist («5 молока») or the word does not agree with numbers (a verb, an adverb), `agree` raises `FormNotFoundError`.

The word forms of `forms`, `inflect` and `agree` are spelled as in the dictionary: lowercase and with the letter «ё» where the dictionary has it.
