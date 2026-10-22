# Исключения

Библиотека поднимает встроенные исключения Python и одно свое - `FormNotFoundError`, подкласс `LookupError`. Он доступен из `rumorph`.

| Исключение | Встроенный класс | Когда поднимается |
| :--------- | :--------------- | :---------------- |
| `FormNotFoundError` | `LookupError` | У лексемы нет формы с запрошенными признаками ([`inflect`](inflection.md#inflect)) или формы, согласованной с количеством, либо слово не согласуется с количеством ([`agree`](inflection.md#agree)) |
| `ValueError` | - | Признака или значения нет в словаре (`inflect`); количество - бесконечность или NaN (`agree`); файл не словарь ruMorph или поврежден ([`Analyzer`](analyzer.md)); у [своего слова](analyzer.md#words) нет образца в словаре, образец неоднозначен, слово не изменяется по нему или в слове пробел, управляющий или невидимый символ (`Analyzer(words=...)`) |
| `TypeError` | - | Аргумент не того типа: слово или значение признака не строка, количество не число, путь к словарю не строка и не `PathLike`, строка вместо набора слов ([`analyze_many`](analyzer.md#analyze_many), [`lemmatize`](analyzer.md#lemmatize)), [свои слова](analyzer.md#words) не словарь строк (`Analyzer(words=...)`) |
| `FileNotFoundError` | `OSError` | Нет файла словаря (`Analyzer`) |
| `PermissionError`, `IsADirectoryError` и др. | `OSError` | Файл словаря не читается (`Analyzer`) |

Сообщения исключений - на русском.

=== "Код"

    ``` python
    from rumorph import Analyzer, FormNotFoundError

    morph = Analyzer()
    try:
        morph.analyze("молоко")[0].agree(5)
    except FormNotFoundError as e:
        print(type(e).__name__, isinstance(e, LookupError))
    try:
        morph.analyze("кошка")[0].inflect(Case="Genitive")
    except ValueError as e:
        print(e)
    ```

=== "Результат"

    ```
    FormNotFoundError True
    Неизвестное значение признака Case: Genitive; есть Acc, Dat, Gen, Ins, Loc, Nom, Par, Voc
    ```
