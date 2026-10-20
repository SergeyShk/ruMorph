# Переход с pymorphy

У pymorphy2 и pymorphy3 один API: pymorphy3 - продолжение pymorphy2, который не работает на Python 3.11 и новее. ruMorph работает с тем же словарем OpenCorpora, но API у него свое: части речи и признаки [Universal Dependencies](https://universaldependencies.org/) вместо граммем OpenCorpora, вероятность вместо `score`, словоформы строкой, исключения вместо `None`.

``` bash
pip install rumorph
```

Словарь входит в пакет, отдельный пакет словаря не нужен.

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
    ('стать', 'VERB', 'Plur', 0.975)
    >>> cat = morph.analyze("кошка")[0]
    >>> cat.inflect(Case="Gen", Number="Plur")
    'кошек'
    >>> morph.analyze("рубль")[0].agree(5)
    'рублей'
    ```

## Соответствие вызовов { #calls }

Здесь `morph` - анализатор, `p` - разбор pymorphy, `a` - разбор ruMorph.

| pymorphy2, pymorphy3 | ruMorph | Отличия |
| :------------------- | :------ | :------ |
| `MorphAnalyzer()` | [`Analyzer()`](analyzer.md) | |
| `MorphAnalyzer(path)` | `Analyzer(path)` | путь к файлу словаря ruMorph; словари pymorphy не подходят |
| `morph.parse(word)` | [`morph.analyze(word)`](analyzer.md#analyze) | разборы, которые в UD совпадают, объединены |
| `[morph.parse(w) for w in words]` | [`morph.analyze_many(words)`](analyzer.md#analyze_many) | разбор без GIL |
| `[morph.parse(w)[0].normal_form for w in words]` | [`morph.lemmatize(words)`](analyzer.md#lemmatize) | разбор без GIL |
| `morph.normal_forms(word)` | `list(dict.fromkeys(a.lemma for a in morph.analyze(word)))` | |
| `morph.tag(word)` | `[(a.pos, a.feats) for a in morph.analyze(word)]` | |
| `morph.word_is_known(word)` | [`morph.is_known(word)`](analyzer.md#is_known) | |
| `p.word` | `a.word` | у разборов по правилам и угаданных - в регистре текста |
| `p.normal_form` | `a.lemma` | леммы по соглашениям UD |
| `p.tag.POS` | `a.pos` | части речи UD, см. [ниже](#pos) |
| `p.tag.case`, `p.tag.number` и др. | `a.feats.get("Case")`, `a.feats.get("Number")` и др. | признаки UD, см. [ниже](#grammemes) |
| `"Infr" in p.tag` | `"Infr" in a.labels` | в `labels` - граммемы, у которых нет признака UD |
| `p.score` | `a.prob` | |
| `p.methods_stack` | `a.source` | `"dictionary"`, `"rule"` или `"guess"` |
| `p.lexeme` | [`a.forms()`](inflection.md#forms) | |
| `p.inflect({"gent", "plur"}).word` | [`a.inflect(Case="Gen", Number="Plur")`](inflection.md#inflect) | строка; нет формы - `FormNotFoundError` вместо `None` |
| `p.make_agree_with_number(n).word` | [`a.agree(n)`](inflection.md#agree) | строка; нет формы - `FormNotFoundError` вместо `None` |

Аналогов нет у русских названий граммем (`cyr_repr`, `lat2cyr`), разбора начальной формы (`p.normalized`), перебора словаря и настройки предсказателя незнакомых слов.

## Части речи { #pos }

| OpenCorpora | UD |
| :---------- | :- |
| `NOUN` | `NOUN`; имена, фамилии, отчества, топонимы, организации и торговые марки - `PROPN` |
| `ADJF` | `ADJ`; местоименные прилагательные - `DET` |
| `ADJS` | `ADJ` с `Variant=Short` |
| `COMP` | `ADJ` с `Degree=Cmp` |
| `VERB` | `VERB` с `VerbForm=Fin`; «быть» - `AUX` |
| `INFN` | `VERB` с `VerbForm=Inf` |
| `PRTF` | `VERB` с `VerbForm=Part` |
| `PRTS` | `VERB` с `VerbForm=Part` и `Variant=Short` |
| `GRND` | `VERB` с `VerbForm=Conv` |
| `NUMR` | `NUM` с `NumType=Card` |
| `ADVB` | `ADV` |
| `NPRO` | `PRON` |
| `PRED` | `VERB` без признаков |
| `PREP` | `ADP` |
| `CONJ` | `CCONJ`; подчинительные союзы - `SCONJ` |
| `PRCL` | `PART` |
| `INTJ` | `INTJ` |

Отдельные слова получают часть речи по соглашениям SynTagRus: вводные «однако», «например» - `ADV`, «бы» - `AUX` и т. д. (см. [Части речи и признаки](analysis.md#ud)). Токены вне словаря: `NUMB` - `NUM` с `NumForm=Digit`, `ROMN` - `ADJ` с `NumForm=Roman`, `LATN` - `X` с `Foreign=Yes`, `PNCT` - `PUNCT` или `SYM` (см. [Токены вне словаря](analysis.md#rules)).

## Граммемы { #grammemes }

| OpenCorpora | UD |
| :---------- | :- |
| `nomn`, `gent`, `datv`, `accs`, `ablt`, `loct`, `voct` | `Case`: `Nom`, `Gen`, `Dat`, `Acc`, `Ins`, `Loc`, `Voc` |
| `gen1`, `acc2`, `loc1` | `Case`: `Gen`, `Acc`, `Loc` |
| `gen2` («стакан чаю») | `Case=Par` |
| `loc2` («в лесу») | `Case=Loc` с пометой `loc2` |
| `sing`, `plur` | `Number`: `Sing`, `Plur` |
| `masc`, `femn`, `neut` | `Gender`: `Masc`, `Fem`, `Neut` |
| `ms-f` («сирота») | `Gender=Masc` |
| `anim`, `inan` | `Animacy`: `Anim`, `Inan` |
| `1per`, `2per`, `3per` | `Person`: `1`, `2`, `3` |
| `pres`, `past`, `futr` | `Tense`: `Pres`, `Past`, `Fut` |
| `perf`, `impf` | `Aspect`: `Perf`, `Imp` |
| `indc`, `impr` | `Mood`: `Ind`, `Imp` |
| `actv`, `pssv` | `Voice`: `Act`, `Pass`; у возвратных глаголов - `Mid` |
| `Name`, `Surn`, `Patr`, `Geox`, `Orgn`, `Trad` | `NameType`: `Giv`, `Sur`, `Pat`, `Geo`, `Com`, `Pro` |
| `Supr` | `Degree=Sup` |
| `Abbr` | `Abbr=Yes` |
| `Poss` | `Poss=Yes` |
| `Fixd` | `InflClass=Ind` |
| `Apro` | часть речи `DET` |
| `Dmns`, `Ques` | у местоимений - `PronType`, у остальных слов - пометы `labels` |
| `Anum`, `GNdr` | нет |
| остальные: `Infr`, `Slng`, `Arch`, `Qual`, `tran`, `intr`, `V-oy` и др. | пометы `labels` |

Признак `Voice` есть у всех форм глагола, а `Degree` - у всех прилагательных и наречий, хотя в OpenCorpora граммемы залога есть только у причастий, а степени - только у превосходной и сравнительной.

## Отличия в поведении { #differences }

- **Леммы** следуют SynTagRus: «Ивановой» - «иванова», а не «иванов», «важнейшей» - «важнейший», а не «важный», «денег» - «деньги», а не «деньга».
- **Разборы**, которые в UD совпадают (союз и наречие «так»), объединены в один, их вероятности сложены, поэтому разборов бывает меньше, чем у pymorphy.
- **Формы** `inflect` и `agree` возвращают строку, а не разбор; если формы нет, бросают [`FormNotFoundError`](exceptions.md), подкласс `LookupError`.
- **Токены вне словаря** [разбираются по правилам](analysis.md#rules) с частями речи и признаками UD: «2005-го» - порядковое прилагательное в родительном падеже с формами («2005-му»), а не несклоняемое существительное, как у pymorphy3.
- **Скорость.** Разбор и лемматизация примерно в 5 раз быстрее, чем у `pymorphy3[fast]`, и в 25-40 раз - чем у pymorphy3 без него, а списки слов разбираются без GIL, см. [Скорость и память](performance.md).
