# Морфологический анализатор русского языка (ruMorph)
#
# Copyright (C) 2026
# Автор: Шкарин Сергей <kouki.sergey@gmail.com>
# URL: <https://github.com/SergeyShk/ruMorph>

from importlib.metadata import PackageNotFoundError, version

from ._rumorph import Analysis, Analyzer, FormNotFoundError

try:
    __version__ = version("rumorph")
except PackageNotFoundError:
    __version__ = "0.0.0"
__description__ = """Быстрый морфологический анализатор русского языка на Rust с API для Python.
Требует версию Python 3.11 и выше"""
__author__ = "Шкарин Сергей"
__author_email__ = "kouki.sergey@gmail.com"

__all__ = ["Analysis", "Analyzer", "FormNotFoundError", "__version__"]
