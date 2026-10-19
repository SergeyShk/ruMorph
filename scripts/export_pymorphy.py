"""
Выгрузка словаря pymorphy3-dicts-ru в текстовые таблицы для сборщика rumorph-build

Описание:
    Словарь OpenCorpora в pymorphy3-dicts-ru хранится в DAWG, который читает только
    pymorphy3. Скрипт выгружает таблицы тегов, префиксов, суффиксов и парадигм (JSON),
    словоформы с номерами парадигм и форм (words.tsv) и P(тег | словоформа)
    в миллионных долях (probabilities.tsv).

Пример использования:
    uv run python scripts/export_pymorphy.py build/opencorpora
"""

import argparse
import importlib.metadata
import json
from itertools import groupby
from pathlib import Path

import pymorphy3


def export(output: Path) -> None:
    """
    Выгрузка словаря pymorphy3-dicts-ru в каталог

    Аргументы:
        output (Path): Каталог выгрузки
    """
    output.mkdir(parents=True, exist_ok=True)
    morph = pymorphy3.MorphAnalyzer()
    dictionary = morph.dictionary

    tags = [str(tag) for tag in dictionary.gramtab]
    meta = dict(dictionary.meta)
    info = {
        "source": "pymorphy3-dicts-ru",
        "source_version": importlib.metadata.version("pymorphy3-dicts-ru"),
        "opencorpora_revision": meta["source_revision"],
        "license": "CC BY-SA 3.0",
    }
    tables = {
        "meta.json": info,
        "tags.json": tags,
        "prefixes.json": list(dictionary.paradigm_prefixes),
        "suffixes.json": list(dictionary.suffixes),
        "paradigms.json": [list(paradigm) for paradigm in dictionary.paradigms],
    }
    for name, table in tables.items():
        (output / name).write_text(json.dumps(table, ensure_ascii=False), encoding="utf-8")

    with (output / "words.tsv").open("w", encoding="utf-8") as file:
        for word, items in groupby(dictionary.words.iteritems(), key=lambda item: item[0]):
            records = " ".join(f"{paradigm}:{form}" for _, (paradigm, form) in items)
            file.write(f"{word}\t{records}\n")

    # Частоты с тегами, которых нет в таблице тегов, не могут совпасть ни с одним разбором
    numbers = {tag: number for number, tag in enumerate(tags)}
    with (output / "probabilities.tsv").open("w", encoding="utf-8") as file:
        for key, millionths in morph.prob_estimator.p_t_given_w.iteritems():
            word, tag = key.split(":", 1)
            if tag in numbers:
                file.write(f"{word}\t{numbers[tag]}\t{millionths}\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.strip().splitlines()[0])
    parser.add_argument("output", type=Path, help="каталог выгрузки")
    export(parser.parse_args().output)
