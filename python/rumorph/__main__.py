# Морфологический анализатор русского языка (ruMorph)
#
# Copyright (C) 2026
# Автор: Шкарин Сергей <kouki.sergey@gmail.com>
# URL: <https://github.com/SergeyShk/ruMorph>

"""
Команда rumorph: морфологический анализ текста из файлов и стандартного ввода

Пример использования:
    rumorph --format conllu text.txt
    python -m rumorph --help
"""

import signal
import sys

from ._rumorph import _main


def main() -> None:
    """
    Точка входа команды rumorph

    Описание:
        Аргументы разбирает, файлы читает и результат пишет ядро на Rust; Ctrl+C завершает
        команду сразу, как обычную программу, а игнорируемый SIGINT (фоновая команда сценария)
        остается игнорируемым

    Исключения:
        SystemExit: Всегда, с кодом выхода команды
    """
    if signal.getsignal(signal.SIGINT) is signal.default_int_handler:
        signal.signal(signal.SIGINT, signal.SIG_DFL)
    sys.exit(_main(sys.argv[1:]))


if __name__ == "__main__":
    main()
