# spaCy

Компонент `rumorph` подключает ruMorph к конвейеру [spaCy](https://spacy.io/): размечает токены документа частью речи (`pos_`), признаками [Universal Dependencies](https://universaldependencies.org/) (`morph`) и леммой (`lemma_`) по [разбору с контекстом](analyzer.md#analyze_sentence). В конвейере он заменяет теггер, морфологизатор и лемматизатор.

``` bash
pip install rumorph[spacy]
```

Компонент регистрируется в spaCy при установке пакета и подключается по имени, импортировать rumorph не нужно. Для Python 3.14t без GIL у spaCy нет готовых колес.

=== "Код"

    ``` python
    import spacy

    nlp = spacy.blank("ru")
    nlp.add_pipe("rumorph")
    doc = nlp("Мост сделан из стали.")
    [(token.lemma_, token.pos_) for token in doc]
    str(doc[3].morph)
    ```

=== "Результат"

    ```
    [('мост', 'NOUN'), ('сделать', 'VERB'), ('из', 'ADP'), ('сталь', 'NOUN'), ('.', 'PUNCT')]
    'Animacy=Inan|Case=Gen|Gender=Fem|Number=Sing'
    ```

## Модели ru_core_news { #models }

В моделях `ru_core_news_*` компонент заменяет морфологизатор и лемматизатор, а парсер и распознавание именованных сущностей остаются. Компонент после парсера берет его границы предложений, а `attribute_ruler` модели переносит часть речи в `tag_`.

=== "Код"

    ``` python
    nlp = spacy.load("ru_core_news_sm", exclude=["morphologizer", "lemmatizer"])
    nlp.add_pipe("rumorph", after="parser")
    nlp.pipe_names
    ```

=== "Результат"

    ```
    ['tok2vec', 'parser', 'rumorph', 'attribute_ruler', 'ner']
    ```

Доля слов с верными частью речи, леммой и признаками UD на тестовых частях трибанков ([Сравнение](performance.md)) и скорость конвейера, токенов в секунду:

| Конвейер | SynTagRus | Taiga | GSD | Скорость |
| :------- | :-------- | :---- | :-- | :------- |
| ru_core_news_sm | 82,0 | 75,5 | 74,8 | 5 291 |
| ru_core_news_sm с rumorph | **93,6** | **90,1** | **81,6** | **8 460** |

## Предложения { #sentences }

Контекстная модель разбирает предложение целиком. Предложения компонент берет из разметки документа - парсера, `senter` или `sentencizer`, а если ее нет, делит документ после знаков конца предложения («.», «!», «?», «…»); предложение длиннее 256 токенов разбирается частями, как в [команде rumorph](cli.md). Пробельные токены получают часть речи `SPACE`, как в моделях spaCy, а лемма у них - сам токен; токен без разбора ruMorph (невидимые символы) - `X` и лемму в нижнем регистре, как в команде rumorph.

## Параметры { #config }

| Параметр | Тип | По умолчанию | Описание |
| :------- | :-- | :----------- | :------- |
| `dictionary` | `str` | `None` | Путь к файлу словаря; по умолчанию - словарь из пакета |
| `words` | `dict` | `None` | [Свои слова](analyzer.md#words) и их образцы; образец с частью речи - кортеж или список |
| `context` | `bool` | `True` | Выбирать разбор по соседним словам; `False` - самый вероятный разбор словоформы, как у [`analyze`](analyzer.md#analyze) |

=== "Код"

    ``` python
    nlp = spacy.blank("ru")
    nlp.add_pipe("rumorph", config={"words": {"зумер": "геймер"}})
    [token.lemma_ for token in nlp("Зумерам нравится")]
    ```

=== "Результат"

    ```
    ['зумер', 'нравиться']
    ```

## Разбор ruMorph { #extension }

`token._.rumorph` - [разбор](analysis.md) ruMorph с леммой, частью речи и признаками токена по словарю и своим словам компонента, разметившего документ: через него доступны [формы слова](inflection.md). Вероятность у него - без учета контекста. Если у словоформы нет такого разбора - например, разметку токена изменил другой компонент, - значение - `None`. Свои словарь и слова компонента документ хранит в `user_data`, поэтому их сохраняют `DocBin(store_user_data=True)` и `span.as_doc(copy_user_data=True)`, а `Doc.from_docs` не переносит.

=== "Код"

    ``` python
    doc = nlp("Мы стали друзьями")
    analysis = doc[2]._.rumorph
    analysis.lemma, analysis.inflect(Case="Dat")
    ```

=== "Результат"

    ```
    ('друг', 'друзьям')
    ```
