# Морфологический анализатор русского языка (ruMorph)
#
# Copyright (C) 2026
# Автор: Шкарин Сергей <kouki.sergey@gmail.com>
# URL: <https://github.com/SergeyShk/ruMorph>

from ._rumorph import Analysis, Analyzer, FormNotFoundError, __version__

# Тип описания словаря нужен только для аннотаций: модуль typing не грузится при импорте пакета
TYPE_CHECKING = False
if TYPE_CHECKING:
    from ._types import DictionaryInfo

__description__ = """Быстрый морфологический анализатор русского языка на Rust с API для Python.
Требует версию Python 3.11 и выше"""
__author__ = "Шкарин Сергей"
__author_email__ = "kouki.sergey@gmail.com"

__all__ = ["Analysis", "Analyzer", "DictionaryInfo", "FormNotFoundError", "__version__"]


def __getattr__(name: str) -> object:
    """
    Тип DictionaryInfo по первому обращению

    Аргументы:
        name (str): Имя атрибута пакета

    Вывод:
        object: Тип DictionaryInfo

    Исключения:
        AttributeError: Атрибута нет
    """
    if name == "DictionaryInfo":
        from ._types import DictionaryInfo

        return DictionaryInfo
    raise AttributeError(f"module 'rumorph' has no attribute {name!r}")
