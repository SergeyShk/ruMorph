"""
Соглашения разметки UD_Russian-Taiga и UD_Russian-GSD, приведенные к соглашениям ruMorph

Описание:
    ruMorph размечает по руководству UD, а где оно молчит - по соглашениям SynTagRus. Taiga и GSD
    в части слов размечают иначе: счетчики словаря (`export_ud.py`) и обучение контекстной модели
    (`train_context.py`) переписывают их разметку по этим правилам. Правила смотрят на словоформу,
    лемму, часть речи и синтаксическую связь токена; признаки, которые однозначно задает словарь,
    достраивает обучение по разборам словаря.
"""

from dataclasses import dataclass, field
from pathlib import Path

TAIGA = "UD_Russian-Taiga"
GSD = "UD_Russian-GSD"
# Предлоги с беглой «о»: лемма - без нее, как у большинства в SynTagRus
SHORT_PREPOSITIONS = {
    "во": "в",
    "со": "с",
    "ко": "к",
    "изо": "из",
    "ото": "от",
    "передо": "перед",
    "подо": "под",
    "надо": "над",
}
# Слова с частью речи SynTagRus: (часть речи трибанка, лемма) - новая часть речи
POS_CHANGES = {
    ("CCONJ", "однако"): "ADV",
    ("ADV", "тоже"): "PART",
    ("PART", "пожалуйста"): "ADV",
    ("PART", "хоть"): "ADV",
    ("ADJ", "угодный"): "ADV",
    ("CCONJ", "то"): "SCONJ",
    ("ADV", "всё"): "PART",
}
# Вводные слова, которые в SynTagRus - наречия: лемма трибанка - лемма наречия
PARENTHETICALS = {"значить": "значит", "разуметься": "разумеется", "правда": "правда"}
# Супплетивные превосходные прилагательные: в SynTagRus Degree=Pos
SUPPLETIVE = {
    "высший",
    "худший",
    "лучший",
    "меньший",
    "наибольший",
    "наилучший",
    "наименьший",
    "низший",
}
# Местоименные прилагательные, которые GSD размечает ADJ или PRON, а SynTagRus и Taiga - DET
DETERMINERS = {"который", "другой", "сам", "самый", "многий"}


@dataclass
class Token:
    """Токен CoNLL-U: номер, форма, лемма, часть речи, признаки, вершина и связь"""

    id: str
    form: str
    lemma: str
    upos: str
    feats: dict[str, str] = field(default_factory=dict)
    head: str = "_"
    deprel: str = "_"


def sentences(path: Path) -> list[list[Token]]:
    """
    Предложения CoNLL-U; многословные токены и пустые узлы пропускаются

    Аргументы:
        path (Path): Файл CoNLL-U

    Вывод:
        list[list[Token]]: Предложения
    """
    result, sentence = [], []
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line:
            if sentence:
                result.append(sentence)
            sentence = []
            continue
        if not line[0].isdigit():
            continue
        cols = line.split("\t")
        if "-" in cols[0] or "." in cols[0]:
            continue
        feats = dict(f.split("=", 1) for f in cols[5].split("|")) if cols[5] != "_" else {}
        sentence.append(Token(cols[0], cols[1], cols[2], cols[3], feats, cols[6], cols[7]))
    if sentence:
        result.append(sentence)
    return result


def reflexive(word: str) -> bool:
    """Глагол на -ся или -сь"""
    return word.endswith(("ся", "сь"))


def harmonize(treebank: str, sentence: list[Token]) -> None:
    """
    Разметка предложения трибанка `treebank` в соглашениях ruMorph: правила по словоформе,
    лемме, части речи и связи токена

    Аргументы:
        treebank (str): Трибанк: `TAIGA` или `GSD`; другие не меняются
        sentence (list[Token]): Предложение, меняется на месте
    """
    if treebank not in (TAIGA, GSD):
        return
    passive_heads = {t.head for t in sentence if t.deprel == "nsubj:pass"}
    for token in sentence:
        form, lemma = token.form.lower(), token.lemma.lower()
        if token.upos == "ADP" and lemma == form and lemma in SHORT_PREPOSITIONS:
            token.lemma = SHORT_PREPOSITIONS[lemma]
        if (token.upos, lemma) in POS_CHANGES:
            token.upos = POS_CHANGES[token.upos, lemma]
            if lemma == "угодный":
                token.lemma = "угодно"
            if token.upos in ("PART", "SCONJ"):
                token.feats = {}
        if lemma in PARENTHETICALS and token.deprel == "parataxis:discourse":
            token.upos, token.lemma, token.feats = "ADV", PARENTHETICALS[lemma], {"Degree": "Pos"}
        if lemma in SUPPLETIVE and token.feats.get("Degree") == "Sup":
            token.feats["Degree"] = "Pos"
        # Пассив на -ся: в Taiga, как в SynTagRus, Voice=Pass и невозвратная лемма, в GSD - Mid
        if (
            token.upos == "VERB"
            and reflexive(form)
            and token.feats.get("Voice") == "Mid"
            and token.id in passive_heads
            and reflexive(lemma)
        ):
            token.feats["Voice"] = "Pass"
            token.lemma = lemma[:-2]
        if treebank == GSD:
            if lemma in DETERMINERS and token.upos in ("ADJ", "PRON"):
                token.upos = "DET"
            if token.upos == "ADP" and lemma == "как":
                token.upos = "SCONJ"
