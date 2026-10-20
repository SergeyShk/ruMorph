<h1 align="center">ruMorph</h1>

<p align="center">
  Быстрый морфологический анализатор русского языка на Rust с API для Python
</p>

<p align="center">
  <a href="https://sergeyshk.github.io/ruMorph/">Документация</a> ·
  <a href="https://github.com/SergeyShk/ruMorph/blob/master/README.en.md">English</a>
</p>

<p align="center">
  <a href="https://github.com/SergeyShk/ruMorph/actions/workflows/ci.yml"><img src="https://github.com/SergeyShk/ruMorph/actions/workflows/ci.yml/badge.svg" alt="Сборка"></a>
  <a href="https://github.com/astral-sh/ruff"><img src="https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json" alt="Ruff"></a>
  <a href="https://github.com/SergeyShk/ruMorph/blob/master/LICENSE.txt"><img src="https://img.shields.io/github/license/sergeyshk/rumorph.svg" alt="Лицензия"></a>
</p>

---

**ruMorph** - морфологический анализатор русского языка: ядро на Rust и API для Python. Для словоформы анализатор находит по словарю OpenCorpora варианты разбора - начальную форму (лемму), часть речи и грамматические признаки - и упорядочивает их по вероятности.

> [!NOTE]
> Ранняя версия: до 1.0 API может меняться между версиями 0.x, изменения описываются в заметках к выпускам.

## Установка

Требуется Python 3.11 или новее.

```bash
pip install rumorph
```

Колеса собираются для Linux, macOS и Windows; на других платформах для сборки из исходников нужен [Rust](https://www.rust-lang.org/tools/install). Словарь входит в пакет. Подробнее - на странице [Установка](https://sergeyshk.github.io/ruMorph/installation/).

## Быстрый старт

```python
>>> from rumorph import Analyzer

>>> morph = Analyzer()
>>> morph.lemmatize(["Мы", "стали", "друзьями"])
['мы', 'стать', 'друг']
>>> [(a.lemma, a.pos, a.feats.get("Case"), round(a.prob, 3)) for a in morph.analyze("стали")][:2]
[('стать', 'VERB', None, 0.975), ('сталь', 'NOUN', 'Gen', 0.011)]
>>> cat = morph.analyze("кошка")[0]
>>> cat.inflect(Case="Gen", Number="Plur"), morph.analyze("рубль")[0].agree(5)
('кошек', 'рублей')
```

Подробнее - на страницах [Анализатор](https://sergeyshk.github.io/ruMorph/analyzer/), [Разбор словоформы](https://sergeyshk.github.io/ruMorph/analysis/) и [Словоизменение](https://sergeyshk.github.io/ruMorph/inflection/). Как перейти с pymorphy2 и pymorphy3 - на странице [Переход с pymorphy](https://sergeyshk.github.io/ruMorph/migration/), сравнение скорости и памяти - на странице [Скорость и память](https://sergeyshk.github.io/ruMorph/performance/).

## Разработка

Проект использует [uv](https://docs.astral.sh/uv/) для управления зависимостями, [maturin](https://www.maturin.rs/) для сборки модуля на Rust и [ruff](https://docs.astral.sh/ruff/) для линтинга и форматирования. Нужен [Rust](https://www.rust-lang.org/tools/install) 1.88 или новее.

```bash
git clone https://github.com/SergeyShk/ruMorph.git
cd ruMorph

make deps        # создать окружение, установить зависимости и собрать модуль на Rust
make dictionary  # собрать словарь из pymorphy3-dicts-ru
make test        # запустить тесты Python, примеры из докстрингов (doctest) и тесты Rust
make lint        # ruff, mypy, rustfmt и clippy
```

Полный список команд - `make help`. Документация двуязычная: русские страницы `docs/*.md`, английские - `docs/*.en.md` рядом с ними; при правке страницы обновляйте обе версии.

Перед отправкой изменений стоит установить хуки, которые прогонят линтеры на коммите и тесты на пуше:

```bash
uv run pre-commit install
```

## Участие в проекте

Баг-репорты, идеи и пул-реквесты приветствуются - [issues](https://github.com/SergeyShk/ruMorph/issues) открыты. Как устроен процесс и что проверить перед отправкой пул-реквеста - в [CONTRIBUTING.md](https://github.com/SergeyShk/ruMorph/blob/master/CONTRIBUTING.md); правила общения - в [кодексе поведения](https://github.com/SergeyShk/ruMorph/blob/master/CODE_OF_CONDUCT.md).

<details>
<summary><b>Структура проекта</b></summary>

<br>

*   **crates/rumorph** - ядро анализатора на Rust: формат словаря, сборка, разбор и словоизменение
*   **crates/rumorph-build** - сборка словаря из выгрузки pymorphy3-dicts-ru и перевод тегов OpenCorpora в UD
*   **crates/rumorph-py** - модуль Python `rumorph._rumorph` на PyO3
*   **python/rumorph** - пакет Python
*   **scripts** - выгрузка словаря pymorphy3-dicts-ru
*   **docs** - документация MkDocs на русском и английском
*   **tests** - тесты пакета Python

</details>

## Авторы

*   Шкарин Сергей (kouki.sergey@gmail.com)

## Лицензия

Код распространяется под лицензией [MIT](https://github.com/SergeyShk/ruMorph/blob/master/LICENSE.txt). Словарь в пакете собран из словаря [OpenCorpora](https://opencorpora.org) и распространяется по лицензии CC BY-SA 3.0 ([LICENSE-DICTIONARY.txt](https://github.com/SergeyShk/ruMorph/blob/master/LICENSE-DICTIONARY.txt)).
