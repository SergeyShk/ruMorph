"""
Выгрузка счетчиков разборов словоформ из трибанков Universal Dependencies для частот словаря

Описание:
    Скачивает обучающие части UD_Russian-Taiga и UD_Russian-GSD (CC BY-SA 4.0) закрепленного
    релиза UD, проверяет их контрольные суммы и пишет, сколько раз каждая словоформа (в нижнем
    регистре, «ё» как «е») размечена каждой парой «часть речи, лемма» (counts.tsv), и описание
    источников (sources.json). Скачанные файлы хранятся в каталоге выгрузки и повторно не
    скачиваются.

Пример использования:
    uv run python scripts/export_ud.py target/ud
"""

import argparse
import gzip
import hashlib
import json
import urllib.request
from collections import Counter
from pathlib import Path

RELEASE = "r2.18"
LICENSE = "CC BY-SA 4.0"
# Трибанк, коммит релиза и файлы с SHA-256
SOURCES = {
    "UD_Russian-Taiga": (
        "d1e48cd29c1f19b6aad1aec361d9f724db764f66",
        {
            "ru_taiga-ud-train-a.conllu": "d12f8d99e1370fad142fdbd5aea387715249bc34bf5494c5d6dd63c795dfa85e",
            "ru_taiga-ud-train-b.conllu": "82c0b23692ff0ebcdb7e53d40d74396b75dc92a076e4c0a014c1f32e10cb14ac",
            "ru_taiga-ud-train-c.conllu": "9a6301fd797395b2b7c5fbc25880f445248ba04571e4cc691e38a2fd3130e738",
            "ru_taiga-ud-train-d.conllu": "6bc5c4ce2e7389f6956373c8e565ba63a3501206988dbf6b64dcb1305be79021",
            "ru_taiga-ud-train-e.conllu": "07bab379f51eafb3aedc46fd77a1a96a01d3d8931186387a373d8a6c2689b02d",
        },
    ),
    "UD_Russian-GSD": (
        "9acc9d677327043bd416fcc89e4b3407c620d885",
        {
            "ru_gsd-ud-train.conllu": "0ae33a4bbda12697e2eb0a32af4971bda73af5852c8521744a974a316c02bab5",
        },
    ),
}


def download(url: str, path: Path, digest: str) -> None:
    """
    Скачивание файла с проверкой контрольной суммы

    Аргументы:
        url (str): Адрес файла
        path (Path): Куда сохранить
        digest (str): SHA-256 файла

    Исключения:
        ValueError: Если контрольная сумма не совпадает
    """
    if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() == digest:
        return
    request = urllib.request.Request(url, headers={"Accept-Encoding": "gzip"})
    with urllib.request.urlopen(request, timeout=300) as response:
        data = response.read()
        if response.headers.get("Content-Encoding") == "gzip":
            data = gzip.decompress(data)
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError(f"{url}: контрольная сумма не совпадает")
    path.write_bytes(data)


def normal(text: str) -> str:
    """
    Словоформа или лемма в записи счетчиков: нижний регистр, «ё» как «е»

    Аргументы:
        text (str): Словоформа или лемма

    Вывод:
        str: Запись для счетчиков
    """
    return text.lower().replace("ё", "е")


def export(output: Path) -> None:
    """
    Выгрузка счетчиков разборов в каталог

    Аргументы:
        output (Path): Каталог выгрузки
    """
    raw = output / "raw"
    raw.mkdir(parents=True, exist_ok=True)
    counts: Counter[tuple[str, str, str]] = Counter()
    for treebank, (commit, files) in SOURCES.items():
        for name, digest in files.items():
            path = raw / name
            url = f"https://raw.githubusercontent.com/UniversalDependencies/{treebank}/{commit}/{name}"
            download(url, path, digest)
            with path.open(encoding="utf-8") as file:
                for line in file:
                    if not line[0].isdigit():
                        continue
                    columns = line.split("\t")
                    # Многословные токены и пустые узлы - не словоформы
                    if "-" in columns[0] or "." in columns[0]:
                        continue
                    counts[normal(columns[1]), columns[3], normal(columns[2])] += 1
    with (output / "counts.tsv").open("w", encoding="utf-8") as file:
        for (word, pos, lemma), count in sorted(counts.items()):
            file.write(f"{word}\t{pos}\t{lemma}\t{count}\n")
    sources = {
        "release": RELEASE,
        "treebanks": list(SOURCES),
        "license": LICENSE,
    }
    (output / "sources.json").write_text(json.dumps(sources, ensure_ascii=False), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.strip().splitlines()[0])
    parser.add_argument("output", type=Path, help="каталог выгрузки")
    export(parser.parse_args().output)
