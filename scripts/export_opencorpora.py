"""
Выгрузка словаря и корпуса OpenCorpora в текстовые таблицы для сборщика rumorph-build

Описание:
    Словарь и корпус OpenCorpora - снимки Wayback Machine (opencorpora.org не отвечает)
    с проверкой SHA-256. Лексемы объединяются по связям, у набора граммем остается одна
    запись тега, формы лексемы делятся на основу и парадигму. Скрипт пишет таблицы
    тегов, префиксов, суффиксов и парадигм (JSON), словоформы с номерами парадигм и форм
    (words.tsv) и P(тег | словоформа) по разметке корпуса в миллионных долях
    (probabilities.tsv). Нужна только стандартная библиотека.

Пример использования:
    uv run python scripts/export_opencorpora.py target/opencorpora
"""

import argparse
import base64
import bz2
import hashlib
import http.client
import json
import struct
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET
from collections import Counter, defaultdict
from collections.abc import Iterator
from pathlib import Path

LICENSE = "CC BY-SA 3.0"
# Файл, адреса (снимок Wayback Machine, затем сайт) и SHA-256; ревизии 417258 и 4580223.
# Корпус - и данные обучения контекстной модели (train_context.py)
DICTIONARY = (
    "dict.opcorpora.xml.bz2",
    [
        "https://web.archive.org/web/20240423122538id_/https://www.opencorpora.org/files/export/dict/dict.opcorpora.xml.bz2",
        "https://opencorpora.org/files/export/dict/dict.opcorpora.xml.bz2",
    ],
    "f27150955c5f0978ece14348096f7d1380fc3564b08fdf99f129ede586309b6c",
)
CORPUS = (
    "annot.opcorpora.xml.bz2",
    [
        "https://web.archive.org/web/20240624222734id_/http://opencorpora.org/files/export/annot/annot.opcorpora.xml.bz2",
        "https://opencorpora.org/files/export/annot/annot.opcorpora.xml.bz2",
    ],
    "dff2b07a3db865eb234515c3c2c51e370fc67b3582f32af7174625c8f2be4422",
)
# Префиксы парадигм: превосходная степень («наибольший») и «по» сравнительной («побольше»)
PARADIGM_PREFIXES = ["", "по", "наи"]
# Связи, по которым лексемы не объединяются: имя - отчество, полная и краткая запись,
# количественное и порядковое числительное, порядковое словом и цифрами
SEPARATE_LINKS = {"7", "21", "23", "27"}
# Одиночная заглавная буква в корпусе может быть инициалом: такой токен неоднозначен при любом словаре
INITIALS = set("АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЭЮЯ")
# P(тег | словоформа) - в миллионных долях
MILLION = 1000000
# Попыток на адрес, пауза перед повтором (растет с номером попытки) и таймаут операции сокета, с;
# Wayback Machine временами не отвечает на рукопожатие или обрывает ответ
ATTEMPTS = 3
PAUSE = 15
TIMEOUT = 60

Lexeme = list[tuple[str, str]]


def download(urls: list[str], path: Path, digest: str) -> Path:
    """
    Скачивание файла с проверкой контрольной суммы: по первому адресу, с которого пришел файл
    с нужной суммой; при сбое сети или ответе 5xx - повтор с паузой

    Аргументы:
        urls (list[str]): Адреса файла
        path (Path): Куда сохранить
        digest (str): SHA-256 файла

    Вывод:
        Path: Путь к файлу

    Исключения:
        ValueError: Если ни один адрес не дал файл с нужной суммой
    """
    if path.exists() and sha256(path) == digest:
        return path
    errors = []
    for url in urls:
        for attempt in range(ATTEMPTS):
            time.sleep(PAUSE * attempt)
            try:
                with urllib.request.urlopen(url, timeout=TIMEOUT) as response:
                    data = response.read()
            except urllib.error.HTTPError as err:
                errors.append(f"{url}: {err}")
                if err.code < 500:
                    break
                continue
            except (OSError, http.client.HTTPException) as err:
                errors.append(f"{url}: {err!r}")
                continue
            if hashlib.sha256(data).hexdigest() != digest:
                errors.append(f"{url}: контрольная сумма не совпадает")
                break
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            return path
    raise ValueError("; ".join(errors))


def sha256(path: Path) -> str:
    """
    SHA-256 файла

    Аргументы:
        path (Path): Путь к файлу

    Вывод:
        str: Контрольная сумма
    """
    digest = hashlib.sha256()
    with path.open("rb") as file:
        while chunk := file.read(1 << 20):
            digest.update(chunk)
    return digest.hexdigest()


def parse_dictionary(path: Path) -> tuple[str, dict[str, Lexeme], list[tuple[str, str, str]]]:
    """
    Чтение словаря OpenCorpora: ревизия, лексемы по номерам и связи

    Аргументы:
        path (Path): Сжатый XML словаря

    Вывод:
        tuple[str, dict[str, Lexeme], list[tuple[str, str, str]]]: Ревизия; формы и теги лексем;
            связи - от какой лексемы, к какой и тип

    Исключения:
        ValueError: Если в файле нет ревизии словаря
    """
    revision = None
    lexemes: dict[str, Lexeme] = {}
    links = []
    with bz2.open(path) as file:
        for event, elem in ET.iterparse(file, events=("start", "end")):
            if event == "start":
                if elem.tag == "dictionary":
                    revision = elem.get("revision")
                continue
            if elem.tag == "lemma":
                lexemes[elem.get("id", "")] = lexeme_forms(elem)
                elem.clear()
            elif elem.tag == "link":
                links.append((elem.get("from", ""), elem.get("to", ""), elem.get("type", "")))
                elem.clear()
    if revision is None:
        raise ValueError(f"{path}: нет ревизии словаря")
    return revision, lexemes, links


def lexeme_forms(elem: ET.Element) -> Lexeme:
    """
    Формы и теги лексемы: граммемы леммы, через пробел - граммемы формы

    Аргументы:
        elem (ET.Element): Элемент lemma

    Вывод:
        Lexeme: Формы в нижнем регистре и теги; пусто, если лексема удалена или у формы нет граммем
    """
    if len(elem) == 0:
        return []
    lemma = elem.find("l")
    base = "" if lemma is None else ",".join(g.get("v", "") for g in lemma.iter("g"))
    lexeme = []
    for form in elem.iter("f"):
        grammemes = ",".join(g.get("v", "") for g in form.iter("g"))
        if not base + grammemes:
            return []
        lexeme.append((form.get("t", "").lower(), f"{base} {grammemes}".strip()))
    return lexeme


def simplify_tags(lexemes: dict[str, Lexeme]) -> None:
    """
    Одна запись тега на набор граммем: loc1, gen1, acc1 - основные падежи, у записей
    с тем же местом пробела - самая частая

    Аргументы:
        lexemes (dict[str, Lexeme]): Лексемы, меняются на месте
    """
    spellings: defaultdict[frozenset[str], Counter[str]] = defaultdict(Counter)
    for lexeme in lexemes.values():
        for _, tag in lexeme:
            spellings[grammemes(tag)][tag] += 1
    replaces = {}
    for tags in spellings.values():
        spaces = [tag.index(" ") if " " in tag else None for tag in tags]
        if len(tags) < 2 or len(spaces) == len(set(spaces)):
            continue
        # Порядок записей с равным числом - порядок первого появления
        ranked = sorted(tags.items(), key=lambda item: item[1], reverse=True)
        for tag, _ in ranked[1:]:
            replaces[tag] = ranked[0][0]
    for number, lexeme in lexemes.items():
        lexemes[number] = [(form, simplify(tag, replaces)) for form, tag in lexeme]


def simplify(tag: str, replaces: dict[str, str]) -> str:
    """
    Запись тега после упрощения

    Аргументы:
        tag (str): Тег OpenCorpora
        replaces (dict[str, str]): Замены записей

    Вывод:
        str: Тег
    """
    tag = redundant(tag)
    return replaces.get(tag, tag)


def redundant(tag: str) -> str:
    """
    Тег с основными падежами вместо loc1, gen1 и acc1

    Аргументы:
        tag (str): Тег OpenCorpora

    Вывод:
        str: Тег
    """
    return tag.replace("loc1", "loct").replace("gen1", "gent").replace("acc1", "accs")


def grammemes(tag: str) -> frozenset[str]:
    """
    Набор граммем тега

    Аргументы:
        tag (str): Тег OpenCorpora

    Вывод:
        frozenset[str]: Граммемы
    """
    return frozenset(redundant(tag).replace(" ", ",", 1).split(","))


def join_lexemes(lexemes: dict[str, Lexeme], links: list[tuple[str, str, str]]) -> list[Lexeme]:
    """
    Объединение связанных лексем: формы лексемы, на которую указывает связь, переходят к той,
    от которой она идет

    Аргументы:
        lexemes (dict[str, Lexeme]): Лексемы по номерам
        links (list[tuple[str, str, str]]): Связи

    Вывод:
        list[Lexeme]: Непустые лексемы по возрастанию номера
    """
    moves: dict[str, str] = {}
    for start, end, kind in links:
        if kind in SEPARATE_LINKS:
            continue
        target = start
        while target in moves:
            target = moves[target]
        lexemes[target].extend(lexemes[end])
        lexemes[end] = []
        moves[end] = target
    return [lexemes[number] for number in sorted(lexemes, key=int) if lexemes[number]]


def common_substring(forms: tuple[str, ...]) -> str:
    """
    Самая длинная общая подстрока форм, из равных - ближайшая к началу первой формы

    Аргументы:
        forms (tuple[str, ...]): Формы лексемы

    Вывод:
        str: Подстрока
    """
    first = forms[0]
    for length in range(len(first), 0, -1):
        for start in range(len(first) - length + 1):
            candidate = first[start : start + length]
            if all(candidate in form for form in forms):
                return candidate
    return ""


def to_paradigm(lexeme: Lexeme) -> tuple[str, tuple[tuple[str, str, str], ...]]:
    """
    Основа и парадигма лексемы: суффикс, тег и префикс каждой формы

    Аргументы:
        lexeme (Lexeme): Формы и теги

    Вывод:
        tuple[str, tuple[tuple[str, str, str], ...]]: Основа и парадигма
    """
    forms, tags = zip(*lexeme, strict=True)
    if len(forms) == 1:
        stem, prefixes = forms[0], [""]
    else:
        stem = common_substring(forms)
        prefixes = [form[: form.index(stem)] for form in forms]
        # Приставка не из PARADIGM_PREFIXES: основа пустая, формы целиком - суффиксы
        if any(prefix not in PARADIGM_PREFIXES for prefix in prefixes):
            stem, prefixes = "", [""] * len(forms)
    suffixes = [
        form[len(prefix) + len(stem) :] for form, prefix in zip(forms, prefixes, strict=True)
    ]
    return stem, tuple(zip(suffixes, tags, prefixes, strict=True))


def compile_dictionary(
    lexemes: list[Lexeme],
) -> tuple[list[str], list[str], list[list[int]], list[tuple[str, int, int]]]:
    """
    Таблицы словаря: теги и парадигмы по порядку появления, суффиксы по алфавиту, словоформы

    Аргументы:
        lexemes (list[Lexeme]): Объединенные лексемы

    Вывод:
        tuple[list[str], list[str], list[list[int]], list[tuple[str, int, int]]]: Теги, суффиксы,
            парадигмы (номера суффиксов, тегов и префиксов форм тремя блоками), словоформы
            с номерами парадигмы и формы
    """
    tags: list[str] = []
    tag_numbers: dict[str, int] = {}
    paradigms: list[tuple[tuple[str, str, str], ...]] = []
    paradigm_numbers: dict[tuple[tuple[str, str, str], ...], int] = {}
    words = []
    for lexeme in lexemes:
        stem, paradigm = to_paradigm(lexeme)
        for _, tag, _ in paradigm:
            if tag not in tag_numbers:
                tag_numbers[tag] = len(tags)
                tags.append(tag)
        number = paradigm_numbers.setdefault(paradigm, len(paradigms))
        if number == len(paradigms):
            paradigms.append(paradigm)
        for form, (suffix, _, prefix) in enumerate(paradigm):
            words.append((prefix + stem + suffix, number, form))
    suffixes = sorted({suffix for paradigm in paradigms for suffix, _, _ in paradigm})
    suffix_numbers = {suffix: number for number, suffix in enumerate(suffixes)}
    prefix_numbers = {prefix: number for number, prefix in enumerate(PARADIGM_PREFIXES)}
    layouts = [
        [suffix_numbers[suffix] for suffix, _, _ in paradigm]
        + [tag_numbers[tag] for _, tag, _ in paradigm]
        + [prefix_numbers[prefix] for _, _, prefix in paradigm]
        for paradigm in paradigms
    ]
    return tags, suffixes, layouts, words


def record_key(word: str, paradigm: int, form: int) -> bytes:
    """
    Ключ порядка словоформ: словоформа, затем номера парадигмы и формы в base64

    Аргументы:
        word (str): Словоформа
        paradigm (int): Номер парадигмы
        form (int): Номер формы

    Вывод:
        bytes: Ключ сортировки
    """
    return word.encode() + b"\x01" + base64.b64encode(struct.pack(">HH", paradigm, form))


def corpus_words(path: Path) -> Iterator[tuple[str, list[str]]]:
    """
    Токены корпуса OpenCorpora с тегами их разборов

    Аргументы:
        path (Path): Сжатый XML корпуса

    Вывод:
        Iterator[tuple[str, list[str]]]: Текст токена и теги вариантов разбора
    """
    with bz2.open(path) as file:
        for _, elem in ET.iterparse(file):
            if elem.tag == "token":
                tags = [",".join(g.get("v", "") for g in lemma) for lemma in elem.iter("l")]
                yield elem.get("text", ""), tags
                elem.clear()
            elif elem.tag == "sentence":
                elem.clear()


def probabilities(
    path: Path, tags: list[str], layouts: list[list[int]], words: list[tuple[str, int, int]]
) -> list[tuple[str, int, int]]:
    """
    P(тег | словоформа) по токенам корпуса с одним разбором: у неоднозначных по словарю
    словоформ, сглаживание Лапласа; словоформы с равными долями всех тегов не пишутся

    Аргументы:
        path (Path): Сжатый XML корпуса
        tags (list[str]): Теги словаря
        layouts (list[list[int]]): Парадигмы
        words (list[tuple[str, int, int]]): Словоформы с номерами парадигмы и формы

    Вывод:
        list[tuple[str, int, int]]: Словоформа, номер тега и доля в миллионных
    """
    # Теги записей словоформы с учетом «ё»: поиск «е» находит и «ё», поиск «ё» - только «ё».
    # Число записей, а не разных тегов - мера неоднозначности и число исходов сглаживания
    by_key: defaultdict[str, defaultdict[str, list[int]]] = defaultdict(lambda: defaultdict(list))
    for word, paradigm, form in words:
        layout = layouts[paradigm]
        by_key[word.replace("ё", "е")][word].append(layout[len(layout) // 3 + form])

    def word_tags(word: str) -> list[int]:
        found: list[int] = []
        for spelling, spelling_tags in by_key.get(word.replace("ё", "е"), {}).items():
            if all(a == b or (a, b) == ("е", "ё") for a, b in zip(word, spelling, strict=True)):
                found.extend(spelling_tags)
        return found

    counts: defaultdict[str, Counter[frozenset[str]]] = defaultdict(Counter)
    for text, variants in corpus_words(path):
        if len(variants) != 1:
            continue
        word = text.lower()
        found = word_tags(word)
        if found and (len(found) > 1 or text in INITIALS):
            observed = grammemes(variants[0])
            if observed != {"UNKN"}:
                counts[word][observed] += 1

    result = []
    for word, observed in counts.items():
        found = word_tags(word)
        total, bins = sum(observed.values()), max(len(found), len(observed))
        shares = {
            tag: int((observed[grammemes(tags[tag])] + 1.0) / (total + bins * 1.0) * MILLION)
            for tag in dict.fromkeys(found)
        }
        if len(set(shares.values())) > 1:
            result.extend((word, tag, share) for tag, share in shares.items())
    result.sort()
    return result


def export(output: Path) -> None:
    """
    Выгрузка словаря и корпуса OpenCorpora в каталог

    Аргументы:
        output (Path): Каталог выгрузки, скачанные файлы - в его подкаталоге raw
    """
    output.mkdir(parents=True, exist_ok=True)
    name, urls, digest = DICTIONARY
    dictionary = download(urls, output / "raw" / name, digest)
    name, urls, digest = CORPUS
    corpus = download(urls, output / "raw" / name, digest)

    revision, lexemes, links = parse_dictionary(dictionary)
    simplify_tags(lexemes)
    for number, lexeme in lexemes.items():
        lexemes[number] = [(form, tag) for form, tag in lexeme if "Init" not in tag]
    joined = join_lexemes(lexemes, links)
    tags, suffixes, layouts, words = compile_dictionary(joined)
    corpus_revision = next(corpus_revisions(corpus))

    info = {
        "source": "opencorpora.org",
        "opencorpora_revision": revision,
        "corpus_revision": corpus_revision,
        "license": LICENSE,
    }
    tables = {
        "meta.json": info,
        "tags.json": tags,
        "prefixes.json": PARADIGM_PREFIXES,
        "suffixes.json": suffixes,
        "paradigms.json": layouts,
    }
    for name, table in tables.items():
        (output / name).write_text(json.dumps(table, ensure_ascii=False), encoding="utf-8")

    # Порядок словаря до 0.5: от него зависят первый разбор при равных долях и объединение лексем.
    # Одинаковые записи омонимичных лексем - одна запись
    words = sorted(set(words), key=lambda item: record_key(*item))
    with (output / "words.tsv").open("w", encoding="utf-8") as file:
        line: list[str] = []
        for number, (word, paradigm, form) in enumerate(words):
            line.append(f"{paradigm}:{form}")
            if number + 1 == len(words) or words[number + 1][0] != word:
                file.write(f"{word}\t{' '.join(line)}\n")
                line = []

    with (output / "probabilities.tsv").open("w", encoding="utf-8") as file:
        for word, tag, millionths in probabilities(corpus, tags, layouts, words):
            file.write(f"{word}\t{tag}\t{millionths}\n")


def corpus_revisions(path: Path) -> Iterator[str]:
    """
    Ревизия корпуса из его корневого элемента

    Аргументы:
        path (Path): Сжатый XML корпуса

    Вывод:
        Iterator[str]: Ревизия
    """
    with bz2.open(path) as file:
        for _, elem in ET.iterparse(file, events=("start",)):
            yield elem.get("revision", "")
            return


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.strip().splitlines()[0])
    parser.add_argument("output", type=Path, help="каталог выгрузки")
    export(parser.parse_args().output)
