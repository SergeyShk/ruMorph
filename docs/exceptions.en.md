# Exceptions

The library raises built-in Python exceptions and one of its own, `FormNotFoundError`, a subclass of `LookupError`. It is available from `rumorph`.

| Exception | Built-in class | When it is raised |
| :-------- | :------------- | :---------------- |
| `FormNotFoundError` | `LookupError` | The lexeme has no form with the requested features ([`inflect`](inflection.md#inflect)) or no form that agrees with the number, or the word does not agree with numbers ([`agree`](inflection.md#agree)) |
| `ValueError` | - | The feature or the value is not in the dictionary (`inflect`); the file is not a ruMorph dictionary or is damaged ([`Analyzer`](analyzer.md)) |
| `TypeError` | - | An argument of the wrong type: a word or a feature value that is not a string, a number that is not an integer, a dictionary path that is neither a string nor `PathLike`, a string instead of a collection of words ([`analyze_many`](analyzer.md#analyze_many), [`lemmatize`](analyzer.md#lemmatize)) |
| `FileNotFoundError` | `OSError` | The dictionary file does not exist (`Analyzer`) |
| `PermissionError`, `IsADirectoryError` and others | `OSError` | The dictionary file cannot be read (`Analyzer`) |

Exception messages are in Russian; `TypeError` messages about the type of an argument may be in English.

=== "Code"

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

=== "Result"

    ```
    FormNotFoundError True
    Неизвестное значение признака Case: Genitive; есть Acc, Dat, Gen, Ins, Loc, Nom, Par, Voc
    ```
