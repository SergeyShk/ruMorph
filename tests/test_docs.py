"""
У каждой русской страницы документации есть английская с теми же заголовками, якорями и ссылками;
блоки сигнатур страниц совпадают с кодом
"""

import inspect
import re
from pathlib import Path

import pytest

import rumorph

DOCS = Path(__file__).parents[1] / "docs"
PAGES = sorted(path for path in DOCS.rglob("*.md") if not path.name.endswith(".en.md"))
# Блок сигнатуры: ``` {.python .signature} с одним вызовом в одной строке
SIGNATURE = re.compile(r"^``` \{\.python \.signature\}\n(\w+)\((.*)\)\n```$", re.MULTILINE)
CLASSES = [
    value
    for name in rumorph.__all__
    if isinstance(value := getattr(rumorph, name), type) and not issubclass(value, BaseException)
]


def anchors(text: str) -> set[str]:
    return set(re.findall(r"\{ #([\w-]+) \}", text))


def links(text: str) -> set[str]:
    return set(re.findall(r"\]\(([^)#\s]+\.md)", text))


@pytest.mark.parametrize("page", PAGES, ids=[str(page.relative_to(DOCS)) for page in PAGES])
def test_english_page(page):
    english = page.with_name(page.name[:-3] + ".en.md")
    assert english.is_file(), f"нет английской версии {english.relative_to(DOCS)}"
    russian_text = page.read_text(encoding="utf-8")
    english_text = english.read_text(encoding="utf-8")
    assert anchors(english_text) == anchors(russian_text)
    assert links(english_text) == links(russian_text)
    assert russian_text.count("\n## ") == english_text.count("\n## ")
    assert SIGNATURE.findall(english_text) == SIGNATURE.findall(russian_text)


def test_no_orphan_english_pages():
    english = {path.relative_to(DOCS) for path in DOCS.rglob("*.en.md")}
    assert english == {page.relative_to(DOCS).with_suffix(".en.md") for page in PAGES}


def signature_text(target: object) -> str:
    """Параметры так, как их пишет блок сигнатуры: без self и разделителя позиционных"""
    parts = []
    keyword_only = False
    for param in inspect.signature(target).parameters.values():
        if param.name == "self":
            continue
        if param.kind is param.KEYWORD_ONLY and not keyword_only:
            parts.append("*")
            keyword_only = True
        if param.kind is param.VAR_POSITIONAL:
            parts.append(f"*{param.name}")
            keyword_only = True
        elif param.kind is param.VAR_KEYWORD:
            parts.append(f"**{param.name}")
        elif param.default is param.empty:
            parts.append(param.name)
        else:
            parts.append(f"{param.name}={param.default!r}")
    return ", ".join(parts)


def target(name: str) -> object:
    """Класс пакета или метод одного из его классов"""
    if name in rumorph.__all__:
        return getattr(rumorph, name)
    methods = [vars(cls)[name] for cls in CLASSES if name in vars(cls)]
    assert len(methods) == 1, f"{name}: метод не найден или неоднозначен"
    return methods[0]


def test_signatures_match_the_code():
    found = set()
    for page in PAGES:
        for name, written in SIGNATURE.findall(page.read_text(encoding="utf-8")):
            assert written == signature_text(target(name)), (
                f"{page.relative_to(DOCS)}: {name}({written}) расходится с кодом"
            )
            found.add(name)
    # Сигнатура нужна классу с конструктором и каждому публичному методу
    required = {cls.__name__ for cls in CLASSES if "__new__" in vars(cls)} | {
        name
        for cls in CLASSES
        for name, value in vars(cls).items()
        if callable(value) and not name.startswith("_")
    }
    assert found >= required, f"нет блоков сигнатур: {sorted(required - found)}"
