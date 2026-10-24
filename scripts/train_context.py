# /// script
# requires-python = ">=3.11,<3.14"
# dependencies = [
#     "rumorph",
#     "numpy==2.3.4",
#     "torch==2.14.1",
# ]
#
# [tool.uv.sources]
# rumorph = { path = "..", editable = true }
# ///
"""
Обучение контекстной модели ruMorph: выбор разбора словоформы по соседним словам

Описание:
    Скачивает обучающие и проверочные части UD_Russian-Taiga и UD_Russian-GSD (CC BY-SA 4.0)
    закрепленного релиза UD и корпус OpenCorpora (CC BY-SA 3.0) с проверкой SHA-256, приводит
    разметку трибанков к соглашениям ruMorph (`conventions.py`, признаки - по разборам словаря),
    разбирает их токены словарем `--dictionary` и учит модель выбирать среди разборов словаря
    тот, что совпадает с эталоном по части речи, лемме и основным признакам. В корпусе OpenCorpora
    верны разборы, которые сборщик выводит из любого оставшегося варианта токена (`rumorph-build
    tags`): частично снятая неоднозначность тоже учит модель; часть речи неизменяемых служебных
    слов корпус не задает - ее соглашения в UD другие. Модель - BiLSTM над признаками токена (запись, слово, окончания,
    теги его разборов) и оценка каждого разбора скалярным произведением с вектором токена.
    Эпоха выбирается по full core на проверочных частях Taiga и GSD. Веса квантуются в int8
    по строкам и пишутся в файл `--output`, который `rumorph-build` вшивает в словарь; точность
    квантованной модели печатается. Версии пакетов закреплены в train_context.py.lock; ruMorph -
    сборка проекта (из репозитория), кандидаты которой должны совпадать со словарем.

Пример использования:
    uv run scripts/train_context.py --dictionary python/rumorph/data/ru.dict \\
        --data target/context --output data/ru-context.bin
"""

import argparse
import bz2
import hashlib
import json
import math
import shutil
import struct
import subprocess
import time
import urllib.request
import xml.etree.ElementTree as ET
from collections import Counter
from pathlib import Path

import numpy as np
import torch
from torch import nn

import conventions
from conventions import GSD, TAIGA, harmonize
from rumorph import Analyzer

# Трибанк, коммит релиза и файлы с SHA-256: обучающие части - как в export_ud.py
SOURCES = {
    "UD_Russian-Taiga": (
        "d1e48cd29c1f19b6aad1aec361d9f724db764f66",
        {
            "ru_taiga-ud-train-a.conllu": "d12f8d99e1370fad142fdbd5aea387715249bc34bf5494c5d6dd63c795dfa85e",
            "ru_taiga-ud-train-b.conllu": "82c0b23692ff0ebcdb7e53d40d74396b75dc92a076e4c0a014c1f32e10cb14ac",
            "ru_taiga-ud-train-c.conllu": "9a6301fd797395b2b7c5fbc25880f445248ba04571e4cc691e38a2fd3130e738",
            "ru_taiga-ud-train-d.conllu": "6bc5c4ce2e7389f6956373c8e565ba63a3501206988dbf6b64dcb1305be79021",
            "ru_taiga-ud-train-e.conllu": "07bab379f51eafb3aedc46fd77a1a96a01d3d8931186387a373d8a6c2689b02d",
            "ru_taiga-ud-dev.conllu": "91e8be6f74afd91623500d73954c5595db51a2eb3c7b0e02e38fb3b2508b1b6f",
        },
    ),
    "UD_Russian-GSD": (
        "9acc9d677327043bd416fcc89e4b3407c620d885",
        {
            "ru_gsd-ud-train.conllu": "0ae33a4bbda12697e2eb0a32af4971bda73af5852c8521744a974a316c02bab5",
            "ru_gsd-ud-dev.conllu": "e3e3d4e8d1d7544e531b2b81ae64257efacc97f93276ce6d2ce52fd1de052fa2",
        },
    ),
}
# Корпус OpenCorpora, ревизия 4580223: сайт или его снимок в Wayback Machine
OPENCORPORA = (
    "annot.opcorpora.xml.bz2",
    [
        "https://web.archive.org/web/20240624222734id_/http://opencorpora.org/files/export/annot/annot.opcorpora.xml.bz2",
        "https://opencorpora.org/files/export/annot/annot.opcorpora.xml.bz2",
    ],
    "dff2b07a3db865eb234515c3c2c51e370fc67b3582f32af7174625c8f2be4422",
)
# Теги OpenCorpora без части речи (числа, знаки, латиница, незнакомые слова): верен любой разбор
UNTAGGED = {"PNCT", "NUMB", "LATN", "ROMN", "SYMB", "UNKN", "TIME", "DATE"}
# Неизменяемые служебные части речи: их выбор из корпуса OpenCorpora не берется
FUNCTION = {"ADP", "ADV", "CCONJ", "INTJ", "PART", "SCONJ"}
TRAIN = [f"ru_taiga-ud-train-{part}.conllu" for part in "abcde"] + ["ru_gsd-ud-train.conllu"]
DEV = {"Taiga": ["ru_taiga-ud-dev.conllu"], "GSD": ["ru_gsd-ud-dev.conllu"]}
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
SOURCE = {"dictionary": 0, "rule": 1, "guess": 2}
# Разборов токена в модели не больше KMAX; ранг разбора по вероятности - не больше RANKS - 1
KMAX = 32
RANKS = 8
SUFFIX_MAX = 4
PROB_EDGES = (1e-4, 1e-3, 0.01, 0.03, 0.1, 0.2, 0.35, 0.5, 0.65, 0.8, 0.9, 0.97, 0.995)
MAGIC = b"RUMCTX\0\0"
FORMAT = 1


def download(urls: list[str], path: Path, digest: str) -> None:
    """
    Скачивание файла с проверкой контрольной суммы: по первому адресу, с которого пришел файл
    с нужной суммой

    Аргументы:
        urls (list[str]): Адреса файла
        path (Path): Куда сохранить
        digest (str): SHA-256 файла

    Исключения:
        ValueError: Если ни один адрес не дал файл с нужной суммой
    """
    if path.exists() and hashlib.sha256(path.read_bytes()).hexdigest() == digest:
        return
    errors = []
    for url in urls:
        try:
            with urllib.request.urlopen(url, timeout=600) as response:
                data = response.read()
        except OSError as err:
            errors.append(f"{url}: {err}")
            continue
        if hashlib.sha256(data).hexdigest() != digest:
            errors.append(f"{url}: контрольная сумма не совпадает")
            continue
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return
    raise ValueError("; ".join(errors))


def sentences(paths: list[Path]) -> list[list[tuple[str, str, str, dict[str, str]]]]:
    """
    Предложения CoNLL-U в соглашениях ruMorph (`conventions.py`): форма, лемма, часть речи
    и признаки токенов; многословные и пустые токены пропускаются

    Аргументы:
        paths (list[Path]): Файлы CoNLL-U

    Вывод:
        list[list[tuple[str, str, str, dict[str, str]]]]: Предложения
    """
    result = []
    for path in paths:
        treebank = GSD if path.name.startswith("ru_gsd") else TAIGA
        for sentence in conventions.sentences(path):
            harmonize(treebank, sentence)
            result.append([(t.form, t.lemma, t.upos, t.feats) for t in sentence])
    return result


def harmonize_features(
    lemma: str, upos: str, feats: dict[str, str], analyses: list
) -> tuple[str, dict[str, str]]:
    """
    Лемма и основные признаки эталона в соглашениях словаря: признак, которого нет ни у одного
    разбора словаря с той же частью речи и леммой, убирается, а одинаковый у всех них -
    добавляется; у сравнительной степени наречия лемма - как у разбора словаря («раньше» - «рано»)

    Аргументы:
        lemma (str): Лемма эталона
        upos (str): Часть речи эталона
        feats (dict[str, str]): Основные признаки эталона
        analyses (list): Разборы словаря

    Вывод:
        tuple[str, dict[str, str]]: Лемма и признаки
    """
    same = [a for a in analyses if a.pos == upos and normal(a.lemma) == normal(lemma)]
    if not same and upos == "ADV" and feats.get("Degree") == "Cmp":
        same = [a for a in analyses if a.pos == "ADV" and a.feats.get("Degree") == "Cmp"][:1]
        lemma = same[0].lemma if same else lemma
    if not same:
        return lemma, feats
    feats = dict(feats)
    for name in CORE:
        values = {a.feats.get(name) for a in same}
        if values == {None}:
            feats.pop(name, None)
        elif None not in values and len(values) == 1 and name not in feats:
            feats[name] = values.pop()
    return lemma, feats


def normal(text: str) -> str:
    """
    Слово для таблицы слов и окончаний: нижний регистр, «е» вместо «ё»

    Аргументы:
        text (str): Токен

    Вывод:
        str: Слово
    """
    return text.lower().replace("ё", "е")


def suffixes(word: str) -> list[str]:
    """
    Окончания слова длиной 1-4 с началом «^»: ключ - длина и окончание

    Аргументы:
        word (str): Слово (`normal`)

    Вывод:
        list[str]: Ключи окончаний
    """
    marked = "^" + word
    return [f"{n}{marked[-n:]}" for n in range(1, SUFFIX_MAX + 1)]


def shape(token: str, first: bool) -> int:
    """
    Запись токена: знаки, число, буквы в нижнем, верхнем регистре, с заглавной или смешанно,
    латиница, начало предложения и дефис

    Аргументы:
        token (str): Токен
        first (bool): Первый токен предложения

    Вывод:
        int: Номер записи, 0-63
    """
    digits = [ch for ch in token if "0" <= ch <= "9"]
    letters = [ch for ch in token if ch.isalpha()]
    if not digits and not letters:
        kind = 0
    elif digits:
        kind = 1 if len(digits) == len(token) else 2
    else:
        lower = all(not ch.isupper() for ch in letters)
        upper = all(not ch.islower() for ch in letters)
        if lower:
            kind = 3
        elif upper and len(letters) > 1:
            kind = 4
        elif letters[0].isupper() and all(not ch.isupper() for ch in letters[1:]):
            kind = 5
        else:
            kind = 6
        kind += 4 * any("a" <= ch.lower() <= "z" for ch in letters)
    return kind * 4 + 2 * first + ("-" in token)


def tag_names(pos: str, feats: dict[str, str]) -> tuple[list[str], str]:
    """
    Теги разбора для модели: часть речи, каждый признак и полный тег

    Аргументы:
        pos (str): Часть речи UD
        feats (dict[str, str]): Признаки UD

    Вывод:
        tuple[list[str], str]: Теги части речи и признаков, полный тег
    """
    pairs = sorted(feats.items())
    full = "C=" + "|".join([pos] + [f"{name}={value}" for name, value in pairs])
    return [f"P={pos}"] + [f"F={name}={value}" for name, value in pairs], full


def bucket(prob: float) -> int:
    """
    Корзина вероятности разбора: число порогов `PROB_EDGES` меньше вероятности

    Аргументы:
        prob (float): Вероятность

    Вывод:
        int: Номер корзины
    """
    return sum(edge < prob for edge in PROB_EDGES)


class Data:
    """Токены предложений и их разборы словарем: признаки для модели и совпадение с эталоном"""

    def __init__(self, sents: list, morph: Analyzer) -> None:
        self.sents = sents
        forms = [token[0] for sent in sents for token in sent]
        self.offsets = np.cumsum([0] + [len(sent) for sent in sents])
        self.forms = forms
        self.analyses = morph.analyze_many(forms)
        golds = [token for sent in sents for token in sent]
        self.evaluated = np.array(
            [g[2] != "PUNCT" and len(a) > 0 for g, a in zip(golds, self.analyses, strict=True)]
        )
        # Биты совпадения разбора с эталоном: часть речи 1, лемма 2, основные признаки 4, падеж 8, есть падеж 16
        self.bits = []
        for (_, lemma, upos, feats), analyses in zip(golds, self.analyses, strict=True):
            gold = {k: v for k, v in feats.items() if k in CORE}
            lemma, gold = harmonize_features(lemma, upos, gold, analyses)
            row = []
            for a in analyses[:KMAX]:
                ours = {k: v for k, v in a.feats.items() if k in CORE}
                bits = (
                    (a.pos == upos) | (normal(a.lemma) == normal(lemma)) << 1 | (ours == gold) << 2
                )
                if "Case" in ours or "Case" in gold:
                    bits |= 16 | (ours.get("Case") == gold.get("Case")) << 3
                row.append(bits)
            self.bits.append(row)


def opencorpora(path: Path) -> list[list[tuple[str, list[tuple[str, str]]]]]:
    """
    Предложения корпуса OpenCorpora: токены и их оставшиеся варианты (тег, лемма); предложения
    с токеном без вариантов пропускаются

    Аргументы:
        path (Path): Выгрузка корпуса (`annot.opcorpora.xml.bz2`)

    Вывод:
        list[list[tuple[str, list[tuple[str, str]]]]]: Предложения
    """
    result = []
    with bz2.open(path) as file:
        for _, element in ET.iterparse(file):
            if element.tag != "sentence":
                continue
            sentence = []
            for token in element.iter("token"):
                variants = [
                    (",".join(g.get("v", "") for g in lexeme.iter("g")), lexeme.get("t", ""))
                    for lexeme in token.iter("l")
                ]
                if not variants:
                    sentence = []
                    break
                sentence.append((token.get("text", ""), variants))
            if sentence:
                result.append(sentence)
            element.clear()
    return result


def derived(pairs: set[tuple[str, str]]) -> dict[tuple[str, str], list]:
    """
    Разборы UD, которые сборщик словаря выводит из тега OpenCorpora и леммы: перевод и копии
    (`rumorph-build tags`)

    Аргументы:
        pairs (set[tuple[str, str]]): Пары (тег, лемма)

    Вывод:
        dict[tuple[str, str], list]: Пара - список (лемма, часть речи, признаки)
    """
    ordered = sorted(pairs)
    output = subprocess.run(
        ["cargo", "run", "--release", "--locked", "--quiet", "-p", "rumorph-build", "--", "tags"],
        input="".join(f"{tag}\t{lemma}\n" for tag, lemma in ordered),
        capture_output=True,
        text=True,
        check=True,
        cwd=Path(__file__).resolve().parent.parent,
    ).stdout.splitlines()
    return {
        pair: [
            (lemma, pos, dict(f.split("=", 1) for f in feats.split("|")) if feats else {})
            for lemma, pos, feats in json.loads(line)
        ]
        for pair, line in zip(ordered, output, strict=True)
    }


def corpus_key(pos: str, lemma: str, feats: dict[str, str]) -> tuple:
    """
    Ключ сравнения разбора словаря с разбором корпуса OpenCorpora: без различий, которых корпус
    не задает

    Аргументы:
        pos (str): Часть речи UD
        lemma (str): Лемма
        feats (dict[str, str]): Признаки UD

    Вывод:
        tuple: Часть речи, лемма и основные признаки
    """
    feats = {k: v for k, v in feats.items() if k in CORE}
    lemma = normal(lemma)
    # «тем» («тем не менее») в OpenCorpora - союз, в UD - местоимение «то»
    if (pos, lemma) == ("SCONJ", "тем"):
        pos, lemma = "PRON", "то"
        feats = {"Animacy": "Inan", "Case": "Ins", "Gender": "Neut", "Number": "Sing"}
    # Границу имени собственного корпус проводит иначе, чем UD: «Запад», «Дальний Восток» - NOUN
    if pos == "PROPN":
        pos = "NOUN"
    # COMP - сравнительная степень прилагательного и наречия; лемма наречия - из другой лексемы
    if feats.get("Degree") == "Cmp":
        return "Cmp", "", tuple(sorted(feats.items()))
    if pos in FUNCTION and "Case" not in feats:
        return "Function", lemma, ()
    return pos, lemma, tuple(sorted(feats.items()))


class Corpus:
    """Токены корпуса OpenCorpora в виде `Data`: биты 7 у разборов, совпавших с вариантами токена"""

    def __init__(self, sents: list, morph: Analyzer, analyses_of: dict) -> None:
        self.forms = [form for sent in sents for form, _ in sent]
        self.offsets = np.cumsum([0] + [len(sent) for sent in sents])
        self.analyses = morph.analyze_many(self.forms)
        self.evaluated = np.zeros(len(self.forms), dtype=bool)
        self.bits = []
        tokens = [variants for sent in sents for _, variants in sent]
        for variants, analyses in zip(tokens, self.analyses, strict=True):
            keys = [corpus_key(a.pos, a.lemma, a.feats) for a in analyses[:KMAX]]
            right = set()
            if any(tag.split(",")[0] in UNTAGGED for tag, _ in variants):
                right = set(range(len(keys)))
                variants = []
            # У форм «быть» в корпусе лемма «есть»; инфинитив «есть» - другой глагол
            be = any(a.lemma == "быть" for a in analyses)
            for tag, lemma in variants:
                golds = []
                pair = (tag, "быть" if lemma == "есть" and be and tag[:4] != "INFN" else lemma)
                for gold_lemma, pos, feats in analyses_of[pair]:
                    core = {k: v for k, v in feats.items() if k in CORE}
                    gold_lemma, core = harmonize_features(gold_lemma, pos, core, analyses)
                    golds.append(corpus_key(pos, gold_lemma, core))
                found = {i for i, k in enumerate(keys) if k in golds}
                if not found:
                    # Лемма корпуса бывает другой: «важнейший» - «важный», предлог «об» - «о»
                    loose = {(g[0], g[2]) for g in golds}
                    found = {i for i, k in enumerate(keys) if (k[0], k[2]) in loose}
                right |= found
            # Наречие из творительного падежа («вечером», «рядом») корпус и UD различают по-разному
            adverbs = {i for i, a in enumerate(analyses[:KMAX]) if a.pos == "ADV"}
            nouns = {
                i
                for i, a in enumerate(analyses[:KMAX])
                if a.pos == "NOUN" and a.feats.get("Case") == "Ins"
            }
            if adverbs and nouns and right & (adverbs | nouns):
                right |= adverbs | nouns
            self.bits.append([7 * (i in right) for i in range(len(keys))])


class Merged:
    """Несколько наборов токенов как один `Data`"""

    def __init__(self, parts: list) -> None:
        self.forms = [form for part in parts for form in part.forms]
        self.analyses = [a for part in parts for a in part.analyses]
        self.bits = [b for part in parts for b in part.bits]
        self.evaluated = np.concatenate([part.evaluated for part in parts])
        offsets, base = [0], 0
        for part in parts:
            offsets.extend(base + part.offsets[1:])
            base += part.offsets[-1]
        self.offsets = np.array(offsets)


class Vocabulary:
    """Таблицы слов, окончаний и тегов по обучающим данным"""

    def __init__(self, train: Data, words: int, suffix_count: int, inventory: list[str]) -> None:
        counts = Counter(normal(form) for form in train.forms)
        ranked = sorted(counts.items(), key=lambda item: (-item[1], item[0]))
        self.words = [word for word, _ in ranked[:words]]
        suffix_counts = Counter()
        for word, count in counts.items():
            for key in suffixes(word):
                suffix_counts[key] += count
        ranked = sorted(
            (item for item in suffix_counts.items() if item[1] >= 3),
            key=lambda item: (-item[1], item[0]),
        )
        self.suffixes = [key for key, _ in ranked[:suffix_count]]
        # Полный тег - отдельным тегом, если он верен у разборов train не реже 5 раз
        full_counts, tags = Counter(), set()
        for analyses, bits in zip(train.analyses, train.bits, strict=True):
            for a, b in zip(analyses, bits, strict=False):
                names, full = tag_names(a.pos, a.feats)
                tags.update(names)
                if b & 7 == 7:
                    full_counts[full] += 1
        self.tags = sorted(tags) + sorted(
            full for full, count in full_counts.items() if count >= 5
        )
        self.inventory = inventory
        self.word_id = {word: i + 2 for i, word in enumerate(self.words)}
        self.suffix_id = {key: i + 2 for i, key in enumerate(self.suffixes)}
        self.tag_id = {tag: i + 1 for i, tag in enumerate(self.tags)}

    def candidate_tags(self, pos: str, feats: dict[str, str]) -> list[int]:
        """
        Номера тегов разбора; неизвестные теги пропускаются

        Аргументы:
            pos (str): Часть речи UD
            feats (dict[str, str]): Признаки UD

        Вывод:
            list[int]: Номера тегов
        """
        names, full = tag_names(pos, feats)
        return [self.tag_id[name] for name in [*names, full] if name in self.tag_id]


def featurize(data: Data, vocab: Vocabulary) -> dict[str, np.ndarray]:
    """
    Признаки токенов и разборов в массивах: разборы токена `i` - строки `off[i]:off[i + 1]`

    Аргументы:
        data (Data): Токены и разборы
        vocab (Vocabulary): Таблицы

    Вывод:
        dict[str, np.ndarray]: Массивы признаков
    """
    first = np.zeros(len(data.forms), dtype=bool)
    first[data.offsets[:-1]] = True
    words = [normal(form) for form in data.forms]
    tag_lists, off = [], [0]
    rank, buckets, source, probs, gold = [], [], [], [], []
    for analyses, bits in zip(data.analyses, data.bits, strict=True):
        analyses = analyses[:KMAX]
        p = [a.prob for a in analyses]
        order = sorted(range(len(p)), key=lambda j: -p[j])
        ranks = [0] * len(p)
        for place, j in enumerate(order):
            ranks[j] = min(place, RANKS - 1)
        for a, r, b in zip(analyses, ranks, bits, strict=True):
            tag_lists.append(vocab.candidate_tags(a.pos, a.feats))
            rank.append(r)
            buckets.append(bucket(a.prob))
            source.append(SOURCE[a.source])
            probs.append(a.prob)
            gold.append(b & 7 == 7)
        off.append(len(rank))
    width = max(len(tags) for tags in tag_lists)
    tags = np.zeros((len(tag_lists), width), dtype=np.int64)
    for i, row in enumerate(tag_lists):
        tags[i, : len(row)] = row
    return {
        "word": np.array([vocab.word_id.get(w, 1) for w in words]),
        "suffix": np.array([[vocab.suffix_id.get(key, 1) for key in suffixes(w)] for w in words]),
        "shape": np.array([shape(form, f) for form, f in zip(data.forms, first, strict=True)]),
        "off": np.array(off),
        "tags": tags,
        "rank": np.array(rank),
        "bucket": np.array(buckets),
        "source": np.array(source),
        "prob": np.array(probs, dtype=np.float64),
        "gold": np.array(gold, dtype=bool),
        "sent_off": data.offsets,
    }


class Model(nn.Module):
    """Оценки разборов токенов предложения"""

    def __init__(self, vocab: Vocabulary, args: argparse.Namespace) -> None:
        super().__init__()
        d, dc = args.d, args.dc
        self.word = nn.Embedding(len(vocab.words) + 2, args.dw, padding_idx=0)
        self.suffix = nn.Embedding(len(vocab.suffixes) + 2, args.ds, padding_idx=0)
        self.shape = nn.Embedding(64, args.dshape)
        self.tag_in = nn.Embedding(len(vocab.tags) + 1, dc, padding_idx=0)
        self.tag_out = nn.Embedding(len(vocab.tags) + 1, dc, padding_idx=0)
        self.tag_bias = nn.Embedding(len(vocab.tags) + 1, 1, padding_idx=0)
        self.source = nn.Embedding(len(SOURCE), dc)
        self.rank = nn.Embedding(RANKS, dc)
        self.bucket = nn.Embedding(len(PROB_EDGES) + 1, dc)
        self.rank_bias = nn.Embedding(RANKS, 1)
        self.bucket_bias = nn.Embedding(len(PROB_EDGES) + 1, 1)
        self.input = nn.Linear(args.dshape + 2 * dc + args.dw + SUFFIX_MAX * args.ds, d)
        self.drop = nn.Dropout(args.dropout)
        self.lstm = nn.LSTM(
            d,
            d // 2,
            num_layers=args.layers,
            bidirectional=True,
            batch_first=True,
            dropout=args.dropout if args.layers > 1 else 0,
        )
        self.norm = nn.LayerNorm(d)
        self.query = nn.Linear(d, dc)

    def forward(self, x: dict[str, torch.Tensor]) -> torch.Tensor:
        cmask = x["cmask"].float()
        tags_in = self.tag_in(x["tags"]).sum(3)
        mean = (tags_in * cmask[..., None]).sum(2) / cmask.sum(2, keepdim=True).clamp(min=1)
        weighted = (tags_in * x["prob"][..., None]).sum(2)
        parts = [
            self.shape(x["shape"]),
            mean,
            weighted,
            self.word(x["word"]),
            self.suffix(x["suffix"]).flatten(2),
        ]
        h = self.drop(torch.relu(self.input(torch.cat(parts, -1)))) * x["mask"].float()[..., None]
        lengths = x["mask"].sum(1).cpu()
        packed = nn.utils.rnn.pack_padded_sequence(
            h, lengths, batch_first=True, enforce_sorted=False
        )
        out, _ = self.lstm(packed)
        h, _ = nn.utils.rnn.pad_packed_sequence(out, batch_first=True, total_length=h.shape[1])
        q = self.query(self.norm(self.drop(h)))
        c = (
            self.tag_out(x["tags"]).sum(3)
            + self.source(x["source"])
            + self.rank(x["rank"])
            + self.bucket(x["bucket"])
        )
        bias = (
            self.tag_bias(x["tags"]).sum(3)[..., 0]
            + self.rank_bias(x["rank"])[..., 0]
            + self.bucket_bias(x["bucket"])[..., 0]
        )
        scores = (q[:, :, None, :] * c).sum(-1) + bias
        return scores.masked_fill(~x["cmask"], -1e9)


def batches(
    f: dict[str, np.ndarray], budget: int, rng: np.random.Generator | None
) -> list[np.ndarray]:
    """
    Пакеты предложений близкой длины не больше `budget` токенов с выравниванием

    Аргументы:
        f (dict[str, np.ndarray]): Признаки
        budget (int): Токенов в пакете
        rng (np.random.Generator | None): Случайный порядок для обучения, None - по длине

    Вывод:
        list[np.ndarray]: Номера предложений пакетов
    """
    lengths = np.diff(f["sent_off"])
    order = (
        np.lexsort((rng.random(len(lengths)), lengths // 4))
        if rng
        else np.argsort(lengths, kind="stable")
    )
    out, current, longest = [], [], 0
    for s in order:
        longest = max(longest, lengths[s])
        if current and longest * (len(current) + 1) > budget:
            out.append(np.array(current))
            current, longest = [], lengths[s]
        current.append(s)
    if current:
        out.append(np.array(current))
    if rng:
        rng.shuffle(out)
    return out


def batch(
    f: dict[str, np.ndarray],
    sents: np.ndarray,
    device: torch.device,
    word_drop: float = 0.0,
    rng=None,
):
    """
    Тензоры пакета: токены [B, L], разборы [B, L, K]

    Аргументы:
        f (dict[str, np.ndarray]): Признаки
        sents (np.ndarray): Номера предложений
        device (torch.device): Устройство
        word_drop (float): Доля слов, заменяемых неизвестным (обучение)
        rng (np.random.Generator | None): Генератор для `word_drop`

    Вывод:
        tuple: Тензоры, номера токенов [B, L] и разборов [B, L, K], маска разборов
    """
    so = f["sent_off"]
    lengths = so[sents + 1] - so[sents]
    tok = np.full((len(sents), lengths.max()), -1)
    for b, (s, n) in enumerate(zip(sents, lengths, strict=True)):
        tok[b, :n] = np.arange(so[s], so[s] + n)
    mask = tok >= 0
    t = tok.clip(min=0)
    off = f["off"]
    count = np.where(mask, off[t + 1] - off[t], 0)
    k = max(count.max(), 1)
    cmask = np.arange(k) < count[..., None]
    c = np.where(cmask, off[t][..., None] + np.arange(k), 0)
    word = f["word"][t].copy()
    if word_drop:
        word[rng.random(word.shape) < word_drop] = 1
    prob = np.where(cmask, f["prob"][c], 0)
    prob = prob / np.maximum(prob.sum(-1, keepdims=True), 1e-9)
    arrays = {
        "mask": mask,
        "word": word,
        "suffix": f["suffix"][t],
        "shape": f["shape"][t],
        "cmask": cmask,
        "tags": f["tags"][c],
        "rank": f["rank"][c],
        "bucket": f["bucket"][c],
        "source": f["source"][c],
        "prob": prob.astype(np.float32),
        "gold": f["gold"][c] & cmask,
    }
    tensors = {
        key: torch.from_numpy(np.ascontiguousarray(value)).to(device)
        for key, value in arrays.items()
    }
    return tensors, tok, c, cmask


@torch.no_grad()
def predict(model: Model, f: dict[str, np.ndarray], device: torch.device) -> np.ndarray:
    """
    Выбранный разбор каждого токена: номер строки разбора в признаках

    Аргументы:
        model (Model): Модель
        f (dict[str, np.ndarray]): Признаки
        device (torch.device): Устройство

    Вывод:
        np.ndarray: Номера разборов
    """
    model.eval()
    chosen = np.zeros(len(f["word"]), dtype=np.int64)
    for sents in batches(f, 4000, None):
        x, tok, c, _ = batch(f, sents, device)
        best = model(x).argmax(-1).cpu().numpy()
        valid = tok >= 0
        chosen[tok[valid]] = np.take_along_axis(c, best[..., None], -1)[..., 0][valid]
    return chosen


def metrics(data: Data, f: dict[str, np.ndarray], chosen: np.ndarray) -> dict[str, float]:
    """
    Точность выбранных разборов: часть речи, лемма, падеж, основные признаки, full core

    Аргументы:
        data (Data): Токены и разборы
        f (dict[str, np.ndarray]): Признаки
        chosen (np.ndarray): Выбранные разборы

    Вывод:
        dict[str, float]: Проценты
    """
    flat = [b for row in data.bits for b in row]
    bits = np.array([flat[c] for c in chosen])[data.evaluated]
    has_case = (bits & 16) > 0
    return {
        "upos": 100 * (bits & 1).astype(bool).mean(),
        "lemma": 100 * (bits & 2).astype(bool).mean(),
        "case": 100 * ((bits & 8) > 0).sum() / max(has_case.sum(), 1),
        "feats": 100 * (bits & 4).astype(bool).mean(),
        "full": 100 * ((bits & 7) == 7).mean(),
    }


def train(
    model: Model, f: dict, dev: dict, args: argparse.Namespace, device: torch.device
) -> None:
    """
    Обучение с выбором эпохи по среднему full core проверочных частей

    Аргументы:
        model (Model): Модель
        f (dict): Признаки обучающих данных
        dev (dict): Проверочные части: имя - (данные, признаки)
        args (argparse.Namespace): Параметры
        device (torch.device): Устройство
    """
    rng = np.random.default_rng(args.seed)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=1e-5)
    total = max(1, args.epochs * len(batches(f, args.batch_tokens, rng)))
    sched = torch.optim.lr_scheduler.LambdaLR(
        opt, lambda s: min(1.0, s / 300) * 0.5 * (1 + math.cos(math.pi * min(s, total) / total))
    )
    best, best_state = -1.0, None
    for epoch in range(args.epochs):
        model.train()
        started, loss_sum, steps = time.time(), 0.0, 0
        for sents in batches(f, args.batch_tokens, rng):
            x, *_ = batch(f, sents, device, args.word_drop, rng)
            gold = x["gold"]
            has = gold.any(-1) & (x["cmask"].sum(-1) > 1)
            if not has.any():
                continue
            logp = model(x).log_softmax(-1)
            # Несколько верных разборов - их вероятности складываются
            loss = -(logp.masked_fill(~gold, -1e9).logsumexp(-1))[has].mean()
            opt.zero_grad()
            loss.backward()
            nn.utils.clip_grad_norm_(model.parameters(), 5.0)
            opt.step()
            sched.step()
            loss_sum, steps = loss_sum + loss.item(), steps + 1
        scores = {
            name: metrics(data, df, predict(model, df, device)) for name, (data, df) in dev.items()
        }
        selected = sum(s["full"] for s in scores.values()) / len(scores)
        print(
            f"эпоха {epoch + 1}: потери {loss_sum / steps:.4f}, {time.time() - started:.0f} с, "
            + ", ".join(f"{name} full {s['full']:.2f}" for name, s in scores.items()),
            flush=True,
        )
        if selected > best:
            best = selected
            best_state = {key: value.detach().clone() for key, value in model.state_dict().items()}
    model.load_state_dict(best_state)


def quantize(weight: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """
    Симметричное квантование int8 по строкам

    Аргументы:
        weight (np.ndarray): Матрица

    Вывод:
        tuple[np.ndarray, np.ndarray]: Масштабы строк (f32) и целые int8
    """
    scale = (np.abs(weight).max(1) / 127).astype(np.float32)
    q = np.clip(np.round(weight / np.where(scale > 0, scale, 1)[:, None]), -127, 127).astype(
        np.int8
    )
    return scale, q


def tensors(model: Model, args: argparse.Namespace) -> dict[str, np.ndarray]:
    """
    Тензоры файла весов в порядке записи: смещения LSTM сложены, матрицы - [выход, вход]

    Аргументы:
        model (Model): Модель
        args (argparse.Namespace): Параметры

    Вывод:
        dict[str, np.ndarray]: Имя - матрица f32
    """
    state = {key: value.float().cpu().numpy() for key, value in model.state_dict().items()}
    out = {
        "word": state["word.weight"],
        "suffix": state["suffix.weight"],
        "shape": state["shape.weight"],
        "tag_in": state["tag_in.weight"],
        "tag_out": state["tag_out.weight"],
        "tag_bias": state["tag_bias.weight"],
        "source": state["source.weight"],
        "rank": state["rank.weight"],
        "bucket": state["bucket.weight"],
        "rank_bias": state["rank_bias.weight"],
        "bucket_bias": state["bucket_bias.weight"],
        "input.weight": state["input.weight"],
        "input.bias": state["input.bias"][None],
    }
    for layer in range(args.layers):
        for direction, suffix in (("forward", ""), ("backward", "_reverse")):
            prefix = f"lstm.{layer}.{direction}"
            out[f"{prefix}.ih"] = state[f"lstm.weight_ih_l{layer}{suffix}"]
            out[f"{prefix}.hh"] = state[f"lstm.weight_hh_l{layer}{suffix}"]
            out[f"{prefix}.bias"] = (
                state[f"lstm.bias_ih_l{layer}{suffix}"] + state[f"lstm.bias_hh_l{layer}{suffix}"]
            )[None]
    out.update({"norm.weight": state["norm.weight"][None], "norm.bias": state["norm.bias"][None]})
    out.update({"query.weight": state["query.weight"], "query.bias": state["query.bias"][None]})
    return out


def quantized(array: np.ndarray) -> bool:
    """Матрицы от 4096 чисел хранятся в int8, остальное - в f32"""
    return array.shape[0] > 1 and array.size >= 4096


def write(
    path: Path, vocab: Vocabulary, weights: dict[str, np.ndarray], args: argparse.Namespace
) -> None:
    """
    Файл весов: сигнатура, версия, число записей, затем записи «имя, вид, данные», числа
    little-endian. Вид 0 - строки, 1 - матрица f32, 2 - матрица int8 с масштабом f32 на строку,
    3 - числа u32, 4 - числа f64

    Аргументы:
        path (Path): Файл
        vocab (Vocabulary): Таблицы строк
        weights (dict[str, np.ndarray]): Тензоры
        args (argparse.Namespace): Параметры
    """
    entries = []

    def name(text: str) -> bytes:
        data = text.encode()
        return struct.pack("<H", len(data)) + data

    config = [args.d, args.layers, args.dw, args.ds, args.dc, args.dshape, KMAX, RANKS, SUFFIX_MAX]
    entries.append(
        name("config")
        + struct.pack("<BI", 3, len(config))
        + struct.pack(f"<{len(config)}I", *config)
    )
    edges = np.array(PROB_EDGES, dtype="<f8")
    entries.append(name("edges") + struct.pack("<BI", 4, len(edges)) + edges.tobytes())
    for key, strings in (
        ("words", vocab.words),
        ("suffixes", vocab.suffixes),
        ("tags", vocab.tags),
        ("inventory", vocab.inventory),
    ):
        body = struct.pack("<BI", 0, len(strings))
        for text in strings:
            data = text.encode()
            body += struct.pack("<I", len(data)) + data
        entries.append(name(key) + body)
    for key, array in weights.items():
        rows, cols = array.shape
        if quantized(array):
            scale, q = quantize(array)
            entries.append(
                name(key)
                + struct.pack("<BII", 2, rows, cols)
                + scale.astype("<f4").tobytes()
                + q.tobytes()
            )
        else:
            entries.append(
                name(key) + struct.pack("<BII", 1, rows, cols) + array.astype("<f4").tobytes()
            )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(MAGIC + struct.pack("<II", FORMAT, len(entries)) + b"".join(entries))


def quantize_model(model: Model) -> None:
    """
    Замена весов модели квантованными, как в файле весов: точность модели из файла и запись
    без второй ошибки квантования

    Аргументы:
        model (Model): Модель
    """
    with torch.no_grad():
        for parameter in model.parameters():
            array = parameter.detach().float().cpu().numpy()
            if array.ndim == 2 and quantized(array):
                scale, q = quantize(array)
                parameter.copy_(torch.from_numpy(q.astype(np.float32) * scale[:, None]))


def main() -> None:
    parser = argparse.ArgumentParser(description="Обучение контекстной модели ruMorph")
    parser.add_argument(
        "--dictionary",
        type=Path,
        required=True,
        help="файл словаря, кандидаты которого учит модель",
    )
    parser.add_argument(
        "--data", type=Path, default=Path("target/context"), help="каталог данных UD"
    )
    parser.add_argument(
        "--output", type=Path, default=Path("data/ru-context.bin"), help="файл весов"
    )
    parser.add_argument("--d", type=int, default=128)
    parser.add_argument("--layers", type=int, default=1)
    parser.add_argument("--dw", type=int, default=32)
    parser.add_argument("--ds", type=int, default=16)
    parser.add_argument("--dc", type=int, default=32)
    parser.add_argument("--dshape", type=int, default=16)
    parser.add_argument("--words", type=int, default=8000)
    parser.add_argument("--suffixes", type=int, default=4000)
    parser.add_argument("--dropout", type=float, default=0.2)
    parser.add_argument("--word-drop", type=float, default=0.1)
    parser.add_argument("--epochs", type=int, default=12)
    parser.add_argument("--lr", type=float, default=2e-3)
    parser.add_argument("--batch-tokens", type=int, default=4000)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    # Разметку корпуса OpenCorpora переводит rumorph-build: без cargo обучение упало бы после загрузок
    if shutil.which("cargo") is None:
        parser.error("нужен cargo: теги корпуса OpenCorpora переводит rumorph-build")
    torch.manual_seed(args.seed)
    device = torch.device("mps" if torch.backends.mps.is_available() else "cpu")

    for treebank, (commit, files) in SOURCES.items():
        for file, digest in files.items():
            url = f"https://raw.githubusercontent.com/UniversalDependencies/{treebank}/{commit}/{file}"
            download([url], args.data / file, digest)
    corpus_file, urls, digest = OPENCORPORA
    download(urls, args.data / corpus_file, digest)
    morph = Analyzer(args.dictionary)
    inventory = sorted(
        {name for pos, feats in morph._ud_tags() for name in tag_names(pos, feats)[0]}
    )
    started = time.time()
    corpus_sents = opencorpora(args.data / corpus_file)
    pairs = {pair for sent in corpus_sents for _, variants in sent for pair in variants}
    pairs |= {(tag, "быть") for tag, lemma in pairs if lemma == "есть"}
    corpus = Corpus(corpus_sents, morph, derived(pairs))
    train_data = Merged([Data(sentences([args.data / file for file in TRAIN]), morph), corpus])
    dev_data = {
        name: Data(sentences([args.data / file for file in files]), morph)
        for name, files in DEV.items()
    }
    vocab = Vocabulary(train_data, args.words, args.suffixes, inventory)
    f = featurize(train_data, vocab)
    dev = {name: (data, featurize(data, vocab)) for name, data in dev_data.items()}
    print(
        f"данные: {len(train_data.forms)} токенов train (OpenCorpora {len(corpus.forms)}), {len(vocab.words)} слов, {len(vocab.suffixes)} окончаний, {len(vocab.tags)} тегов, {time.time() - started:.0f} с"
    )

    model = Model(vocab, args).to(device)
    train(model, f, dev, args, device)
    quantize_model(model)
    write(args.output, vocab, tensors(model, args), args)
    for name, (data, df) in dev.items():
        result = metrics(data, df, predict(model, df, device))
        print(
            f"{name} dev (int8): "
            + "  ".join(f"{key} {value:.2f}" for key, value in result.items())
        )
    print(f"{args.output}: {args.output.stat().st_size / 1e6:.2f} МБ")


if __name__ == "__main__":
    main()
