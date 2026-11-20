# /// script
# requires-python = ">=3.11,<3.14"
# dependencies = [
#     "numpy==2.3.4",
#     "torch==2.14.1",
# ]
# ///
"""
Обучение модели класса парадигмы ruMorph: выбор формы таблицы словоизменения незнакомого слова

Описание:
    Модель выбирает разбор незнакомого нарицательного слова и учится на самом словаре. Для
    словарных слов выгрузки (`words.tsv`) анализатор отдает кандидатов угадывания среди
    нарицательных - формы таблиц из списков окончаний слова такими, как если бы слова не было
    в словаре, - и отмечает верные. Модель - посимвольный LSTM над словом с конца и оценка
    кандидата скалярным произведением с вектором из признаков тега UD, суффиксов формы леммы
    и самой формы и длины окончания, плюс числа словоформ списка окончания.
    Слова лексем с хэшем леммы 0 по модулю 50 - проверочные; эпоха выбирается по доле верных
    первых кандидатов на них. Веса квантуются в int8 по строкам и пишутся в файл `--output`,
    который `rumorph-build` вшивает в словарь; точность квантованной модели печатается. Версии
    пакетов закреплены в train_paradigms.py.lock; ruMorph - сборка проекта (`--with-editable .`),
    кандидаты которой должны совпадать со словарем.

Пример использования:
    uv run --with-editable . scripts/train_paradigms.py --dictionary python/rumorph/data/ru.dict \\
        --export target/opencorpora --output data/ru-paradigms.bin
"""

import argparse
import hashlib
import json
import math
import struct
import time
from array import array
from pathlib import Path

import numpy as np
import torch
from torch import nn

try:
    from rumorph import Analyzer
except ImportError:
    raise SystemExit(
        "нужна сборка ruMorph из репозитория: uv run --with-editable . scripts/train_paradigms.py ..."
    ) from None

MAGIC = b"RUMPAR\0\0"
FORMAT = 1
# Модель видит последние MAX_CHARS букв слова; угадываются слова от MIN_LENGTH букв кириллицей
MAX_CHARS = 16
MIN_LENGTH = 3
MAX_ENDING = 5
CYRILLIC = set("абвгдеёжзийклмнопрстуфхцчшщъыьэюя")


def chars_of(word: str) -> str:
    """Буквы входа модели: последние MAX_CHARS с конца слова, «ё» - как «е» (в текстах ее часто нет)"""
    return word[::-1][:MAX_CHARS].replace("ё", "е")


def tag_keys(pos: str, feats: dict[str, str]) -> list[str]:
    """Ключи признаков тега UD: часть речи и каждый признак"""
    return [f"P={pos}"] + [f"F={name}={value}" for name, value in sorted(feats.items())]


def examples_in(f: dict[str, np.ndarray]) -> int:
    return len(f["offsets"]) - 1


def percents(scores: tuple[float, float]) -> str:
    return f"тег {scores[0] * 100:.2f}, лемма и часть речи {scores[1] * 100:.2f}"


def dev_lemma(lemma: str) -> bool:
    return hashlib.md5(lemma.encode()).digest()[0] % 50 == 0


class Data:
    """Примеры: буквы слова, индекс угадывания и кандидаты подряд в массивах"""

    def __init__(self) -> None:
        self.chars = array("H")
        self.char_offsets = array("I", [0])
        self.offsets = array("I", [0])
        self.ud = array("H")
        self.lemma_suffix = array("I")
        self.form_suffix = array("I")
        self.length = array("B")
        self.count = array("f")
        self.share = array("f")
        self.gold = array("B")
        self.loose = array("B")

    def add(
        self,
        word: str,
        candidates: list,
        pos: list[str],
        chars: dict[str, int],
        suffixes: dict[str, int],
        grow: bool,
    ) -> None:
        golds = {(lemma, pos[ud]) for lemma, ud, *_, gold in candidates if gold}
        for char in chars_of(word):
            if char not in chars and grow:
                chars[char] = len(chars) + 2
            self.chars.append(chars.get(char, 1))
        self.char_offsets.append(len(self.chars))
        for lemma, ud, lemma_suffix, form_suffix, length, count, share, gold in candidates:
            self.loose.append((lemma, pos[ud]) in golds)
            for suffix, column in (
                (lemma_suffix, self.lemma_suffix),
                (form_suffix, self.form_suffix),
            ):
                if suffix not in suffixes and grow:
                    suffixes[suffix] = len(suffixes) + 1
                column.append(suffixes.get(suffix, 0))
            self.ud.append(ud)
            self.length.append(length)
            self.count.append(count)
            self.share.append(share)
            self.gold.append(gold)
        self.offsets.append(len(self.ud))

    def numpy(self) -> dict[str, np.ndarray]:
        f = {
            name: np.frombuffer(getattr(self, name), dtype=getattr(self, name).typecode).copy()
            for name in (
                "chars",
                "char_offsets",
                "offsets",
                "ud",
                "lemma_suffix",
                "form_suffix",
                "length",
                "count",
                "share",
                "gold",
                "loose",
            )
        }
        # Признак самого длинного окончания среди кандидатов примера
        owner = np.repeat(np.arange(examples_in(f)), np.diff(f["offsets"]))
        top = np.zeros(examples_in(f), dtype=np.uint8)
        np.maximum.at(top, owner, f["length"])
        f["longest"] = (f["length"] == top[owner]).astype(np.float32)
        f["owner"] = owner
        return f


def collect(
    morph: Analyzer, words: list[str], chars: dict, suffixes: dict, grow: bool
) -> tuple[Data, Data]:
    """
    Примеры обучения и проверки: проверочные - слова лексем с проверочной леммой

    Аргументы:
        morph (Analyzer): Анализатор словаря
        words (list[str]): Словарные слова
        chars (dict): Номера букв
        suffixes (dict): Номера суффиксов
        grow (bool): Пополнять ли таблицы

    Вывод:
        tuple[Data, Data]: Обучающие и проверочные примеры
    """
    train, dev = Data(), Data()
    pos = [pos for pos, _ in morph._ud_tags()]
    for word in words:
        candidates = morph._guess_examples(word)
        golds = [candidate for candidate in candidates if candidate[7]]
        if not golds:
            continue
        target = dev if dev_lemma(golds[0][0]) else train
        target.add(word, candidates, pos, chars, suffixes, grow and target is train)
    return train, dev


class Model(nn.Module):
    """
    Посимвольный LSTM над словом с конца и оценка кандидатов: строки таблиц признаков - вектор
    размера dim и смещение в последнем столбце
    """

    def __init__(self, n_chars: int, n_keys: int, n_suffixes: int, args: argparse.Namespace):
        super().__init__()
        self.char = nn.Embedding(n_chars, args.dchar, padding_idx=0)
        self.lstm = nn.LSTM(args.dchar, args.hidden, batch_first=True)
        self.hidden = nn.Linear(args.hidden, args.dim)
        width = args.dim + 1
        # Строка 0 ключей - дополнение коротких тегов: она всегда нулевая, как в ядре
        self.key = nn.Embedding(n_keys, width, padding_idx=0)
        self.lemma_suffix = nn.Embedding(n_suffixes, width)
        self.form_suffix = nn.Embedding(n_suffixes, width)
        self.length = nn.Embedding(MAX_ENDING, width)
        self.numeric = nn.Linear(3, 1, bias=False)
        for table in (self.key, self.lemma_suffix, self.form_suffix):
            nn.init.normal_(table.weight, std=0.05)
            with torch.no_grad():
                table.weight[0].zero_()

    def forward(self, x: dict[str, torch.Tensor], tag_keys: torch.Tensor) -> torch.Tensor:
        packed = nn.utils.rnn.pack_padded_sequence(
            self.char(x["chars"]), x["char_lengths"], batch_first=True, enforce_sorted=False
        )
        _, (state, _) = self.lstm(packed)
        h = torch.tanh(self.hidden(state[0]))
        h = torch.cat([h, torch.ones(len(h), 1)], 1)[x["owner"]]
        # Ключи тега: строки таблицы key по номеру тега UD, 0 - пусто
        e = self.key(tag_keys[x["ud"]]).sum(1)
        e = e + self.lemma_suffix(x["lemma_suffix"]) + self.form_suffix(x["form_suffix"])
        e = e + self.length(x["length"] - 1)
        numeric = torch.stack([torch.log1p(x["count"]), x["share"], x["longest"]], 1)
        return (h * e).sum(1) + self.numeric(numeric)[:, 0]


def batch(f: dict[str, np.ndarray], examples: np.ndarray) -> dict[str, torch.Tensor]:
    """Примеры `examples`: буквы с дополнением нулями, кандидаты подряд и номер примера в пакете"""
    starts, ends = f["char_offsets"][examples], f["char_offsets"][examples + 1]
    lengths = ends - starts
    chars = np.zeros((len(examples), int(lengths.max())), dtype=np.int64)
    for row, (start, end) in enumerate(zip(starts, ends, strict=True)):
        chars[row, : end - start] = f["chars"][start:end]
    first, last = f["offsets"][examples], f["offsets"][examples + 1]
    rows = np.concatenate([np.arange(a, b) for a, b in zip(first, last, strict=True)])
    owner = np.repeat(np.arange(len(examples)), last - first)
    t = torch.from_numpy
    return {
        "chars": t(chars),
        "char_lengths": t(lengths.astype(np.int64)),
        "owner": t(owner),
        "ud": t(f["ud"][rows].astype(np.int64)),
        "lemma_suffix": t(f["lemma_suffix"][rows].astype(np.int64)),
        "form_suffix": t(f["form_suffix"][rows].astype(np.int64)),
        "length": t(f["length"][rows].astype(np.int64)),
        "count": t(f["count"][rows]),
        "share": t(f["share"][rows]),
        "longest": t(f["longest"][rows]),
        "gold": t(f["gold"][rows].astype(bool)),
    }


def segment_logsumexp(scores: torch.Tensor, owner: torch.Tensor, n: int) -> torch.Tensor:
    top = torch.full((n,), -1e30).scatter_reduce(0, owner, scores, "amax")
    sums = torch.zeros(n).index_add(0, owner, torch.exp(scores - top[owner]))
    return top + torch.log(sums)


def first_right(f: dict[str, np.ndarray], scores: np.ndarray) -> tuple[float, float]:
    """
    Доли примеров, у которых первый по оценке кандидат верен: та же форма или те же лемма и тег,
    и с теми же леммой и частью речи

    Аргументы:
        f (dict[str, np.ndarray]): Примеры
        scores (np.ndarray): Оценки кандидатов

    Вывод:
        tuple[float, float]: Доли верных по тегу и по лемме с частью речи
    """
    exact = loose = 0
    for example in range(examples_in(f)):
        a, b = f["offsets"][example], f["offsets"][example + 1]
        best = a + int(np.argmax(scores[a:b]))
        exact += bool(f["gold"][best])
        loose += bool(f["loose"][best])
    return exact / examples_in(f), loose / examples_in(f)


def predict(model: Model, f: dict[str, np.ndarray], tag_keys: torch.Tensor) -> np.ndarray:
    """Оценки всех кандидатов"""
    model.eval()
    n = examples_in(f)
    with torch.no_grad():
        scores = [
            model(batch(f, np.arange(start, min(n, start + 4096))), tag_keys).numpy()
            for start in range(0, n, 4096)
        ]
    return np.concatenate(scores)


def baseline(f: dict[str, np.ndarray]) -> tuple[float, float]:
    """Доли верных у выбора по числу словоформ самого длинного окончания, как без модели"""
    return first_right(f, f["longest"] * 1e9 + f["count"])


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


def quantized(array_: np.ndarray) -> bool:
    """Матрицы от 4096 чисел хранятся в int8, остальное - в f32"""
    return array_.shape[0] > 1 and array_.size >= 4096


def quantize_model(model: Model) -> None:
    """Замена весов модели квантованными, как в файле весов"""
    with torch.no_grad():
        for parameter in model.parameters():
            values = parameter.detach().float().cpu().numpy()
            if values.ndim == 2 and quantized(values):
                scale, q = quantize(values)
                parameter.copy_(torch.from_numpy(q.astype(np.float32) * scale[:, None]))


def tensors(model: Model) -> dict[str, np.ndarray]:
    """Тензоры файла весов: смещения LSTM сложены, матрицы - [выход, вход]"""
    state = {key: value.float().cpu().numpy() for key, value in model.state_dict().items()}
    out = {
        "char": state["char.weight"],
        "lstm.ih": state["lstm.weight_ih_l0"],
        "lstm.hh": state["lstm.weight_hh_l0"],
        "lstm.bias": (state["lstm.bias_ih_l0"] + state["lstm.bias_hh_l0"])[None],
    }
    out["hidden.weight"] = state["hidden.weight"]
    out["hidden.bias"] = state["hidden.bias"][None]
    for name in ("key", "lemma_suffix", "form_suffix", "length"):
        out[name] = state[f"{name}.weight"]
    out["numeric"] = state["numeric.weight"]
    return out


def write(
    path: Path,
    strings: dict[str, list[str]],
    weights: dict[str, np.ndarray],
    config: list[int],
    sources: dict,
) -> None:
    """
    Файл весов: сигнатура, версия, число записей, затем записи «имя, вид, данные», числа
    little-endian. Вид 0 - строки, 1 - матрица f32, 2 - матрица int8 с масштабом f32 на строку,
    3 - числа u32. Запись sources - источники обучения строкой JSON

    Аргументы:
        path (Path): Файл
        strings (dict[str, list[str]]): Таблицы строк
        weights (dict[str, np.ndarray]): Тензоры
        config (list[int]): Размеры модели
        sources (dict): Источники обучения
    """
    entries = []

    def name(text: str) -> bytes:
        data = text.encode()
        return struct.pack("<H", len(data)) + data

    entries.append(
        name("config")
        + struct.pack("<BI", 3, len(config))
        + struct.pack(f"<{len(config)}I", *config)
    )
    for key, items in {"sources": [json.dumps(sources, ensure_ascii=False)], **strings}.items():
        body = struct.pack("<BI", 0, len(items))
        for text in items:
            data = text.encode()
            body += struct.pack("<I", len(data)) + data
        entries.append(name(key) + body)
    for key, values in weights.items():
        rows, cols = values.shape
        if quantized(values):
            scale, q = quantize(values)
            entries.append(
                name(key)
                + struct.pack("<BII", 2, rows, cols)
                + scale.astype("<f4").tobytes()
                + q.tobytes()
            )
        else:
            entries.append(
                name(key) + struct.pack("<BII", 1, rows, cols) + values.astype("<f4").tobytes()
            )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(MAGIC + struct.pack("<II", FORMAT, len(entries)) + b"".join(entries))


def main() -> None:
    parser = argparse.ArgumentParser(description="Обучение модели класса парадигмы ruMorph")
    parser.add_argument(
        "--dictionary",
        type=Path,
        required=True,
        help="файл словаря, кандидатов которого учит модель",
    )
    parser.add_argument(
        "--export",
        type=Path,
        default=Path("target/opencorpora"),
        help="каталог выгрузки словаря: слова - из words.tsv",
    )
    parser.add_argument(
        "--output", type=Path, default=Path("data/ru-paradigms.bin"), help="файл весов"
    )
    parser.add_argument("--words", type=int, default=1_200_000, help="обучающих слов, 0 - все")
    parser.add_argument("--dim", type=int, default=64)
    parser.add_argument("--hidden", type=int, default=32)
    parser.add_argument("--dchar", type=int, default=32)
    parser.add_argument("--epochs", type=int, default=8)
    parser.add_argument("--lr", type=float, default=2e-3)
    parser.add_argument("--batch", type=int, default=512)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    torch.manual_seed(args.seed)
    torch.set_num_threads(min(8, torch.get_num_threads()))
    rng = np.random.default_rng(args.seed)

    started = time.time()
    morph = Analyzer(args.dictionary)
    with (args.export / "words.tsv").open(encoding="utf-8") as lines:
        words = [line.split("\t", 1)[0] for line in lines]
    words = [word for word in words if len(word) >= MIN_LENGTH and set(word) <= CYRILLIC]
    if args.words and args.words < len(words):
        words = [words[i] for i in sorted(rng.choice(len(words), args.words, replace=False))]
    chars: dict[str, int] = {}
    suffixes: dict[str, int] = {}
    train_data, dev_data = collect(morph, words, chars, suffixes, grow=True)
    f, dev = train_data.numpy(), dev_data.numpy()
    keys = sorted({key for pos, feats in morph._ud_tags() for key in tag_keys(pos, feats)})
    key_numbers = {key: number + 1 for number, key in enumerate(keys)}
    tags = [[key_numbers[key] for key in tag_keys(pos, feats)] for pos, feats in morph._ud_tags()]
    width = max(map(len, tags))
    tag_table = torch.tensor([row + [0] * (width - len(row)) for row in tags])
    print(
        f"данные: {examples_in(f)} примеров ({len(f['ud'])} кандидатов), проверка "
        f"{examples_in(dev)}, {len(chars)} букв, {len(suffixes)} суффиксов, {len(keys)} "
        f"признаков тегов, {time.time() - started:.0f} с; без модели: проверка "
        + percents(baseline(dev)),
        flush=True,
    )

    model = Model(len(chars) + 2, len(keys) + 1, len(suffixes) + 1, args)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=1e-5)
    n = examples_in(f)
    steps = args.epochs * math.ceil(n / args.batch)
    sched = torch.optim.lr_scheduler.LambdaLR(
        opt, lambda s: min(1.0, s / 300) * 0.5 * (1 + math.cos(math.pi * min(s, steps) / steps))
    )
    best, best_state = -1.0, None
    for epoch in range(args.epochs):
        model.train()
        started, loss_sum, count = time.time(), 0.0, 0
        order = rng.permutation(n)
        for start in range(0, n, args.batch):
            examples = order[start : start + args.batch]
            x = batch(f, examples)
            scores = model(x, tag_table)
            all_ = segment_logsumexp(scores, x["owner"], len(examples))
            right = segment_logsumexp(
                scores.masked_fill(~x["gold"], -1e30), x["owner"], len(examples)
            )
            loss = (all_ - right).mean()
            opt.zero_grad()
            loss.backward()
            nn.utils.clip_grad_norm_(model.parameters(), 5.0)
            opt.step()
            sched.step()
            loss_sum, count = loss_sum + loss.item(), count + 1
        scores = first_right(dev, predict(model, dev, tag_table))
        score = scores[0]
        print(
            f"эпоха {epoch + 1}: потери {loss_sum / count:.4f}, {time.time() - started:.0f} с, "
            "проверка " + percents(scores),
            flush=True,
        )
        if score > best:
            best = score
            best_state = {key: value.detach().clone() for key, value in model.state_dict().items()}
    model.load_state_dict(best_state)
    quantize_model(model)
    print("проверка (int8): " + percents(first_right(dev, predict(model, dev, tag_table))))
    info = morph.dictionary_info
    sources = {
        "opencorpora_revision": info.get("opencorpora_revision"),
        "wiktionary_dump": info.get("wiktionary", {}).get("dump"),
        "words": len(words),
        "license": "CC BY-SA 4.0",
    }
    by_number = lambda table: [item for item, _ in sorted(table.items(), key=lambda pair: pair[1])]  # noqa: E731
    write(
        args.output,
        {"chars": by_number(chars), "keys": keys, "suffixes": by_number(suffixes)},
        tensors(model),
        [args.dim, args.hidden, args.dchar, MAX_CHARS],
        sources,
    )
    print(f"{args.output}: {args.output.stat().st_size / 1e6:.2f} МБ")


if __name__ == "__main__":
    main()
