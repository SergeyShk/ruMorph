"""
Картинки документации: как ruMorph разбирает предложение

Описание:
    Рисует по настоящему разбору словаря и контекстной модели пакета: конвейер «предложение →
    разборы словаря → выбор по соседним словам» (docs/img/pipeline.svg), устройство
    контекстной модели с размерами из файла весов (docs/img/model.svg) и вероятности разборов
    неоднозначного слова «стали» без контекста и в двух предложениях (docs/img/choice.svg);
    к каждой - английская версия (*.en.svg). Вероятности в контексте считает прямой проход
    модели на numpy по файлу весов (data/ru-context.bin), сверенный с разбором ядра на Rust.
    Запуск - в окружении проекта: нужен собранный rumorph, а numpy подключается на время запуска.

Пример использования:
    uv run --with numpy python scripts/figures.py
"""

import json
import struct
from pathlib import Path

import numpy as np

from rumorph import Analysis, Analyzer

ROOT = Path(__file__).parents[1]
MODEL = ROOT / "data" / "ru-context.bin"
PARADIGMS = ROOT / "data" / "ru-paradigms.bin"
IMG = ROOT / "docs" / "img"
EXAMPLE = ["Мы", "стали", "друзьями", "."]
OTHER = ["Мост", "сделан", "из", "стали", "."]
WORD = "стали"
UNKNOWN = "ноотропил"
# Без модели разборы берутся из стольких первых форм списка самого длинного окончания
MAX_COUNTED = 10


def read_weights(path: Path, magic: bytes = b"RUMCTX\0\0") -> dict:
    """
    Записи файла весов модели ruMorph: строки, матрицы (int8 по строкам - деквантованные)
    и числа

    Аргументы:
        path (Path): Файл весов
        magic (bytes): Сигнатура: контекстной модели или модели класса парадигмы

    Вывод:
        dict: Записи по именам

    Исключения:
        ValueError: Если у файла другая сигнатура
    """
    data = path.read_bytes()
    if data[:8] != magic:
        raise ValueError(f"{path}: не веса модели ruMorph с сигнатурой {magic!r}")
    count = struct.unpack_from("<I", data, 12)[0]
    at, entries = 16, {}
    for _ in range(count):
        (length,) = struct.unpack_from("<H", data, at)
        name = data[at + 2 : at + 2 + length].decode()
        at += 2 + length
        kind = data[at]
        at += 1
        if kind == 0:
            (size,) = struct.unpack_from("<I", data, at)
            at += 4
            items = []
            for _ in range(size):
                (n,) = struct.unpack_from("<I", data, at)
                items.append(data[at + 4 : at + 4 + n].decode())
                at += 4 + n
            entries[name] = items
        elif kind in (1, 2):
            rows, cols = struct.unpack_from("<II", data, at)
            at += 8
            if kind == 1:
                matrix = np.frombuffer(data, "<f4", rows * cols, at).reshape(rows, cols)
                at += 4 * rows * cols
            else:
                scales = np.frombuffer(data, "<f4", rows, at)
                at += 4 * rows
                values = np.frombuffer(data, "i1", rows * cols, at).reshape(rows, cols)
                at += rows * cols
                matrix = values.astype(np.float32) * scales[:, None]
            entries[name] = matrix.astype(np.float32)
        elif kind == 3:
            (size,) = struct.unpack_from("<I", data, at)
            entries[name] = list(struct.unpack_from(f"<{size}I", data, at + 4))
            at += 4 + 4 * size
        else:
            (size,) = struct.unpack_from("<I", data, at)
            entries[name] = list(struct.unpack_from(f"<{size}d", data, at + 4))
            at += 4 + 8 * size
    return entries


def shape(token: str, first: bool) -> int:
    """Запись токена, как в ядре: знаки, число, регистр букв, латиница, начало предложения, дефис"""
    digits = sum(ch.isascii() and ch.isdigit() for ch in token)
    letters = [ch for ch in token if ch.isalpha()]
    if not digits and not letters:
        kind = 0
    elif digits:
        kind = 1 if digits == len(token) else 2
    else:
        if all(not ch.isupper() for ch in letters):
            kind = 3
        elif all(not ch.islower() for ch in letters) and len(letters) > 1:
            kind = 4
        elif letters[0].isupper() and all(not ch.isupper() for ch in letters[1:]):
            kind = 5
        else:
            kind = 6
        kind += 4 * any(ch.lower().isascii() and ch.lower().isalpha() for ch in letters)
    return kind * 4 + 2 * first + ("-" in token)


class Numpy:
    """
    Прямой проход контекстной модели на numpy до вероятностей разборов - для картинок

    Аргументы:
        path (Path): Файл весов
    """

    def __init__(self, path: Path) -> None:
        self.e = read_weights(path)
        keys = ("d", "layers", "dw", "ds", "dc", "dshape", "kmax", "ranks", "suffix_max")
        self.config = dict(zip(keys, self.e["config"], strict=True))
        self.words = {text: i + 2 for i, text in enumerate(self.e["words"])}
        self.suffixes = {text: i + 2 for i, text in enumerate(self.e["suffixes"])}
        self.tags = {text: i + 1 for i, text in enumerate(self.e["tags"])}

    def tag(self, table: str, analysis: Analysis) -> np.ndarray:
        """Сумма строк ключей тега разбора: часть речи, каждый признак и их сочетание"""
        feats = sorted(analysis.feats.items())
        keys = [f"P={analysis.pos}"] + [f"F={name}={value}" for name, value in feats]
        keys.append("C=" + "|".join([analysis.pos] + [f"{name}={value}" for name, value in feats]))
        rows = [self.tags[key] for key in keys if key in self.tags]
        return self.e[table][rows].sum(0) if rows else np.zeros(self.e[table].shape[1], np.float32)

    def lstm(self, x: np.ndarray, prefix: str, reverse: bool) -> np.ndarray:
        ih, hh, bias = (self.e[f"{prefix}.{name}"] for name in ("ih", "hh", "bias"))
        hidden = hh.shape[1]
        h, c = np.zeros(hidden, np.float32), np.zeros(hidden, np.float32)
        out = np.zeros((len(x), hidden), np.float32)
        for t in range(len(x) - 1, -1, -1) if reverse else range(len(x)):
            i, f, g, o = np.split(ih @ x[t] + bias[0] + hh @ h, 4)
            c = 1 / (1 + np.exp(-f)) * c + 1 / (1 + np.exp(-i)) * np.tanh(g)
            h = 1 / (1 + np.exp(-o)) * np.tanh(c)
            out[t] = h
        return out

    def probabilities(self, words: list[str], analyses: list[list[Analysis]]) -> list[np.ndarray]:
        """
        Вероятности разборов каждого слова предложения: модель видит первые kmax разборов

        Аргументы:
            words (list[str]): Слова
            analyses (list[list[Analysis]]): Разборы словаря каждого слова

        Вывод:
            list[np.ndarray]: Вероятности разборов слов
        """
        c = self.config
        rows, seen_all = [], []
        for t, (word, found) in enumerate(zip(words, analyses, strict=True)):
            seen = found[: c["kmax"]]
            total = max(sum(a.prob for a in seen), 1e-9)
            tag_in = [self.tag("tag_in", a) for a in seen]
            mean = np.mean(tag_in, 0)
            weighted = sum(
                v * np.float32(a.prob / total) for v, a in zip(tag_in, seen, strict=True)
            )
            normal = word.lower().replace("ё", "е")
            marked = "^" + normal
            suffixes = [
                self.e["suffix"][self.suffixes.get(f"{n}{marked[-n:]}", 1)]
                for n in range(1, c["suffix_max"] + 1)
            ]
            rows.append(
                np.concatenate(
                    [
                        self.e["shape"][shape(word, t == 0)],
                        mean,
                        weighted,
                        self.e["word"][self.words.get(normal, 1)],
                        *suffixes,
                    ]
                )
            )
            seen_all.append(seen)
        x = np.maximum(np.stack(rows) @ self.e["input.weight"].T + self.e["input.bias"][0], 0)
        for layer in range(c["layers"]):
            x = np.concatenate(
                [
                    self.lstm(x, f"lstm.{layer}.forward", False),
                    self.lstm(x, f"lstm.{layer}.backward", True),
                ],
                1,
            )
        mean, var = x.mean(1, keepdims=True), x.var(1, keepdims=True)
        x = (x - mean) / np.sqrt(var + 1e-5) * self.e["norm.weight"][0] + self.e["norm.bias"][0]
        queries = x @ self.e["query.weight"].T + self.e["query.bias"][0]
        result = []
        for query, seen in zip(queries, seen_all, strict=True):
            order = sorted(range(len(seen)), key=lambda k: -seen[k].prob)
            ranks = [0] * len(seen)
            for place, k in enumerate(order):
                ranks[k] = min(place, c["ranks"] - 1)
            source = {"dictionary": 0, "rule": 1, "guess": 2}
            scores = []
            for a, rank in zip(seen, ranks, strict=True):
                bucket = sum(edge < a.prob for edge in self.e["edges"])
                scores.append(
                    query @ self.tag("tag_out", a)
                    + query @ self.e["source"][source[a.source]]
                    + query @ self.e["rank"][rank]
                    + query @ self.e["bucket"][bucket]
                    + self.tag("tag_bias", a)[0]
                    + self.e["rank_bias"][rank][0]
                    + self.e["bucket_bias"][bucket][0]
                )
            scores = np.array(scores) - max(scores)
            result.append(np.exp(scores) / np.exp(scores).sum())
        return result


class Paradigms:
    """
    Прямой проход модели класса парадигмы на numpy до вероятностей кандидатов - для картинок

    Аргументы:
        path (Path): Файл весов
        tags (list): Части речи и признаки тегов UD словаря (`Analyzer._ud_tags`)
    """

    def __init__(self, path: Path, tags: list) -> None:
        self.e = read_weights(path, b"RUMPAR\0\0")
        self.config = dict(
            zip(("dim", "hidden", "dchar", "max_chars"), self.e["config"], strict=True)
        )
        self.chars = {text: i + 2 for i, text in enumerate(self.e["chars"])}
        self.keys = {text: i + 1 for i, text in enumerate(self.e["keys"])}
        self.suffixes = {text: i + 1 for i, text in enumerate(self.e["suffixes"])}
        self.tags = tags

    def word(self, word: str) -> np.ndarray:
        """Вектор слова и единица для смещений: LSTM читает последние буквы с конца"""
        letters = word.lower()[::-1][: self.config["max_chars"]].replace("ё", "е")
        x = self.e["char"][[self.chars.get(ch, 1) for ch in letters]]
        ih, hh, bias = (self.e[f"lstm.{name}"] for name in ("ih", "hh", "bias"))
        h = np.zeros(hh.shape[1], np.float32)
        c = np.zeros(hh.shape[1], np.float32)
        for row in x:
            i, f, g, o = np.split(ih @ row + bias[0] + hh @ h, 4)
            c = 1 / (1 + np.exp(-f)) * c + 1 / (1 + np.exp(-i)) * np.tanh(g)
            h = 1 / (1 + np.exp(-o)) * np.tanh(c)
        vector = np.tanh(self.e["hidden.weight"] @ h + self.e["hidden.bias"][0])
        return np.append(vector, 1.0)

    def probabilities(self, word: str, candidates: list) -> np.ndarray:
        """
        Вероятности кандидатов незнакомого слова (`Analyzer._guess_examples`)

        Аргументы:
            word (str): Слово
            candidates (list): Кандидаты: лемма, тег UD, суффиксы формы леммы и формы, длина
                окончания, число словоформ, доля в списке, верен ли

        Вывод:
            np.ndarray: Вероятности кандидатов
        """
        vector = self.word(word)
        longest = max(candidate[4] for candidate in candidates)
        scores = []
        for _, ud, lemma_suffix, form_suffix, length, count, share, _ in candidates:
            pos, feats = self.tags[ud]
            keys = [f"P={pos}"] + [f"F={name}={value}" for name, value in sorted(feats.items())]
            e = self.e["key"][[self.keys[key] for key in keys if key in self.keys]].sum(0)
            e = e + self.e["lemma_suffix"][self.suffixes.get(lemma_suffix, 0)]
            e = e + self.e["form_suffix"][self.suffixes.get(form_suffix, 0)]
            e = e + self.e["length"][length - 1]
            numeric = np.array([np.log1p(count), share, float(length == longest)], np.float32)
            scores.append(vector @ e + numeric @ self.e["numeric"][0])
        scores = np.array(scores) - max(scores)
        return np.exp(scores) / np.exp(scores).sum()


DARK, LIGHT, INK, MUTED, SOFT = "#1976d2", "#64b5f6", "#263238", "#78909c", "#e3f2fd"
FONT = "PT Sans, Source Sans Pro, Arial, sans-serif"
TEXTS = {
    "ru": {
        "pos": {
            "NOUN": "сущ.",
            "VERB": "глаг.",
            "ADJ": "прил.",
            "ADV": "нареч.",
            "ADP": "предлог",
            "PRON": "мест.",
            "PUNCT": "знак",
            "PROPN": "имя",
        },
        "case": {
            "Nom": "им. п.",
            "Gen": "род. п.",
            "Dat": "дат. п.",
            "Acc": "вин. п.",
            "Ins": "твор. п.",
            "Loc": "пр. п.",
        },
        "number": {"Sing": "ед. ч.", "Plur": "мн. ч."},
        "past": "прош. вр.",
        "steps": (
            "1. Предложение",
            "2. Словарь дает все разборы каждого слова и их частоты",
            "3. Контекстная модель выбирает разбор по соседним словам",
        ),
        "context": (
            ("запись", "слово", "окончания 1-{suffix_max}", "разборы слова"),
            "вектор слова: {din}",
            "линейный слой и ReLU: {d}",
            "BiLSTM {layers} × {half}, в обе стороны: {d} на слово",
            "нормализация и вектор запроса: {dc}",
            "оценка разбора: запрос · (тег + источник + ранг + доля) + смещения",
            "softmax по разборам слова: P(разбор | предложение)",
            "первые {kmax} разборов слова · {params} млн параметров, int8 по строкам",
        ),
        "paradigm": (
            ("буквы слова с конца", "{max_chars} × {dchar}"),
            ("кандидаты: формы из списков окончаний 1-5", "до 30 форм на окончание"),
            "LSTM {hidden}",
            (
                ("тег", "UD"),
                ("окончание", "леммы"),
                ("окончание", "формы"),
                ("длина", "окончания"),
            ),
            "вектор слова: {dim}",
            "вектор кандидата: {dim}",
            "оценка: вектор слова · вектор кандидата + число и доля формы в списке",
            "softmax по кандидатам: P(форма | слово)",
            "вес лексемы делится между ее совпадающими формами по числу словоформ",
            "до 10 разборов с вероятностью не меньше 1 % от лучшего",
            "{params} млн параметров, int8 по строкам · {keys} признаков тегов, {suffixes} окончаний",
        ),
        "guess": (
            "Незнакомое слово «{word}»",
            "по частоте окончания",
            "модель",
            (
                "Доли разборов с той же леммой и частью речи: по частоте окончания - как без модели",
                "(самое длинное окончание), модель - прямой проход по файлу весов",
            ),
        ),
        "choice": (
            "Разбор слова «стали»",
            "без контекста",
            "Мы стали друзьями.",
            "Мост сделан из стали.",
            "Вероятности: без контекста - частоты словаря, в предложении - контекстная модель",
        ),
    },
    "en": {
        "pos": {
            "NOUN": "noun",
            "VERB": "verb",
            "ADJ": "adj.",
            "ADV": "adv.",
            "ADP": "prep.",
            "PRON": "pron.",
            "PUNCT": "punct.",
            "PROPN": "name",
        },
        "case": {
            "Nom": "nom.",
            "Gen": "gen.",
            "Dat": "dat.",
            "Acc": "acc.",
            "Ins": "ins.",
            "Loc": "loc.",
        },
        "number": {"Sing": "sing.", "Plur": "plur."},
        "past": "past",
        "steps": (
            "1. Sentence",
            "2. The dictionary gives every analysis of every word and its frequency",
            "3. The context model chooses the analysis by the neighbouring words",
        ),
        "context": (
            ("shape", "word", "endings 1-{suffix_max}", "analyses of the word"),
            "word vector: {din}",
            "linear layer and ReLU: {d}",
            "BiLSTM {layers} × {half}, both directions: {d} per word",
            "normalization and query vector: {dc}",
            "analysis score: query · (tag + source + rank + share) + biases",
            "softmax over the analyses of the word: P(analysis | sentence)",
            "the first {kmax} analyses of a word · {params} million parameters, int8 by rows",
        ),
        "paradigm": (
            ("the letters of the word from the end", "{max_chars} × {dchar}"),
            ("candidates: forms from the lists of endings 1-5", "up to 30 forms per ending"),
            "LSTM {hidden}",
            (("UD", "tag"), ("lemma", "ending"), ("form", "ending"), ("ending", "length")),
            "word vector: {dim}",
            "candidate vector: {dim}",
            "score: word vector · candidate vector + count and share of the form in the list",
            "softmax over the candidates: P(form | word)",
            "the weight of a lexeme is split among its identical forms by the counts",
            "up to 10 analyses with at least 1 % of the probability of the best",
            "{params} million parameters, int8 by rows · {keys} tag features, {suffixes} endings",
        ),
        "guess": (
            "The unknown word «{word}»",
            "by the ending",
            "model",
            (
                "Shares of the analyses with the same lemma and part of speech: by the ending - as",
                "without the model (the longest ending), model - a forward pass over the weights file",
            ),
        ),
        "choice": (
            "Analyses of «стали»",
            "no context",
            "Мы стали друзьями.",
            "Мост сделан из стали.",
            "Probabilities: without context - dictionary frequencies, in a sentence - the context "
            "model",
        ),
    },
}


def width(text: str, size: float) -> float:
    return len(text) * size * 0.56 + 6


def text(
    x: float,
    y: float,
    value: str,
    size: float = 16,
    color: str = INK,
    bold: bool = False,
    anchor: str = "middle",
) -> str:
    weight = ' font-weight="bold"' if bold else ""
    return (
        f'<text x="{x:.1f}" y="{y:.1f}" font-size="{size}" fill="{color}"{weight} '
        f'text-anchor="{anchor}">{value}</text>'
    )


def arrow(x: float, y: float, size: float = 7, color: str = DARK) -> str:
    return (
        f'<path d="M{x - size:.1f} {y - 1.6 * size:.1f}L{x + size:.1f} {y - 1.6 * size:.1f}'
        f'L{x:.1f} {y:.1f}Z" fill="{color}"/>'
    )


def down(x: float, y1: float, y2: float) -> list[str]:
    return [
        f'<path d="M{x:.1f} {y1:.1f}L{x:.1f} {y2 - 9:.1f}" stroke="{DARK}" stroke-width="2"/>',
        arrow(x, y2, 6),
    ]


def box(
    x: float,
    y: float,
    w: float,
    h: float,
    label: str,
    fill: str = SOFT,
    color: str = INK,
    size: float = 15,
) -> list[str]:
    return [
        f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="8" fill="{fill}" '
        f'stroke="{DARK}" stroke-width="1.5"/>',
        text(x + w / 2, y + h / 2 + size * 0.35, label, size, color),
    ]


def save(name: str, lang: str, w: float, h: float, body: list[str]) -> None:
    """Картинка с белой подложкой: в темной теме сайта она остается читаемой"""
    suffix = "" if lang == "ru" else ".en"
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.0f} {h:.0f}" width="{w:.0f}" '
        f'height="{h:.0f}" font-family="{FONT}" role="img">\n'
        f'<rect width="{w:.0f}" height="{h:.0f}" rx="12" fill="white"/>\n'
        + "\n".join(body)
        + "\n</svg>\n"
    )
    (IMG / f"{name}{suffix}.svg").write_text(svg, encoding="utf-8")


def describe(analysis: Analysis, lang: str) -> str:
    """Часть речи и главные признаки разбора школьными словами: «сущ., род. п., ед. ч.»"""
    t = TEXTS[lang]
    parts = [t["pos"].get(analysis.pos, analysis.pos)]
    feats = analysis.feats
    if "Case" in feats:
        parts.append(t["case"].get(feats["Case"], feats["Case"]))
    if feats.get("Tense") == "Past":
        parts.append(t["past"])
    if "Number" in feats:
        parts.append(t["number"][feats["Number"]])
    return ", ".join(parts)


def percent(value: float, lang: str) -> str:
    shown = f"{value:.2f}" if value >= 0.005 else "&lt;0.01"
    return shown.replace(".", "," if lang == "ru" else ".")


def draw_pipeline(
    words: list[str], dictionary: list[list[Analysis]], chosen: list[Analysis], lang: str
) -> None:
    steps = TEXTS[lang]["steps"]
    column, top = 200, 3
    xs = [20 + column * k + column / 2 for k in range(len(words))]
    body = [
        text(20, 30, steps[0], 15, DARK, True, "start"),
        text(20, 62, " ".join(words[:-1]) + words[-1], 22, INK, False, "start"),
        text(20, 104, steps[1], 15, DARK, True, "start"),
    ]
    rows = max(min(len(found), top) for found in dictionary)
    for x, word, found in zip(xs, words, dictionary, strict=True):
        body.append(text(x, 136, word, 19, INK, True))
        for k, analysis in enumerate(found[:top]):
            y = 150 + k * 48
            body.append(
                f'<rect x="{x - column / 2 + 8:.1f}" y="{y}" width="{column - 16}" height="42" '
                f'rx="6" fill="white" stroke="{LIGHT}" stroke-width="1.5"/>'
            )
            body.append(text(x, y + 18, f"{analysis.lemma} · {percent(analysis.prob, lang)}", 15))
            body.append(text(x, y + 35, describe(analysis, lang), 12, MUTED))
        if len(found) > top:
            body.append(text(x, 150 + top * 48 + 6, f"+ {len(found) - top}", 13, MUTED))
    y0 = 150 + rows * 48 + 40
    body.append(text(20, y0, steps[2], 15, DARK, True, "start"))
    for x, word, analysis in zip(xs, words, chosen, strict=True):
        body.append(text(x, y0 + 32, word, 19, INK, True))
        body.append(
            f'<rect x="{x - column / 2 + 8:.1f}" y="{y0 + 46}" width="{column - 16}" height="42" '
            f'rx="6" fill="{DARK}"/>'
        )
        body.append(text(x, y0 + 64, analysis.lemma, 15, "white", True))
        body.append(text(x, y0 + 81, describe(analysis, lang), 12, "white"))
    save("pipeline", lang, 40 + column * len(words), y0 + 110, body)


def draw_context(model: Numpy, lang: str, params: float) -> None:
    c = model.config
    t = TEXTS[lang]["context"]
    din = c["dshape"] + 2 * c["dc"] + c["dw"] + c["suffix_max"] * c["ds"]
    fmt = {
        "din": din,
        "d": c["d"],
        "half": c["d"] // 2,
        "layers": c["layers"],
        "dc": c["dc"],
        "kmax": c["kmax"],
        "suffix_max": c["suffix_max"],
        "params": f"{params:.1f}".replace(".", "," if lang == "ru" else "."),
    }
    body = []
    dims = (
        str(c["dshape"]),
        str(c["dw"]),
        f"{c['suffix_max']} × {c['ds']}",
        f"2 × {c['dc']}",
    )
    for k, (name, dim) in enumerate(zip(t[0], dims, strict=True)):
        x = 40 + k * 205
        body += box(x, 20, 185, 54, "", "white")
        body += [text(x + 92, 43, name.format(**fmt), 15), text(x + 92, 63, dim, 13, MUTED)]
        body += down(x + 92, 74, 104)
    y = 104
    for k, label in enumerate(t[1:7]):
        dark = k == 2
        body += box(
            40, y, 800, 44, label.format(**fmt), DARK if dark else SOFT, "white" if dark else INK
        )
        if k < 5:
            body += down(440, y + 44, y + 70)
        y += 70
    body.append(text(440, y + 14, t[7].format(**fmt), 13, MUTED))
    save("context", lang, 880, y + 34, body)


def draw_choice(
    candidates: list[Analysis], columns: list[np.ndarray], chosen: list[int], lang: str
) -> None:
    t = TEXTS[lang]["choice"]
    cell, left, topy = 150, 260, 92
    body = [text(30, 34, t[0], 17, DARK, True, "start")]
    for j, name in enumerate(t[1:4]):
        body.append(text(left + j * cell + cell / 2, topy - 14, name, 14, INK, True))
    for i, analysis in enumerate(candidates):
        y = topy + i * 52
        body.append(text(left - 14, y + 22, analysis.lemma, 16, INK, True, "end"))
        body.append(text(left - 14, y + 40, describe(analysis, lang), 12, MUTED, False, "end"))
        for j, probs in enumerate(columns):
            x = left + j * cell
            p = float(probs[i])
            body.append(
                f'<rect x="{x + 2}" y="{y + 2}" width="{cell - 4}" height="48" fill="{DARK}" '
                f'fill-opacity="{min(1.0, p) ** 0.6:.3f}"/>'
            )
            body.append(
                text(
                    x + cell / 2, y + 32, percent(p, lang), 16, "white" if p > 0.35 else INK, True
                )
            )
            if chosen[j] == i:
                body.append(
                    f'<rect x="{x + 4}" y="{y + 4}" width="{cell - 8}" height="44" fill="none" '
                    f'stroke="{INK}" stroke-width="3" rx="4"/>'
                )
    bottom = topy + len(candidates) * 52
    body.append(text(30, bottom + 30, t[4], 13, MUTED, False, "start"))
    save("choice", lang, left + 3 * cell + 30, bottom + 50, body)


def draw_paradigm(model: Paradigms, lang: str, params: float) -> None:
    c = model.config
    t = TEXTS[lang]["paradigm"]
    fmt = {
        **c,
        "params": f"{params:.1f}".replace(".", "," if lang == "ru" else "."),
        "keys": len(model.keys),
        "suffixes": f"{len(model.suffixes):,}".replace(",", " " if lang == "ru" else ","),
    }
    left, right, half = 40, 460, 380
    body = []
    for x, (name, dim) in ((left, t[0]), (right, t[1])):
        body += box(x, 20, half, 54, "", "white")
        body += [
            text(x + half / 2, 43, name.format(**fmt), 15),
            text(x + half / 2, 63, dim.format(**fmt), 13, MUTED),
        ]
        body += down(x + half / 2, 74, 104)
    body += box(left, 104, half, 44, t[2].format(**fmt), DARK, "white", 16)
    for k, (first, second) in enumerate(t[3]):
        x = right + k * 96
        body += box(x, 104, 92, 44, "")
        body += [text(x + 46, 122, first, 13), text(x + 46, 139, second, 13)]
        body += down(x + 46, 148, 174)
    body += down(left + half / 2, 148, 174)
    body += box(left, 174, half, 44, t[4].format(**fmt))
    body += box(right, 174, half, 44, t[5].format(**fmt))
    body += down(left + half / 2, 218, 244) + down(right + half / 2, 218, 244)
    y = 244
    for k, label in enumerate(t[6:10]):
        dark = k == 3
        body += box(
            left, y, 800, 44, label.format(**fmt), DARK if dark else SOFT, "white" if dark else INK
        )
        if k < 3:
            body += down(440, y + 44, y + 70)
        y += 70
    body.append(text(440, y + 14, t[10].format(**fmt), 13, MUTED))
    save("paradigm", lang, 880, y + 34, body)


def draw_guess(word: str, rows: list[tuple[str, str, float, float]], lang: str) -> None:
    t = TEXTS[lang]["guess"]
    cell, left, topy = 190, 270, 92
    body = [text(30, 34, t[0].format(word=word), 17, DARK, True, "start")]
    for j, name in enumerate(t[1:3]):
        body.append(text(left + j * cell + cell / 2, topy - 14, name, 14, INK, True))
    best = [max(range(len(rows)), key=lambda i: rows[i][2 + j]) for j in range(2)]
    for i, (lemma, pos, by_ending, by_model) in enumerate(rows):
        y = topy + i * 52
        body.append(text(left - 14, y + 22, lemma, 16, INK, True, "end"))
        body.append(
            text(left - 14, y + 40, TEXTS[lang]["pos"].get(pos, pos), 12, MUTED, False, "end")
        )
        for j, p in enumerate((by_ending, by_model)):
            x = left + j * cell
            body.append(
                f'<rect x="{x + 2}" y="{y + 2}" width="{cell - 4}" height="48" fill="{DARK}" '
                f'fill-opacity="{min(1.0, p) ** 0.6:.3f}"/>'
            )
            body.append(
                text(
                    x + cell / 2, y + 32, percent(p, lang), 16, "white" if p > 0.35 else INK, True
                )
            )
            if best[j] == i:
                body.append(
                    f'<rect x="{x + 4}" y="{y + 4}" width="{cell - 8}" height="44" fill="none" '
                    f'stroke="{INK}" stroke-width="3" rx="4"/>'
                )
    bottom = topy + len(rows) * 52
    for k, line in enumerate(t[3]):
        body.append(text(30, bottom + 30 + k * 18, line, 13, MUTED, False, "start"))
    save("guess", lang, left + 2 * cell + 30, bottom + 68, body)


def guess_rows(
    morph: Analyzer, model: Paradigms, word: str
) -> list[tuple[str, str, float, float]]:
    """
    Разборы незнакомого слова по лемме и части речи: доля по частоте самого длинного окончания
    и вероятность модели, сверенная с разбором ядра

    Аргументы:
        morph (Analyzer): Анализатор
        model (Paradigms): Модель на numpy
        word (str): Незнакомое слово

    Вывод:
        list[tuple[str, str, float, float]]: Лемма, часть речи, доля по окончанию, вероятность модели
    """
    candidates = morph._guess_examples(word)
    tags = morph._ud_tags()
    longest = max(candidate[4] for candidate in candidates)
    counted = [c for c in candidates if c[4] == longest][:MAX_COUNTED]
    total = sum(c[5] for c in counted)
    probs = model.probabilities(word, candidates)
    shares: dict[tuple[str, str], list[float]] = {}
    for candidate in counted:
        key = (candidate[0], tags[candidate[1]][0])
        shares.setdefault(key, [0.0, 0.0])[0] += candidate[5] / total
    for candidate, p in zip(candidates, probs, strict=True):
        key = (candidate[0], tags[candidate[1]][0])
        shares.setdefault(key, [0.0, 0.0])[1] += float(p)
    # Прямой проход на numpy выбирает те же лемму и часть речи, что ядро на Rust
    core: dict[tuple[str, str], float] = {}
    for analysis in morph.analyze(word):
        key = (analysis.lemma, analysis.pos)
        core[key] = core.get(key, 0.0) + analysis.prob
    top = max(shares, key=lambda key: shares[key][1])
    assert top == max(core, key=core.get), (top, core)
    assert abs(shares[top][1] - core[top]) < 0.05, (shares[top], core[top])
    rows = sorted(
        (item for item in shares.items() if max(item[1]) >= 0.005), key=lambda item: -max(item[1])
    )[:5]
    return [(lemma, pos, by_ending, by_model) for (lemma, pos), (by_ending, by_model) in rows]


def main() -> None:
    morph = Analyzer()
    model = Numpy(MODEL)
    params = sum(v.size for v in model.e.values() if isinstance(v, np.ndarray)) / 1e6
    columns, chosen, contexts = [], [], []
    candidates = morph.analyze(WORD)[: model.config["kmax"]]
    columns.append(np.array([a.prob for a in candidates]))
    chosen.append(0)
    for words in (EXAMPLE, OTHER):
        analyses = [morph.analyze(word) for word in words]
        probs = model.probabilities(words, analyses)
        rust = morph.analyze_sentence(words)
        # Прямой проход на numpy дает те же вероятности, что ядро на Rust
        for found, own, ranked in zip(analyses, probs, rust, strict=True):
            by_key = {(a.lemma, a.pos, tuple(sorted(a.feats.items()))): a.prob for a in ranked}
            core = [
                by_key[(a.lemma, a.pos, tuple(sorted(a.feats.items())))] for a in found[: len(own)]
            ]
            assert np.allclose(own, core, atol=1e-3), (own, core)
        at = words.index(WORD)
        column = probs[at]
        columns.append(column)
        chosen.append(int(np.argmax(column)))
        contexts.append((words, analyses, rust))
    paradigms = Paradigms(PARADIGMS, morph._ud_tags())
    paradigm_params = sum(v.size for v in paradigms.e.values() if isinstance(v, np.ndarray)) / 1e6
    rows = guess_rows(morph, paradigms, UNKNOWN)
    for lang in TEXTS:
        # Конвейер - на предложении, где модель меняет первый разбор словаря
        words, analyses, rust = contexts[1]
        draw_pipeline(words, analyses, [ranked[0] for ranked in rust], lang)
        draw_context(model, lang, params)
        draw_choice(candidates, columns, chosen, lang)
        draw_paradigm(paradigms, lang, paradigm_params)
        draw_guess(UNKNOWN, rows, lang)
    print(
        json.dumps(
            {
                "params_million": round(params, 2),
                "choice": [[round(float(p), 4) for p in column] for column in columns],
                "paradigm_params_million": round(paradigm_params, 2),
                "guess": [[lemma, pos, round(a, 3), round(b, 3)] for lemma, pos, a, b in rows],
            }
        )
    )


if __name__ == "__main__":
    main()
