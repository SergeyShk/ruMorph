# Inflection

Every [analysis](analysis.md) knows its lexeme - all forms of the word with the same lemma. From an analysis you can get the list of forms, the form with given UD features or the form that agrees with a number. The lexeme is defined by the lemma, so it follows the UD lemma conventions: the lexeme of «Ивановой» is the feminine forms of the surname «Иванова», the lexeme of «важнейшего» is the superlative forms.

The word forms are spelled as in the dictionary: lowercase and with «ё» where the dictionary has it; the forms of guessed words are described in [Unknown words](analysis.md#guess).

## Methods

<div class="metric-card" markdown>

### Forms of the lexeme { #forms }

`forms` · `list[Analysis]`
{ .metric-meta }

``` {.python .signature}
forms()
```

The forms of the lexeme in dictionary order - analyses like those of `analyze`, with probability 1. Form variants differ in labels: «кошкою» has `V-oy`, the second locative «в лесу» has `loc2` (its `Case=Loc` feature is the same as in «о лесе»): «лес» with `inflect(Case="Loc")` gives «лесе», and «лесу» is chosen from `forms()` by the `loc2` label.

=== "Code"

    ``` python
    from rumorph import Analyzer

    morph = Analyzer()
    cat = morph.analyze("кошка")[0]
    [form.word for form in cat.forms()]
    form = cat.forms()[8]
    form.word, form.feats
    ```

=== "Result"

    ```
    ['кошка',
     'кошки',
     'кошке',
     'кошку',
     'кошкой',
     'кошкою',
     'кошке',
     'кошки',
     'кошек',
     'кошкам',
     'кошек',
     'кошками',
     'кошках']
    ('кошек', {'Animacy': 'Anim', 'Case': 'Gen', 'Gender': 'Fem', 'Number': 'Plur'})
    ```

</div>

<div class="metric-card" markdown>

### Form by features { #inflect }

`inflect` · `str`
{ .metric-meta }

``` {.python .signature}
inflect(**feats)
```

The form of the lexeme with the given UD features. Features that are not given are kept where possible: «кошка» in the genitive is «кошки», not «кошек». The short form, the animate accusative («нового») and the inclusive imperative («сделаем») are chosen only when asked for: `Variant="Short"`, `Animacy="Anim"`, `Person="1"`. The future of «быть» in SynTagRus is `Tense=Pres` with the `futr` label: «будет» in the plural is «будут», while `inflect(Tense="Fut")` of «быть» raises `FormNotFoundError` - the future is taken from `forms()` by the `futr` label.

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `**feats` | `str` | - | UD features and their values: `Case="Gen"`, `Number="Plur"` |

Exceptions:

- `FormNotFoundError` - the lexeme has no form with these features;
- `ValueError` - the feature or the value is not in the dictionary (a typo like `Case="Genitive"`), the message lists the valid ones;
- `TypeError` - a feature value that is not a string.

=== "Code"

    ``` python
    cat.inflect(Case="Gen"), cat.inflect(Case="Gen", Number="Plur")
    verb = morph.analyze("читал")[0]
    verb.inflect(Gender="Fem"), verb.inflect(VerbForm="Inf")
    verb.inflect(Tense="Pres", Person="3", Number="Plur")
    morph.analyze("новый")[0].inflect(Degree="Cmp")
    ```

=== "Result"

    ```
    ('кошки', 'кошек')
    ('читала', 'читать')
    'читают'
    'новее'
    ```

</div>

<div class="metric-card" markdown>

### Agreement with a number { #agree }

`agree` · `str`
{ .metric-meta }

``` {.python .signature}
agree(number)
```

The form of a noun, an adjective, a pronominal adjective or a participle that agrees with a number. The case of the analysis is kept: «рублях» with 2 is «рублях» («о 2 рублях»), the accusative of animate nouns gives «вижу двух котов». Adjectives and nouns declined like adjectives agree by gender: «две новые», «два новых», «два учёных», «две столовые». After a number the counting form is taken: «5 вольт», «5 лет», «5 человек». Words without a singular take the plural: «1 сутки», «2 суток». Feminine surnames agree like adjectives: «две Ивановы», «пять Ивановых». A negative number agrees as its absolute value, a fractional one («1,5», `Decimal`, `Fraction`) takes the genitive singular in any case: «1,5 рубля», «о 2,5 рубля».

| Parameter | Type | Default | Description |
| :-------- | :--- | :------ | :---------- |
| `number` | `int` / `float` / `Decimal` / `Fraction` | - | The number; an integer is any number with `__index__` |

Exceptions:

- `FormNotFoundError` - the word does not agree with numbers (a verb, an adverb) or the lexeme has no needed form («5 молока»);
- `TypeError` - a value that is not a number;
- `ValueError` - infinity or NaN.

=== "Code"

    ``` python
    ruble = morph.analyze("рубль")[0]
    [f"{n} {ruble.agree(n)}" for n in (1, 2, 5, 21)]
    morph.analyze("сутки")[0].agree(2)
    ```

=== "Result"

    ```
    ['1 рубль', '2 рубля', '5 рублей', '21 рубль']
    'суток'
    ```

</div>
