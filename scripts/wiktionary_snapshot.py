"""
Снимок русского Викисловаря для сборки словаря ruMorph

Описание:
    Читает выгрузку ru.wiktionary.org, которую делает wiktextract (kaikki.org, файл
    raw-wiktextract-data.jsonl.gz), и пишет русские статьи с таблицами форм в JSON Lines,
    сжатый xz: заголовок, часть речи, теги и категории статьи, формы с тегами и пометами,
    начала значений (по ним видны пометы «устар.», «рег.»). Снимок выкладывается в датасет
    на Hugging Face: kaikki.org хранит только последнюю выгрузку. Данные Викисловаря - под
    лицензией CC BY-SA 4.0.

Пример использования:
    python scripts/wiktionary_snapshot.py raw-wiktextract-data.jsonl.gz ru-wiktionary.jsonl.xz
"""

import argparse
import gzip
import json
import lzma
from pathlib import Path

# Начала значений - только для помет в начале
GLOSS = 60


def snapshot(source: Path, output: Path) -> int:
    """
    Снимок русских статей с таблицами форм

    Аргументы:
        source (Path): Выгрузка wiktextract, сжатая gzip
        output (Path): Файл снимка

    Вывод:
        int: Число статей в снимке
    """
    count = 0
    with (
        gzip.open(source, "rt", encoding="utf-8") as lines,
        lzma.open(output, "wt", encoding="utf-8", preset=9) as out,
    ):
        for line in lines:
            if '"lang_code": "ru"' not in line:
                continue
            entry = json.loads(line)
            forms = [
                [form["form"], form.get("tags", []), form.get("raw_tags", [])]
                for form in entry.get("forms", [])
                if form.get("form") and form.get("tags")
            ]
            if entry.get("lang_code") != "ru" or not forms:
                continue
            record = {
                "word": entry["word"],
                "pos": entry.get("pos", ""),
                "tags": entry.get("tags", []),
                "categories": [
                    category if isinstance(category, str) else category.get("name", "")
                    for category in entry.get("categories", [])
                ],
                "forms": forms,
                "glosses": [
                    (sense.get("glosses") or [""])[0][:GLOSS] for sense in entry.get("senses", [])
                ],
            }
            out.write(json.dumps(record, ensure_ascii=False) + "\n")
            count += 1
    return count


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.strip().splitlines()[0])
    parser.add_argument(
        "source", type=Path, help="выгрузка wiktextract (raw-wiktextract-data.jsonl.gz)"
    )
    parser.add_argument("output", type=Path, help="файл снимка (.jsonl.xz)")
    args = parser.parse_args()
    print(snapshot(args.source, args.output))
