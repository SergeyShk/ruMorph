"""
Лицензии крейтов Rust, которые собраны в модуль Python: THIRD-PARTY-NOTICES.txt

Описание:
    Крейты - обычные зависимости `rumorph-py` на любой платформе, без процедурных макросов
    и своих крейтов рабочего пространства. У каждого крейта на выбор несколько лицензий: в файл
    идет его лицензия MIT (у крейта только с одной лицензией - она) и лицензия данных Unicode.
    После изменения Cargo.lock файл пишется заново; совпадение проверяет
    `tests/test_package.py::test_third_party_notices`.

Пример использования:
    uv run python scripts/notices.py
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parents[1]
HEADER = [
    "Licenses of the Rust crates linked into the Python module of ruMorph. Where a crate offers a",
    "choice of licenses, the MIT license is the one used here, with the Unicode license of the Unicode",
    "data.",
    "",
]
# Файлы лицензии MIT и данных Unicode; у крейта без них - его единственная лицензия
TEXTS = ("LICENSE-MIT", "LICENSE-UNICODE", "COPYRIGHT")
SINGLE = ("LICENSE", "LICENSE.txt", "LICENSE.md")


def linked(metadata: dict) -> list[dict]:
    """
    Крейты crates.io, которые модуль собирает на какой-либо платформе, по имени

    Аргументы:
        metadata (dict): Вывод `cargo metadata`

    Вывод:
        list[dict]: Пакеты крейтов
    """
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = next(
        package["id"] for package in metadata["packages"] if package["name"] == "rumorph-py"
    )
    seen, todo = set(), [root]
    while todo:
        for dependency in nodes[todo.pop()]["deps"]:
            package = packages[dependency["pkg"]]
            normal = any(kind["kind"] is None for kind in dependency["dep_kinds"])
            macro = any("proc-macro" in target["kind"] for target in package["targets"])
            if normal and not macro and package["id"] not in seen:
                seen.add(package["id"])
                todo.append(package["id"])
    crates = [packages[package] for package in seen if packages[package]["source"] is not None]
    return sorted(crates, key=lambda package: package["name"])


def notices() -> str:
    """
    Текст файла

    Вывод:
        str: Заголовок и лицензии крейтов

    Исключения:
        ValueError: Если у крейта не нашлось текста лицензии
    """
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked"],
            cwd=ROOT,
            capture_output=True,
            check=True,
        ).stdout
    )
    lines = list(HEADER)
    for package in linked(metadata):
        folder = Path(package["manifest_path"]).parent
        files = sorted(folder.iterdir())
        texts = [path for path in files if path.name.upper().startswith(TEXTS)]
        texts = texts or [path for path in files if path.name in SINGLE]
        if not texts:
            raise ValueError(f"{package['name']}: нет текста лицензии в {folder}")
        rule = "=" * 100
        title = f"{package['name']} {package['version']} ({package['license']})"
        lines += [rule, f"{title}: {package.get('repository') or ''}", rule, ""]
        for path in texts:
            lines += [path.read_text(encoding="utf-8").strip(), ""]
    text = "\n".join(line.rstrip() for line in "\n".join(lines).splitlines())
    return text.rstrip() + "\n"


if __name__ == "__main__":
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "THIRD-PARTY-NOTICES.txt"
    # LF на любой системе, как в файле репозитория
    path.write_text(notices(), encoding="utf-8", newline="\n")
