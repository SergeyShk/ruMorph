# /// script
# requires-python = ">=3.11,<3.14"
# dependencies = [
#     "rumorph",
#     "pymorphy3==2.0.6",
#     "pymorphy3-dicts-ru==2.4.417150.4580142",
#     "pymystem3==0.2.0",
#     "spacy==3.8.16",
#     "ru-core-news-sm @ https://github.com/explosion/spacy-models/releases/download/ru_core_news_sm-3.8.0/ru_core_news_sm-3.8.0-py3-none-any.whl",
#     "stanza==1.15.0",
#     "slovnet==0.6.0",
#     "navec==0.10.0",
# ]
# ///
"""
Сравнение ruMorph с другими анализаторами: точность на тестовых частях трибанков UD и скорость

Описание:
    Скачивает тестовые части UD_Russian-SynTagRus, UD_Russian-Taiga и UD_Russian-GSD
    закрепленного релиза UD, модели Slovnet и mystem (с проверкой SHA-256) в каталог данных
    и разбирает эталонные токены каждой системой. Анализаторы без контекста (ruMorph, pymorphy3,
    mystem без снятия омонимии) оцениваются по первому разбору и по лучшему из разборов
    (оракул), теггеры с контекстом (spaCy, Stanza, Slovnet) - по своему ответу. Части речи
    pymorphy3 и mystem переводятся в UD простой таблицей (`PYMORPHY_POS`, `MYSTEM_POS`), падеж
    и число - тоже; признаки UD целиком сравниваются только у систем с признаками UD. Скорость -
    токены в секунду на SynTagRus test без загрузки (лучший из двух прогонов), а для графика -
    полное время с импортом и загрузкой в отдельном процессе на тексте разного объема. Пишет
    таблицы Markdown, results.json в каталог данных и график SVG. ruMorph - версия с PyPI или
    колесо из `--with`: сборка `--with-editable` отладочная, ее скорость не показательна.

Пример использования:
    uv run scripts/compare.py build/compare --chart docs/img/speed.svg
    uv run --with dist/rumorph-*.whl scripts/compare.py build/compare --systems rumorph pymorphy3
"""

import argparse
import hashlib
import json
import math
import os
import subprocess
import sys
import tarfile
import time
import urllib.request
import zipfile
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from importlib import metadata
from pathlib import Path

# Набор, файл, коммит релиза UD 2.18 и SHA-256
DATASETS = {
    "SynTagRus": (
        "UD_Russian-SynTagRus",
        "ru_syntagrus-ud-test.conllu",
        "c5266a6f6f4aaa94f4dfa9b6cc863b94a5358afa",
        "97f473474d1aa8c007572cd122f90030bc7dc1707579d56b0b354c19b64af258",
    ),
    "Taiga": (
        "UD_Russian-Taiga",
        "ru_taiga-ud-test.conllu",
        "d1e48cd29c1f19b6aad1aec361d9f724db764f66",
        "14183d7f002557e9e26b85e212dc54fe3f7929a7d72491615c50b9b3145e6278",
    ),
    "GSD": (
        "UD_Russian-GSD",
        "ru_gsd-ud-test.conllu",
        "9acc9d677327043bd416fcc89e4b3407c620d885",
        "f26e022329162a1c6306f76644d06f770f1572501755421165387137fe63138d",
    ),
}
# Файлы моделей и программ: адрес и SHA-256
DOWNLOADS = {
    "navec_news_v1_1B_250K_300d_100q.tar": (
        "https://storage.yandexcloud.net/natasha-navec/packs/navec_news_v1_1B_250K_300d_100q.tar",
        "f07270833d78523edc5781538d67038e95b43975e4a7ae757c693b687f9cbfca",
    ),
    "slovnet_morph_news_v1.tar": (
        "https://storage.yandexcloud.net/natasha-slovnet/packs/slovnet_morph_news_v1.tar",
        "54812762a641c742dbfa9c950c4b53bbb469301983077ed01b9f074643d540d3",
    ),
    "mystem-3.1-macosx.tar.gz": (
        "https://download.cdn.yandex.net/mystem/mystem-3.1-macosx.tar.gz",
        "346e576ada01cc7c63414a9d91f6733bd418f496f073d13a4812aec3628e5693",
    ),
    "mystem-3.1-linux-64bit.tar.gz": (
        "https://download.cdn.yandex.net/mystem/mystem-3.1-linux-64bit.tar.gz",
        "4696f4ea8ce3ecda24ef5e8dfe7e4b16cfa5f1844edfcca31c34d636b73c0a62",
    ),
    "mystem-3.1-win-64bit.zip": (
        "https://download.cdn.yandex.net/mystem/mystem-3.1-win-64bit.zip",
        "03cdbe2c01661eb449e84771817096161203553fca4bca934dc17f1bc9e53bc8",
    ),
}
# Основные признаки UD: их сравнение не зависит от помет, которых нет у других систем
CORE = (
    "Case",
    "Number",
    "Gender",
    "Person",
    "Tense",
    "Aspect",
    "VerbForm",
    "Mood",
    "Voice",
    "Degree",
    "Animacy",
)
# Части речи OpenCorpora в UD; имена собственные - по граммемам `PROPER`
PYMORPHY_POS = {
    "NOUN": "NOUN",
    "ADJF": "ADJ",
    "ADJS": "ADJ",
    "COMP": "ADJ",
    "VERB": "VERB",
    "INFN": "VERB",
    "PRTF": "VERB",
    "PRTS": "VERB",
    "GRND": "VERB",
    "NUMR": "NUM",
    "ADVB": "ADV",
    "NPRO": "PRON",
    "PRED": "ADV",
    "PREP": "ADP",
    "CONJ": "CCONJ",
    "PRCL": "PART",
    "INTJ": "INTJ",
    "LATN": "X",
    "NUMB": "NUM",
    "ROMN": "NUM",
    "PNCT": "PUNCT",
    "UNKN": "X",
}
PYMORPHY_PROPER = {"Name", "Surn", "Patr", "Geox", "Orgn", "Trad"}
PYMORPHY_CASE = {
    "nomn": "Nom",
    "gent": "Gen",
    "gen2": "Gen",
    "datv": "Dat",
    "accs": "Acc",
    "acc2": "Acc",
    "ablt": "Ins",
    "loct": "Loc",
    "loc2": "Loc",
    "voct": "Voc",
}
# Части речи mystem в UD; имена собственные - по граммемам `MYSTEM_PROPER`
MYSTEM_POS = {
    "S": "NOUN",
    "A": "ADJ",
    "V": "VERB",
    "ADV": "ADV",
    "ADVPRO": "ADV",
    "APRO": "DET",
    "ANUM": "ADJ",
    "NUM": "NUM",
    "SPRO": "PRON",
    "PR": "ADP",
    "CONJ": "CCONJ",
    "PART": "PART",
    "INTJ": "INTJ",
    "COM": "X",
}
MYSTEM_PROPER = {"имя", "фам", "отч", "гео"}
MYSTEM_CASE = {
    "им": "Nom",
    "род": "Gen",
    "парт": "Gen",
    "дат": "Dat",
    "вин": "Acc",
    "твор": "Ins",
    "пр": "Loc",
    "местн": "Loc",
    "зват": "Voc",
}
NUMBER = {"sing": "Sing", "plur": "Plur", "ед": "Sing", "мн": "Plur"}
# Объем текста для графика скорости, токенов
SIZES = (10, 100, 1_000, 10_000, 100_000)


@dataclass
class Token:
    word: str
    lemma: str
    pos: str
    feats: dict[str, str]


# Разбор токена: лемма (у Slovnet ее нет), часть речи UD и признаки UD
Candidate = tuple[str | None, str, dict[str, str]]


def download(url: str, path: Path, digest: str) -> Path:
    """
    Скачивание файла с проверкой контрольной суммы

    Аргументы:
        url (str): Адрес файла
        path (Path): Куда сохранить
        digest (str): SHA-256 файла

    Вывод:
        Path: Путь к файлу

    Исключения:
        ValueError: Если контрольная сумма не совпадает
    """
    if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() == digest:
        return path
    with urllib.request.urlopen(url, timeout=300) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError(f"{url}: контрольная сумма не совпадает")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return path


def sentences(path: Path) -> list[list[Token]]:
    """
    Предложения CoNLL-U без многословных токенов и пустых узлов

    Аргументы:
        path (Path): Файл CoNLL-U

    Вывод:
        list[list[Token]]: Предложения
    """
    result: list[list[Token]] = [[]]
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line:
            if result[-1]:
                result.append([])
            continue
        cols = line.split("\t")
        if line.startswith("#") or "-" in cols[0] or "." in cols[0]:
            continue
        feats = dict(f.split("=", 1) for f in cols[5].split("|")) if cols[5] != "_" else {}
        result[-1].append(Token(cols[1], cols[2], cols[3], feats))
    return [sentence for sentence in result if sentence]


def mystem_binary(data: Path) -> Path:
    """
    Программа mystem в каталоге данных: архив скачивается по HTTPS с проверкой SHA-256

    Аргументы:
        data (Path): Каталог данных

    Вывод:
        Path: Путь к программе
    """
    name = {
        "darwin": "mystem-3.1-macosx.tar.gz",
        "linux": "mystem-3.1-linux-64bit.tar.gz",
        "win32": "mystem-3.1-win-64bit.zip",
    }[sys.platform]
    binary = data / "mystem" / ("mystem.exe" if sys.platform == "win32" else "mystem")
    if binary.exists():
        return binary
    archive = download(DOWNLOADS[name][0], data / name, DOWNLOADS[name][1])
    binary.parent.mkdir(parents=True, exist_ok=True)
    if name.endswith(".zip"):
        with zipfile.ZipFile(archive) as packed:
            packed.extract(binary.name, binary.parent)
    else:
        with tarfile.open(archive) as packed:
            packed.extract(binary.name, binary.parent, filter="data")
    binary.chmod(0o755)
    return binary


# Разбор предложений системой: свой API (его время замеряется) и перевод его ответа в разборы
Analyze = Callable[[list[list[str]]], object]
Convert = Callable[[list[list[str]], object], list[list[list[Candidate]]]]


def rumorph_system(data: Path) -> tuple[Analyze, Convert]:
    """ruMorph: все разборы токена по убыванию вероятности"""
    from rumorph import Analyzer

    morph = Analyzer()

    def convert(text: list[list[str]], found: object) -> list[list[list[Candidate]]]:
        analyses = iter(found)
        return [
            [[(a.lemma, a.pos, dict(a.feats)) for a in next(analyses)] for _ in s] for s in text
        ]

    return (lambda text: morph.analyze_many([w for s in text for w in s])), convert


def pymorphy_system(data: Path) -> tuple[Analyze, Convert]:
    """pymorphy3: все разборы токена, часть речи, падеж и число - по таблицам"""
    import pymorphy3

    morph = pymorphy3.MorphAnalyzer()

    def candidate(parse) -> Candidate:
        tag = parse.tag
        # У токенов без части речи («NUMB,intg», «LATN») - первая граммема записи тега
        pos = PYMORPHY_POS.get(tag.POS or str(tag).split(",")[0].split(" ")[0], "X")
        if pos == "NOUN" and tag.grammemes & PYMORPHY_PROPER:
            pos = "PROPN"
        feats = {}
        if tag.case in PYMORPHY_CASE:
            feats["Case"] = PYMORPHY_CASE[tag.case]
        if tag.number in NUMBER:
            feats["Number"] = NUMBER[tag.number]
        return parse.normal_form, pos, feats

    def convert(text: list[list[str]], found: object) -> list[list[list[Candidate]]]:
        return [[[candidate(p) for p in parses] for parses in s] for s in found]

    return (lambda text: [[morph.parse(word) for word in s] for s in text]), convert


def mystem_candidates(word: str, items: list[dict]) -> list[Candidate]:
    """Разборы mystem одного токена: каждый вариант словоизменения - отдельный разбор"""
    result: list[Candidate] = []
    for item in items:
        for analysis in item.get("analysis", []):
            lexical, _, inflection = analysis.get("gr", "").partition("=")
            grammemes = lexical.split(",")
            pos = MYSTEM_POS.get(grammemes[0], "X")
            if pos == "NOUN" and MYSTEM_PROPER & set(grammemes):
                pos = "PROPN"
            # Варианты словоизменения: «(вин,ед|им,ед)»
            for variant in inflection.strip("()").split("|"):
                feats = {}
                for mark in set(variant.split(",")) | set(grammemes):
                    if mark in MYSTEM_CASE:
                        feats["Case"] = MYSTEM_CASE[mark]
                    if mark in NUMBER:
                        feats["Number"] = NUMBER[mark]
                result.append((analysis["lex"], pos, feats))
    # Без разбора (числа, знаки, латиница) - токен как есть
    return result or [(word.lower(), "X", {})]


def mystem_system(data: Path) -> tuple[Analyze, Convert]:
    """mystem без снятия омонимии: все разборы токена, разобранного отдельно"""
    from pymystem3 import Mystem

    mystem = Mystem(mystem_bin=str(mystem_binary(data)), disambiguation=False, entire_input=False)

    def convert(text: list[list[str]], found: object) -> list[list[list[Candidate]]]:
        return [
            [mystem_candidates(word, items) for word, items in zip(s, f, strict=True)]
            for s, f in zip(text, found, strict=True)
        ]

    return (lambda text: [[mystem.analyze(word) for word in s] for s in text]), convert


def spacy_system(data: Path) -> tuple[Analyze, Convert]:
    """spaCy ru_core_news_sm: теггер и лемматизатор на эталонных токенах"""
    import spacy
    from spacy.tokens import Doc

    nlp = spacy.load("ru_core_news_sm", exclude=["parser", "ner", "attribute_ruler"])

    def analyze(text: list[list[str]]) -> object:
        docs = []
        for sentence in text:
            doc = Doc(nlp.vocab, words=sentence)
            for _, component in nlp.pipeline:
                doc = component(doc)
            docs.append(doc)
        return docs

    def convert(text: list[list[str]], docs: object) -> list[list[list[Candidate]]]:
        return [[[(t.lemma_, t.pos_, t.morph.to_dict())] for t in doc] for doc in docs]

    return analyze, convert


def stanza_system(data: Path) -> tuple[Analyze, Convert]:
    """Stanza: теггер и лемматизатор модели по умолчанию на эталонных токенах"""
    import stanza

    directory = str(data / "stanza")
    processors = "tokenize,pos,lemma"
    stanza.download("ru", model_dir=directory, processors=processors, verbose=False)
    nlp = stanza.Pipeline(
        "ru",
        dir=directory,
        processors=processors,
        tokenize_pretokenized=True,
        download_method=None,
        use_gpu=False,
        verbose=False,
    )

    def feats(word) -> dict[str, str]:
        return dict(f.split("=", 1) for f in (word.feats or "").split("|") if f)

    def convert(text: list[list[str]], doc: object) -> list[list[list[Candidate]]]:
        return [[[(w.lemma, w.upos, feats(w))] for w in s.words] for s in doc.sentences]

    return nlp, convert


def slovnet_system(data: Path) -> tuple[Analyze, Convert]:
    """Slovnet, морфология Natasha: часть речи и признаки, без лемм"""
    from navec import Navec
    from slovnet import Morph

    paths = {}
    for file in ("navec_news_v1_1B_250K_300d_100q.tar", "slovnet_morph_news_v1.tar"):
        paths[file] = download(DOWNLOADS[file][0], data / file, DOWNLOADS[file][1])
    morph = Morph.load(str(paths["slovnet_morph_news_v1.tar"]), batch_size=32)
    morph.navec(Navec.load(str(paths["navec_news_v1_1B_250K_300d_100q.tar"])))

    def candidate(tag: str) -> Candidate:
        pos, *feats = tag.split("|")
        return None, pos, dict(f.split("=", 1) for f in feats if "=" in f)

    def convert(text: list[list[str]], markups: object) -> list[list[list[Candidate]]]:
        return [[[candidate(t.tag)] for t in m.tokens] for m in markups]

    return (lambda text: list(morph.map(text))), convert


SYSTEMS = {
    "rumorph": rumorph_system,
    "pymorphy3": pymorphy_system,
    "mystem": mystem_system,
    "spacy": spacy_system,
    "stanza": stanza_system,
    "slovnet": slovnet_system,
}


CONTEXT_FREE = ("rumorph", "pymorphy3", "mystem")
UD_SYSTEMS = ("rumorph", "spacy", "stanza", "slovnet")
CONTEXTUAL = ("spacy", "stanza", "slovnet")
TITLES = {
    "rumorph": "ruMorph",
    "pymorphy3": "pymorphy3",
    "mystem": "mystem",
    "spacy": "spaCy ru_core_news_sm",
    "stanza": "Stanza",
    "slovnet": "Slovnet (Natasha)",
}


def norm(lemma: str) -> str:
    return lemma.lower().replace("ё", "е")


def check(token: Token, candidate: Candidate) -> dict[str, bool]:
    """
    Верные ответы разбора токена по метрикам

    Аргументы:
        token (Token): Эталонный токен
        candidate (Candidate): Разбор

    Вывод:
        dict[str, bool]: Метрика - верна ли
    """
    lemma, pos, feats = candidate
    core = {k: v for k, v in feats.items() if k in CORE}
    gold = {k: v for k, v in token.feats.items() if k in CORE}
    lemma_ok = lemma is not None and norm(lemma) == norm(token.lemma)
    return {
        "lemma": lemma_ok,
        "pos": pos == token.pos,
        "pos+lemma": lemma_ok and pos == token.pos,
        "case": feats.get("Case") == token.feats.get("Case"),
        "number": feats.get("Number") == token.feats.get("Number"),
        "feats": core == gold,
        "all": lemma_ok and pos == token.pos and core == gold,
    }


def scores(
    gold: list[list[Token]], found: list[list[list[Candidate]]], ud: bool
) -> dict[str, float | None]:
    """
    Доли верных ответов первого разбора и оракула, без знаков препинания

    Аргументы:
        gold (list[list[Token]]): Эталонные предложения
        found (list[list[list[Candidate]]]): Разборы токенов
        ud (bool): Признаки системы - признаки UD: иначе сравниваются только падеж и число

    Вывод:
        dict[str, float | None]: Метрика - доля в процентах; `None`, если система ее не дает
    """
    hits: dict[str, int] = {}
    total = 0
    lemmas = all(c[0] is not None for s in found for t in s for c in t[:1])
    for sentence, analyses in zip(gold, found, strict=True):
        for token, candidates in zip(sentence, analyses, strict=True):
            if token.pos == "PUNCT":
                continue
            total += 1
            for key, ok in check(token, candidates[0]).items():
                hits[key] = hits.get(key, 0) + ok
            oracle = any(check(token, candidate)["pos+lemma"] for candidate in candidates)
            hits["oracle"] = hits.get("oracle", 0) + oracle
    result: dict[str, float | None] = {key: 100 * value / total for key, value in hits.items()}
    if not lemmas:
        for key in ("lemma", "pos+lemma", "all", "oracle"):
            result[key] = None
    if not ud:
        result["feats"] = result["all"] = None
    return result


def speed_run(name: str, data: Path, size: int) -> None:
    """
    Замер в чистом процессе: время импорта и загрузки системы, первого разбора первых `size`
    токенов SynTagRus test (0 - всех) и лучшего из двух разборов

    Аргументы:
        name (str): Имя системы
        data (Path): Каталог данных
        size (int): Число токенов
    """
    text: list[list[str]] = []
    count = 0
    for sentence in sentences(data / DATASETS["SynTagRus"][1]):
        words = [token.word for token in sentence][: size - count if size else None]
        text.append(words)
        count += len(words)
        if size and count >= size:
            break
    start = time.perf_counter()
    analyze, _ = SYSTEMS[name](data)
    loaded = time.perf_counter()
    analyze(text)
    first = time.perf_counter()
    analyze(text)
    best = min(first - loaded, time.perf_counter() - first)
    result = {"tokens": count, "load": loaded - start, "first": first - loaded, "best": best}
    print(json.dumps(result))


def measure(name: str, data: Path, size: int) -> dict[str, float]:
    """
    Замер `speed_run` в отдельном процессе

    Аргументы:
        name (str): Имя системы
        data (Path): Каталог данных
        size (int): Число токенов, 0 - все

    Вывод:
        dict[str, float]: Токены и время в секундах
    """
    run = subprocess.run(
        [sys.executable, __file__, str(data), "--speed-run", name, str(size)],
        capture_output=True,
        text=True,
        check=True,
        env={**os.environ, "PYTHONWARNINGS": "ignore"},
    )
    return json.loads(run.stdout.strip().splitlines()[-1])


def chart(curves: dict[str, list[tuple[int, float]]], path: Path, english: bool = False) -> None:
    """
    График SVG: время разбора текста с загрузкой от объема текста, обе оси логарифмические

    Аргументы:
        curves (dict[str, list[tuple[int, float]]]): Система - точки «токенов, секунд»
        path (Path): Файл SVG
        english (bool): Подписи и числа по-английски
    """
    width, height, left, right, top, bottom = 780, 420, 70, 240, 20, 50
    xs = [x for points in curves.values() for x, _ in points]
    ys = [y for points in curves.values() for _, y in points]
    x_range = (math.log10(min(xs)), math.log10(max(xs)))
    y_range = (math.floor(math.log10(min(ys))), math.ceil(math.log10(max(ys))))
    plot = (width - left - right, height - top - bottom)

    def to_x(value: float) -> float:
        return left + (math.log10(value) - x_range[0]) / (x_range[1] - x_range[0]) * plot[0]

    def to_y(value: float) -> float:
        share = (math.log10(value) - y_range[0]) / (y_range[1] - y_range[0])
        return height - bottom - share * plot[1]

    def number(value: float) -> str:
        text = f"{value:,}" if value >= 1 else f"{value:.{-math.floor(math.log10(value))}f}"
        return text if english else text.replace(",", " ").replace(".", ",")

    colors = ["#1e88e5", "#e53935", "#43a047", "#fb8c00", "#8e24aa", "#00acc1"]
    style = 'font-family="sans-serif" font-size="13" fill="#888"'
    grid = 'stroke="#888" stroke-opacity="0.3"'
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" '
        f'width="{width}" height="{height}">'
    ]
    for power in range(y_range[0], y_range[1] + 1):
        y = to_y(10**power)
        value = number(10**power if power >= 0 else 10.0**power)
        parts.append(f'<line x1="{left}" y1="{y:.1f}" x2="{width - right}" y2="{y:.1f}" {grid}/>')
        parts.append(
            f'<text x="{left - 8}" y="{y + 4:.1f}" text-anchor="end" {style}>{value}</text>'
        )
    for size in sorted(set(xs)):
        x = to_x(size)
        parts.append(f'<line x1="{x:.1f}" y1="{top}" x2="{x:.1f}" y2="{height - bottom}" {grid}/>')
        y = height - bottom + 18
        parts.append(
            f'<text x="{x:.1f}" y="{y}" text-anchor="middle" {style}>{number(size)}</text>'
        )
    tokens, seconds = ("tokens", "seconds") if english else ("токенов", "секунд")
    middle = (top + height - bottom) / 2
    parts.append(
        f'<text x="{left + plot[0] / 2}" y="{height - 8}" text-anchor="middle" {style}>{tokens}</text>'
    )
    parts.append(
        f'<text x="16" y="{middle}" transform="rotate(-90 16 {middle})" '
        f'text-anchor="middle" {style}>{seconds}</text>'
    )
    for index, (name, points) in enumerate(curves.items()):
        color = colors[index % len(colors)]
        line = " ".join(f"{to_x(x):.1f},{to_y(y):.1f}" for x, y in points)
        parts.append(
            f'<polyline points="{line}" fill="none" stroke="{color}" stroke-width="2.5"/>'
        )
        for x, y in points:
            parts.append(f'<circle cx="{to_x(x):.1f}" cy="{to_y(y):.1f}" r="3.5" fill="{color}"/>')
        y = top + 14 + index * 22
        x = width - right + 16
        parts.append(
            f'<line x1="{x}" y1="{y - 4}" x2="{x + 24}" y2="{y - 4}" stroke="{color}" stroke-width="2.5"/>'
        )
        parts.append(f'<text x="{x + 32}" y="{y}" {style}>{TITLES[name]}</text>')
    parts.append("</svg>")
    path.write_text("\n".join(parts) + "\n", encoding="utf-8")


def table(results: dict, names: list[str], keys: list[tuple[str, str]]) -> Iterator[str]:
    """
    Строки таблицы Markdown: системы по строкам, метрики по наборам по столбцам

    Аргументы:
        results (dict): Метрики систем по наборам
        names (list[str]): Системы
        keys (list[tuple[str, str]]): Метрики и их заголовки
    """
    header = ["Система"] + [f"{title}, {dataset}" for dataset in DATASETS for _, title in keys]
    yield "| " + " | ".join(header) + " |"
    yield "| " + " | ".join(":--" for _ in header) + " |"
    for name in names:
        cells = [TITLES[name]]
        for dataset in DATASETS:
            for key, _ in keys:
                value = results[name]["quality"][dataset].get(key)
                cells.append("-" if value is None else f"{value:.1f}".replace(".", ","))
        yield "| " + " | ".join(cells) + " |"


def evaluate(systems: list[str], data: Path) -> dict:
    """
    Точность систем на трех наборах и их скорость

    Аргументы:
        systems (list[str]): Системы
        data (Path): Каталог данных

    Вывод:
        dict: Метрики качества по наборам, токены в секунду и время на тексте разного объема
    """
    gold = {}
    for dataset, (treebank, name, commit, digest) in DATASETS.items():
        url = f"https://raw.githubusercontent.com/UniversalDependencies/{treebank}/{commit}/{name}"
        gold[dataset] = sentences(download(url, data / name, digest))
    results: dict = {}
    for name in systems:
        analyze, convert = SYSTEMS[name](data)
        quality = {}
        for dataset, text in gold.items():
            words = [[token.word for token in sentence] for sentence in text]
            quality[dataset] = scores(text, convert(words, analyze(words)), name in UD_SYSTEMS)
        # Время с загрузкой на тексте разного объема и скорость без загрузки - в чистом процессе
        curve = []
        for size in SIZES:
            run = measure(name, data, size)
            curve.append((size, run["load"] + run["first"]))
        full = measure(name, data, 0)
        speed = full["tokens"] / full["best"]
        results[name] = {"quality": quality, "speed": speed, "curve": curve}
        print(f"{TITLES[name]}: {speed:,.0f} токенов/с", file=sys.stderr)
    return results


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.strip().splitlines()[0])
    parser.add_argument("data", type=Path, help="каталог данных")
    parser.add_argument(
        "--systems",
        nargs="+",
        default=list(CONTEXT_FREE + CONTEXTUAL),
        choices=CONTEXT_FREE + CONTEXTUAL,
    )
    parser.add_argument(
        "--chart", type=Path, help="файл графика SVG; рядом - английский вариант с .en.svg"
    )
    parser.add_argument(
        "--report", action="store_true", help="таблицы и график из results.json без замеров"
    )
    parser.add_argument("--speed-run", nargs=2, metavar=("SYSTEM", "SIZE"), help=argparse.SUPPRESS)
    args = parser.parse_args()
    data: Path = args.data
    if args.speed_run:
        speed_run(args.speed_run[0], data, int(args.speed_run[1]))
        return
    # Версии Python и пакетов окружения - для описания замера
    packages = ["rumorph", "pymorphy3", "pymorphy3-dicts-ru", "pymystem3", "spacy"]
    packages += ["ru-core-news-sm", "stanza", "torch", "slovnet", "navec"]
    environment = {"python": sys.version.split()[0]}
    environment |= {package: metadata.version(package) for package in packages}
    (data / "environment.json").write_text(json.dumps(environment, indent=2), encoding="utf-8")
    if args.report:
        results = json.loads((data / "results.json").read_text(encoding="utf-8"))
    else:
        results = evaluate(args.systems, data)
        (data / "results.json").write_text(
            json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8"
        )
    free = [name for name in CONTEXT_FREE if name in results]
    contextual = [name for name in ("rumorph", *CONTEXTUAL) if name in results]
    print("## Без контекста\n")
    print(
        "\n".join(
            table(results, free, [("pos+lemma", "часть речи и лемма"), ("oracle", "оракул")])
        )
    )
    print("\n## С контекстом\n")
    print(
        "\n".join(
            table(
                results,
                contextual,
                [("pos", "часть речи"), ("lemma", "лемма"), ("feats", "признаки")],
            )
        )
    )
    print("\n## Подробно\n")
    keys = [
        ("lemma", "лемма"),
        ("pos", "часть речи"),
        ("case", "падеж"),
        ("number", "число"),
        ("feats", "признаки"),
        ("all", "все вместе"),
    ]
    for dataset in DATASETS:
        print(f"\n### {dataset}\n")
        print("| Система | " + " | ".join(title for _, title in keys) + " |")
        print("| :-- | " + " | ".join(":--" for _ in keys) + " |")
        for name in results:
            values = [results[name]["quality"][dataset].get(key) for key, _ in keys]
            cells = ["-" if v is None else f"{v:.1f}".replace(".", ",") for v in values]
            print(f"| {TITLES[name]} | " + " | ".join(cells) + " |")
    print("\n## Скорость\n")
    sizes = " | ".join(f"{size:,}".replace(",", " ") for size in SIZES)
    print(f"| Система | токенов/с | {sizes} |")
    print("| :-- | :-- | " + " | ".join(":--" for _ in SIZES) + " |")
    for name, result in results.items():
        times = " | ".join(f"{seconds:.3g}".replace(".", ",") for _, seconds in result["curve"])
        speed = f"{result['speed']:,.0f}".replace(",", " ")
        print(f"| {TITLES[name]} | {speed} | {times} |")
    if args.chart:
        curves = {name: result["curve"] for name, result in results.items()}
        chart(curves, args.chart)
        chart(curves, args.chart.with_suffix(".en.svg"), english=True)


if __name__ == "__main__":
    main()
