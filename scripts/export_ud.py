"""
Выгрузка счетчиков разборов словоформ из трибанков Universal Dependencies для частот словаря

Описание:
    Скачивает обучающие части UD_Russian-Taiga и UD_Russian-GSD (CC BY-SA 4.0) закрепленного
    релиза UD, проверяет их контрольные суммы и пишет, сколько раз каждая словоформа (в нижнем
    регистре, «ё» как «е») размечена каждой парой «часть речи, лемма» всего, с заглавной буквы
    и с признаком Abbr=Yes (counts.tsv; лемма - в нижнем регистре с «ё»: «все» и «всё» - разные леммы; разметка -
    в соглашениях ruMorph, `conventions.py`), и описание
    источников (sources.json). Скачанные файлы хранятся в каталоге выгрузки и повторно не
    скачиваются.

Пример использования:
    uv run python scripts/export_ud.py target/ud
"""

import argparse
import json
from collections import Counter
from pathlib import Path

from conventions import harmonize, sentences
from export_opencorpora import download

RELEASE = "r2.19"
LICENSE = "CC BY-SA 4.0"
# Трибанк, коммит релиза и файлы с SHA-256
SOURCES = {
    "UD_Russian-Taiga": (
        "25ec1a29bedcb51b8ca48f31b26f2cd8818739db",
        {
            "ru_taiga-ud-train-a.conllu": "48205ec38c2d6c530415dc27f1277e37db296b8ed37b0acfbed7586ce7d0351f",
            "ru_taiga-ud-train-b.conllu": "e207b0518bbcb12f6325e99a726a5a46c2c98876c2dc4b0f0cf8d72791e06078",
            "ru_taiga-ud-train-c.conllu": "3858143d8b5330cca78c1a73031f89cb8e6b9181d521c60d36195f9eee113146",
            "ru_taiga-ud-train-d.conllu": "24f520ec5e374ff8ef5660ba83c37ee658c35e953c83a825f296225e88bb04a7",
            "ru_taiga-ud-train-e.conllu": "56f197a7c36f481ade530001d035e498819a628923d2b4a1f594aab274ccb311",
        },
    ),
    "UD_Russian-GSD": (
        "486a9c07204286d630944a905bdf854a6f471f20",
        {
            "ru_gsd-ud-train.conllu": "f434e52cdc24a154df0de850b86ea0fc829d51f11d33211fa09753d3e3a35584",
        },
    ),
}


def normal(text: str) -> str:
    """
    Словоформа в записи счетчиков: нижний регистр, «ё» как «е»

    Аргументы:
        text (str): Словоформа

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
    capitalized: Counter[tuple[str, str, str]] = Counter()
    abbreviated: Counter[tuple[str, str, str]] = Counter()
    for treebank, (commit, files) in SOURCES.items():
        for name, digest in files.items():
            path = raw / name
            url = f"https://raw.githubusercontent.com/UniversalDependencies/{treebank}/{commit}/{name}"
            download([url], path, digest)
            # Разметка - в соглашениях ruMorph (conventions.py); многословные токены и пустые
            # узлы - не словоформы
            for sentence in sentences(path):
                harmonize(treebank, sentence)
                for token in sentence:
                    key = normal(token.form), token.upos, token.lemma.lower()
                    counts[key] += 1
                    capitalized[key] += token.form[:1].isupper()
                    abbreviated[key] += token.feats.get("Abbr") == "Yes"
    with (output / "counts.tsv").open("w", encoding="utf-8") as file:
        for key, count in sorted(counts.items()):
            word, pos, lemma = key
            file.write(
                f"{word}\t{pos}\t{lemma}\t{count}\t{capitalized[key]}\t{abbreviated[key]}\n"
            )
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
