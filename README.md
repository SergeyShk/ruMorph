<p align="center">
  <img src="https://raw.githubusercontent.com/SergeyShk/ruMorph/master/docs/img/rumorph.png" alt="ruMorph" width="480">
</p>

<h1 align="center">ruMorph</h1>

<p align="center">
  <b>Russian Morphology</b> - быстрый морфологический анализатор русского языка на Rust с API для Python
</p>

<p align="center">
  <a href="https://sergeyshk.github.io/ruMorph/">Документация</a> ·
  <a href="https://pypi.org/project/rumorph/">PyPI</a> ·
  <a href="https://crates.io/crates/rumorph">crates.io</a> ·
  <a href="https://github.com/SergeyShk/ruMorph/blob/master/README.en.md">English</a>
</p>

<p align="center">
  <a href="https://pypi.org/project/rumorph/"><img src="https://img.shields.io/pypi/v/rumorph?logo=pypi&logoColor=FFE873" alt="Версия"></a>
  <a href="https://pypi.org/project/rumorph/"><img src="https://img.shields.io/pypi/pyversions/rumorph.svg?logo=python&logoColor=FFE873" alt="Поддерживаемые версии Python"></a>
  <a href="https://crates.io/crates/rumorph"><img src="https://img.shields.io/crates/v/rumorph?logo=rust" alt="Крейт"></a>
  <a href="https://github.com/SergeyShk/ruMorph/actions/workflows/ci.yml"><img src="https://github.com/SergeyShk/ruMorph/actions/workflows/ci.yml/badge.svg" alt="Сборка"></a>
  <a href="https://github.com/astral-sh/ruff"><img src="https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json" alt="Ruff"></a>
  <a href="https://github.com/SergeyShk/ruMorph/blob/master/LICENSE.txt"><img src="https://img.shields.io/github/license/sergeyshk/rumorph.svg" alt="Лицензия"></a>
  <a href="https://doi.org/10.5281/zenodo.23520738"><img src="https://zenodo.org/badge/DOI/10.5281/zenodo.23520738.svg" alt="DOI"></a>
</p>

---

**ruMorph** - морфологический анализатор русского языка: ядро на Rust и API для Python. Для словоформы анализатор находит по словарю [OpenCorpora](https://opencorpora.org) варианты разбора - начальную форму (лемму), часть речи и признаки [Universal Dependencies](https://universaldependencies.org/) - упорядочивает их по вероятности и строит формы слова.

Разбор и лемматизация в 7-8 раз быстрее, чем в pymorphy3 с ускорением `pymorphy3[fast]`, форма по признакам и согласование с количеством - в 17-18 раз; списки слов разбираются без GIL. Части речи, признаки и леммы следуют соглашениям трибанка SynTagRus, поэтому результат можно сравнивать с разметкой spaCy, Stanza и других инструментов на UD.

* **[Анализатор](https://sergeyshk.github.io/ruMorph/analyzer/)** - разборы словоформы по убыванию вероятности, разбор списков слов и лемматизация без GIL, проверка словоформы по словарю
* **[Части речи и признаки UD](https://sergeyshk.github.io/ruMorph/analysis/#ud)** - части речи, признаки и леммы в соглашениях SynTagRus; пометы OpenCorpora, у которых нет признака UD
* **[Токены вне словаря](https://sergeyshk.github.io/ruMorph/analysis/#rules)** - числа, знаки, латиница, инициалы и сокращения по правилам
* **[Незнакомые слова](https://sergeyshk.github.io/ruMorph/analysis/#guess)** - разборы по окончанию, слова через дефис и с приставками, формы угаданных слов
* **[Словоизменение](https://sergeyshk.github.io/ruMorph/inflection/)** - формы лексемы, форма по признакам UD, согласование с количеством
* **[Команда rumorph](https://sergeyshk.github.io/ruMorph/cli/)** - разбор файлов и потоков: леммы, CoNLL-U, TSV и JSON Lines
* **[Переход с pymorphy](https://sergeyshk.github.io/ruMorph/migration/)** - соответствие вызовов pymorphy2 и pymorphy3, частей речи и граммем
* **[Сравнение](https://sergeyshk.github.io/ruMorph/performance/)** - точность и скорость в сравнении с pymorphy3, mystem, spaCy, Stanza и Slovnet, загрузка словаря и память

> [!NOTE]
> Ранняя версия: до 1.0 API может меняться между версиями 0.x, изменения описываются в заметках к выпускам.

## Установка

Требуется Python 3.11 или новее.

```bash
pip install rumorph
```

Или с помощью [uv](https://docs.astral.sh/uv/):

```bash
uv add rumorph
```

Колеса собираются для Linux (в том числе Alpine), macOS и Windows, включая Python 3.14t без GIL; Rust для установки не нужен, словарь входит в пакет. На других платформах pip соберет пакет из исходников - для этого нужен [Rust](https://www.rust-lang.org/tools/install) 1.88 или новее. Подробнее - на странице [Установка](https://sergeyshk.github.io/ruMorph/installation/).

Для Rust - крейт [`rumorph`](https://crates.io/crates/rumorph) со словарем (`cargo add rumorph`, документация - на [docs.rs](https://docs.rs/rumorph)); команда `rumorph` без Python ставится через `cargo install rumorph --features cli`.

## Быстрый старт

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> morph.lemmatize("Мы стали друзьями в ковиде".split())
['мы', 'стать', 'друг', 'в', 'ковид']
>>> [(a.lemma, a.pos, a.feats.get("Case"), round(a.prob, 3)) for a in morph.analyze("стали")][:2]
[('стать', 'VERB', None, 0.988), ('сталь', 'NOUN', 'Gen', 0.005)]
>>> morph.analyze("кошка")[0].inflect(Case="Gen", Number="Plur")
'кошек'
>>> morph.analyze("рубль")[0].agree(5)
'рублей'
```

Анализатор достаточно создать один раз: при создании он загружает словарь, дальше им можно пользоваться во всей программе.

## Возможности

### Разбор словоформы

Метод `analyze` возвращает разборы словоформы по убыванию вероятности - объекты `Analysis` с леммой, частью речи, признаками UD, пометами словаря, вероятностью и источником разбора. Регистр для поиска не важен, но меняет доли имен собственных («шаров» - «шар», «Шаров» - фамилия), буква «е» может обозначать «ё». Разборы неизменяемы: их можно сравнивать, класть в множества и сохранять через pickle.

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> morph.analyze("стали")[0]
Analysis(word='стали', lemma='стать', pos='VERB', feats='Aspect=Perf|Mood=Ind|Number=Plur|Tense=Past|VerbForm=Fin|Voice=Mid', labels='intr', prob=0.9877829877829878, source='dictionary')
>>> [len(analyses) for analyses in morph.analyze_many(["мама", "мыла", "раму"])]
[1, 4, 2]
>>> morph.is_known("Ежи"), morph.is_known("вайб")
(True, False)
```

Списки слов методы `analyze_many` и `lemmatize` разбирают без GIL, поэтому несколько потоков с одним анализатором работают параллельно; в Python 3.14t без GIL параллельно работают и `analyze`, `inflect` и `agree`. Анализатор и разборы сохраняются через pickle и работают с `multiprocessing` и joblib.

Подробнее - в документации: [анализатор](https://sergeyshk.github.io/ruMorph/analyzer/), [разбор словоформы](https://sergeyshk.github.io/ruMorph/analysis/).

<details>
<summary><b>Части речи и признаки Universal Dependencies</b></summary>

<br>

Части речи и признаки следуют [Universal Dependencies](https://universaldependencies.org/u/pos/) в соглашениях крупнейшего русского трибанка [UD_Russian-SynTagRus](https://github.com/UniversalDependencies/UD_Russian-SynTagRus). Отличия от частей речи OpenCorpora:

*   «который», «этот», «свой», «весь», «такой» и другие местоименные прилагательные - `DET`
*   «быть» и «бы» - `AUX`
*   вводные слова «однако», «например», «конечно» - `ADV`
*   «можно», «надо», «нельзя» - `VERB` без признаков
*   предлоги - `ADP`, сочинительные союзы - `CCONJ`, подчинительные - `SCONJ`, частицы - `PART`

Признаки: падеж, число, род, одушевленность, лицо, время, вид, наклонение, залог, форма глагола, степень, краткая форма, тип имени собственного, сокращение, притяжательность, тип числительного и его запись, тип местоимения, возвратность, несклоняемость, отрицание. Леммы тоже следуют SynTagRus: «Ивановой» - «иванова», «важнейшей» - «важнейший», «денег» - «деньги». Граммемы OpenCorpora, для которых в UD нет признака, - стиль, переходность, второй предложный падеж и другие - лежат в пометах `labels`.

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> words = ["который", "был", "конечно", "можно", "Ивановой", "денег"]
>>> [(analyses[0].lemma, analyses[0].pos) for analyses in morph.analyze_many(words)]
[('который', 'DET'), ('быть', 'AUX'), ('конечно', 'ADV'), ('можно', 'VERB'), ('иванова', 'PROPN'), ('деньги', 'NOUN')]
>>> lesu = morph.analyze("лесу")[0]
>>> lesu.feats["Case"], lesu.labels
('Loc', frozenset({'loc2'}))
```

Подробнее - в [документации](https://sergeyshk.github.io/ruMorph/analysis/#ud); соответствие граммем OpenCorpora признакам UD - на странице [Переход с pymorphy](https://sergeyshk.github.io/ruMorph/migration/#grammemes).

</details>

<details>
<summary><b>Токены вне словаря</b></summary>

<br>

Числа, знаки, латиницу и сокращения анализатор разбирает по правилам, `source` у таких разборов - `"rule"`:

*   знаки препинания - `PUNCT`, символы `%`, `$`, `№` и эмодзи - `SYM`
*   числа цифрами - `NUM`; целое число может быть и порядковым `ADJ`, у годов это первый разбор
*   числа с окончанием «2005-го», «5-ти» - порядковые `ADJ` с падежом или количественные `NUM`, римские числа - порядковые `ADJ`
*   слова латиницей - `X` с `Foreign=Yes`
*   инициалы «А.» - имя, отчество или фамилия с `Abbr=Yes`
*   сокращения с точкой «т.», «им.» - разборы словарного сокращения, аббревиатуры капсом «НКО» - `PROPN` или `NOUN` с `Abbr=Yes`
*   наречия «по-бурятски», «по-молодому» - `ADV`
*   прочие токены «км/ч», «100%-ный» - `X` без признаков, лемма - токен в нижнем регистре

Доли неоднозначных разборов взяты из частот в трибанках UD Taiga и GSD, доли имен собственных - с учетом регистра: «Земли» - прежде всего планета (PROPN), «земли» - NOUN.

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> [(a.lemma, a.pos, a.feats["NumType"], round(a.prob, 2)) for a in morph.analyze("2026")]
[('2026', 'ADJ', 'Ord', 0.98), ('2026', 'NUM', 'Card', 0.02)]
>>> [(a.lemma, a.pos, a.feats["Case"]) for a in morph.analyze("2005-го")][:1]
[('2005-й', 'ADJ', 'Gen')]
>>> [(a.pos, a.feats, a.source) for a in morph.analyze("iPhone")]
[('X', {'Foreign': 'Yes'}, 'rule')]
```

Подробнее - в [документации](https://sergeyshk.github.io/ruMorph/analysis/#rules).

</details>

<details>
<summary><b>Незнакомые слова</b></summary>

<br>

Разборы слова кириллицей, которого нет в словаре, анализатор угадывает по окончанию: берет формы словарных слов с тем же окончанием - их часть речи, признаки и словоизменение. `source` у таких разборов - `"guess"`.

*   Слово в нижнем регистре угадывается как нарицательное, слово с заглавной буквы - в основном как имя собственное: фамилия, имя, топоним
*   Слово через дефис разбирается по последней части: «крэш-тестов» - «крэш-тест», «Мак-Киннона» - «мак-киннон», а слово с протяжкой или заиканием - по словарному слову без них: «не-е-ет» - «нет», «с-спасибо» - «спасибо»
*   Слово с известной приставкой - «эко», «анти», «супер», «микро» и другими - разбирается по словарному слову без нее: «экодомов» - «экодом»
*   У угаданного разбора есть формы, и они сохраняют заглавные буквы: «Мак-Киннона» в дательном падеже - «Мак-Киннону»
*   Слово, которое нужно разбирать точно, можно добавить по образцу - словарному слову, которое изменяется так же: `Analyzer(words={"зумер": "геймер"})`

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> [(a.lemma, a.pos, a.feats["Case"], a.source) for a in morph.analyze("чатботу")][:1]
[('чатбот', 'NOUN', 'Dat', 'guess')]
>>> guess = morph.analyze("Мартова")[0]
>>> guess.lemma, guess.pos, guess.feats["NameType"], guess.inflect(Case="Dat")
('мартов', 'PROPN', 'Sur', 'Мартову')
>>> morph.analyze("крэш-тестов")[0].lemma, morph.analyze("экодомов")[0].lemma
('крэш-тест', 'экодом')
>>> zoomers = Analyzer(words={"зумер": "геймер"})
>>> zoomers.analyze("зумерами")[0].lemma, zoomers.is_known("зумеры")
('зумер', True)
```

Подробнее - в [документации](https://sergeyshk.github.io/ruMorph/analysis/#guess).

</details>

<details>
<summary><b>Словоизменение</b></summary>

<br>

Каждый разбор знает свою лексему - все формы слова с той же леммой:

*   `forms` - формы лексемы в порядке словаря
*   `inflect` - форма с заданными признаками UD; признаки, которые не названы, по возможности сохраняются
*   `agree` - форма существительного, прилагательного или причастия, согласованная с количеством: «1 рубль», «2 рубля», «5 рублей», «2 суток»

Если у лексемы нет нужной формы, `inflect` и `agree` бросают `FormNotFoundError`, а на опечатку в признаке `inflect` бросает `ValueError` со списком допустимых значений.

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> cat = morph.analyze("кошка")[0]
>>> [form.word for form in cat.forms()][:5]
['кошка', 'кошки', 'кошке', 'кошку', 'кошкой']
>>> cat.inflect(Case="Gen"), cat.inflect(Case="Gen", Number="Plur")
('кошки', 'кошек')
>>> verb = morph.analyze("читал")[0]
>>> verb.inflect(Gender="Fem"), verb.inflect(Tense="Pres", Person="3", Number="Plur")
('читала', 'читают')
>>> ruble = morph.analyze("рубль")[0]
>>> [f"{n} {ruble.agree(n)}" for n in (1, 2, 5, 21)]
['1 рубль', '2 рубля', '5 рублей', '21 рубль']
```

Подробнее - в [документации](https://sergeyshk.github.io/ruMorph/inflection/).

</details>

<details>
<summary><b>Команда rumorph</b></summary>

<br>

Команда ставится вместе с пакетом и разбирает текст из файлов или стандартного ввода: леммы, CoNLL-U, TSV или JSON Lines со всеми разборами. На входе - текст, токены по одному в строке или CoNLL-U, в котором заполняются LEMMA, UPOS и FEATS.

```bash
$ echo "Мы стали друзьями в ковиде." | rumorph
мы стать друг в ковид .
$ rumorph --format conllu text.txt > text.conllu
```

Подробнее - в [документации](https://sergeyshk.github.io/ruMorph/cli/): параметры, форматы и коды выхода.

</details>

<details>
<summary><b>Переход с pymorphy</b></summary>

<br>

ruMorph работает с тем же словарем OpenCorpora, что pymorphy2 и pymorphy3, но API у него свое: части речи и признаки UD вместо граммем OpenCorpora, вероятность вместо `score`, словоформы строкой, исключения вместо `None`. Здесь `morph` - анализатор, `p` - разбор pymorphy, `a` - разбор ruMorph:

| pymorphy2, pymorphy3 | ruMorph |
| :------------------- | :------ |
| `MorphAnalyzer()` | `Analyzer()` |
| `morph.parse(word)` | `morph.analyze(word)` |
| `[morph.parse(w)[0].normal_form for w in words]` | `morph.lemmatize(words)` |
| `morph.word_is_known(word)` | `morph.is_known(word)` |
| `p.normal_form`, `p.tag.POS`, `p.score` | `a.lemma`, `a.pos`, `a.prob` |
| `p.tag.case`, `p.tag.number` | `a.feats.get("Case")`, `a.feats.get("Number")` |
| `p.lexeme` | `a.forms()` |
| `p.inflect({"gent", "plur"}).word` | `a.inflect(Case="Gen", Number="Plur")` |
| `p.make_agree_with_number(n).word` | `a.agree(n)` |

Подробнее - в [документации](https://sergeyshk.github.io/ruMorph/migration/): все вызовы, части речи и граммемы.

</details>

<details>
<summary><b>Сравнение</b></summary>

<br>

Доля слов, у которых верны часть речи и лемма первого разбора, % (тестовые части трибанков UD 2.18):

| Система | SynTagRus | Taiga | GSD | Токенов в секунду |
| :------ | :-------- | :---- | :-- | :---------------- |
| ruMorph | **93,7** | **90,5** | **92,1** | **647 922** |
| pymorphy3 | 81,4 | 79,8 | 83,1 | 23 216 |
| mystem | 81,9 | 79,4 | 79,9 | 33 171 |

Теггеры с контекстом точнее в признаках, но намного медленнее: Stanza - 2 795 токенов в секунду, spaCy ru_core_news_sm - 6 977. Загрузка словаря ruMorph занимает 5 мс против 0,2-2,6 с у теггеров, процессы делят словарь, и каждый следующий добавляет 1,4 МБ. Замер - Apple M3 Pro, Python 3.11.

Подробнее - в [документации](https://sergeyshk.github.io/ruMorph/performance/): теггеры с контекстом, график скорости от объема текста, операции в сравнении с pymorphy3, память и как это измерено.

</details>

## Разработка

Проект использует [uv](https://docs.astral.sh/uv/) для управления зависимостями, [maturin](https://www.maturin.rs/) для сборки модуля на Rust и [ruff](https://docs.astral.sh/ruff/) для линтинга и форматирования. Нужен [Rust](https://www.rust-lang.org/tools/install) 1.88 или новее.

```bash
git clone https://github.com/SergeyShk/ruMorph.git
cd ruMorph

make deps        # создать окружение, установить зависимости и собрать модуль на Rust
make dictionary  # собрать словарь из pymorphy3-dicts-ru и трибанков UD
make test        # запустить тесты Python, примеры из докстрингов (doctest) и тесты Rust
make lint        # ruff, mypy, rustfmt и clippy
```

Полный список команд - `make help`. Документация двуязычная: русские страницы `docs/*.md`, английские - `docs/*.en.md` рядом с ними; при правке страницы обновляйте обе версии.

Перед отправкой изменений стоит установить хуки, которые прогонят линтеры на коммите и тесты на пуше:

```bash
uv run pre-commit install
```

## Участие в проекте

Баг-репорты, идеи и пул-реквесты приветствуются - [issues](https://github.com/SergeyShk/ruMorph/issues) открыты. Как устроен процесс и что проверить перед отправкой пул-реквеста - в [CONTRIBUTING.md](https://github.com/SergeyShk/ruMorph/blob/master/CONTRIBUTING.md); правила общения - в [кодексе поведения](https://github.com/SergeyShk/ruMorph/blob/master/CODE_OF_CONDUCT.md).

<details>
<summary><b>Структура проекта</b></summary>

<br>

*   **crates/rumorph** - ядро анализатора на Rust: формат словаря, сборка, разбор и словоизменение
*   **crates/rumorph-build** - сборка словаря из выгрузки pymorphy3-dicts-ru, перевод тегов OpenCorpora в UD и частоты разборов по трибанкам UD
*   **crates/rumorph-dict-ru** - крейт со словарем для `rumorph` на crates.io
*   **crates/rumorph-py** - модуль Python `rumorph._rumorph` на PyO3
*   **python/rumorph** - пакет Python
*   **scripts** - выгрузка словаря pymorphy3-dicts-ru и проверка установленного пакета перед публикацией
*   **docs** - документация MkDocs на русском и английском
*   **tests** - тесты пакета Python

</details>

## Авторы

*   Шкарин Сергей (kouki.sergey@gmail.com)

## Лицензия

Код распространяется под лицензией [MIT](https://github.com/SergeyShk/ruMorph/blob/master/LICENSE.txt). Словарь в пакете собран из словаря [OpenCorpora](https://opencorpora.org) с частотами по трибанкам [UD_Russian-Taiga](https://github.com/UniversalDependencies/UD_Russian-Taiga) и [UD_Russian-GSD](https://github.com/UniversalDependencies/UD_Russian-GSD) и распространяется по лицензии CC BY-SA 4.0 ([LICENSE-DICTIONARY.txt](https://github.com/SergeyShk/ruMorph/blob/master/LICENSE-DICTIONARY.txt)).

## Цитирование

Если вы используете **ruMorph** в исследованиях или программах, пожалуйста, цитируйте библиотеку по следующей записи BibTeX - это помогает развитию и поддержке проекта. Те же метаданные лежат в [CITATION.cff](https://github.com/SergeyShk/ruMorph/blob/master/CITATION.cff) - GitHub показывает их по кнопке «Cite this repository». Concept DOI [10.5281/zenodo.23520738](https://doi.org/10.5281/zenodo.23520738) на Zenodo указывает на все версии библиотеки; DOI конкретной версии - на странице релиза.

```bibtex
@software{ruMorph,
  author = {Sergey Shkarin},
  title = {{ruMorph, a fast morphological analyzer for Russian}},
  year = 2026,
  doi = {10.5281/zenodo.23520738},
  url = {https://github.com/SergeyShk/ruMorph}
}
```
