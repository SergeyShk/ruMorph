"""
Перевод статей русского Викисловаря в лексемы OpenCorpora

Описание:
    Статья снимка Викисловаря (`wiktionary_snapshot.py`) - заголовок, часть речи, теги
    и категории статьи, формы таблицы с тегами и начала значений. Существительные, имена
    собственные и прилагательные переводятся в лексемы OpenCorpora: формы с тегами в порядке
    и записи тегов лексем OpenCorpora той же части речи. Статьи, лемма которых есть
    в OpenCorpora, статьи с пометой у первого значения (устаревшие, областные) и таблицы,
    форма которых не начинается с начала леммы, пропускаются.
"""

import re
from collections.abc import Iterable, Mapping

Lexeme = list[tuple[str, str]]

# Запись без ударений и помет таблицы: побочное ударение - буквами «ѐ», «ѝ» и латинскими с ударением
# («пельмéнный»), «^△» - помета отклонения от схемы склонения
PLAIN = str.maketrans(
    {"\u0301": None, "\u0300": None, "^": None, "△": None}
    | dict(zip("ѐѝáàéèóòý", "еиааееооу", strict=True))
)
LETTERS = set("абвгдеёжзийклмнопрстуфхцчшщъыьэюя-")
# Второй предложный - с предлогом в скобках: «(в, на) снегу»
PREPOSITION = re.compile(r"\([^)]*\)\s*(\S+)")
CASES = {
    "nominative": "nomn",
    "genitive": "gent",
    "dative": "datv",
    "accusative": "accs",
    "instrumental": "ablt",
    "prepositional": "loct",
    "locative": "loc2",
    "vocative": "voct",
}
# Частичный родительный («снегу») - помета формы, а не тег
RAW_CASES = {"Разд.": "gen2"}
CASE_ORDER = ["nomn", "gent", "gen2", "datv", "accs", "ablt", "loct", "loc2", "voct"]
NUMBERS = {"singular": "sing", "plural": "plur"}
GENDERS = {"masculine": "masc", "feminine": "femn", "neuter": "neut"}
# Формы, которых в языке нет или которые редки: в таблице Викисловаря - со звездочкой или с пометой
DOUBTFUL = (
    "данная форма предположительна или малоупотребима",
    "образование данной формы затруднительно",
)
# Помета у первого, основного значения: лексема устаревшая, областная или церковная - ее формы дали бы частым словам
# лишние разборы, а незнакомым («Шатов» - родительный от «шат») - неверный словарный
MARKS = re.compile(
    r"^(?:[^\s.]+\.,?\s|\([^)]*\)\s)*?(устар|диал|рег|обл|старин|арх|церк|местн)\.", re.IGNORECASE
)
# Категории имен собственных: окончание категории имен, фамилий и отчеств («Армянские фамилии/ru»; у статьи в двух
# категориях - имя, как в OpenCorpora), начало категории топонимов («Реки России/ru»)
PROPER = (("Name", "имена/ru"), ("Surn", "фамилии/ru"), ("Patr", "отчества/ru"))
PLACES = (
    "Реки",
    "Названия рек",
    "Гидронимы",
    "Озёра",
    "Моря",
    "Заливы",
    "Острова",
    "Полуострова",
    "Горы",
    "Хребты",
    "Города",
    "Столицы",
    "Сёла",
    "Посёлки",
    "Деревни",
    "Станицы",
    "Хутора",
    "Аулы",
    "Населённые пункты",
    "Топонимы",
    "Страны",
    "Регионы",
    "Области",
    "Районы",
    "Провинции",
    "Штаты",
)
INDECLINABLE = "Несклоняемые существительные/ru"
# Предел длины словоформы в словаре ruMorph; форма из двух букв редкого слова («ит» у «ита») закрыла бы
# разбор аббревиатуры правилом («ИТ»)
MAX_LENGTH = 64
MIN_LENGTH = 3


def lexemes(
    entries: Iterable[Mapping],
    known: set[str],
    spellings: Mapping[frozenset[str], str],
    forms: set[str],
) -> list[Lexeme]:
    """
    Лексемы OpenCorpora из статей Викисловаря

    Аргументы:
        entries (Iterable[Mapping]): Статьи снимка
        known (set[str]): Леммы OpenCorpora в нижнем регистре с «е» вместо «ё»
        spellings (Mapping[frozenset[str], str]): Записи тегов OpenCorpora по набору граммем
        forms (set[str]): Словоформы OpenCorpora

    Вывод:
        list[Lexeme]: Лексемы без повторов
    """
    result: list[Lexeme] = []
    seen: set[tuple[tuple[str, str], ...]] = set()
    for entry in entries:
        word = entry["word"]
        if normal(word) in known or marked(entry.get("glosses", [])):
            continue
        if entry["pos"] in ("noun", "name"):
            lexeme = noun(entry)
        elif entry["pos"] == "adj":
            lexeme = adjective(entry)
        else:
            continue
        if lexeme is None or not consistent(lexeme):
            continue
        # Субстантиват («косая»), pluralia tantum («слова») и несклоняемое слово (буква «люди») без новых
        # форм дали бы словоформам OpenCorpora только лишние разборы
        tag = lexeme[0][1]
        if all(form in forms for form, _ in lexeme) and (
            "adjective" in entry["tags"] or "Pltm" in tag or "Fixd" in tag
        ):
            continue
        lexeme = [(form, spelling(tag, spellings)) for form, tag in lexeme]
        if tuple(lexeme) not in seen:
            seen.add(tuple(lexeme))
            result.append(lexeme)
    return result


def normal(word: str) -> str:
    """
    Запись для сравнения лемм: без ударений, в нижнем регистре, «е» вместо «ё»

    Аргументы:
        word (str): Слово

    Вывод:
        str: Запись
    """
    return word.lower().translate(PLAIN).replace("ё", "е")


def marked(glosses: Iterable[str]) -> bool:
    """
    Первое значение статьи - с пометой устаревшего, областного или церковного слова

    Аргументы:
        glosses (Iterable[str]): Начала значений

    Вывод:
        bool: Помета у первого непустого значения
    """
    first = next((gloss for gloss in glosses if gloss), "")
    return bool(MARKS.match(first))


def consistent(lexeme: Lexeme) -> bool:
    """
    Таблица не битая: каждая форма из кириллических букв и дефиса, длиной от MIN_LENGTH до MAX_LENGTH
    и, кроме сравнительной степени на по-, начинается с двух первых букв леммы

    Аргументы:
        lexeme (Lexeme): Формы и теги

    Вывод:
        bool: Таблица годится
    """
    lemma = normal(lexeme[0][0])
    start = lemma[:2]
    return all(
        MIN_LENGTH <= len(form) <= MAX_LENGTH and set(form) <= LETTERS for form, _ in lexeme
    ) and all(normal(form).startswith(start) for form, tag in lexeme if "Cmp2" not in tag)


def spelling(tag: str, spellings: Mapping[frozenset[str], str]) -> str:
    """
    Запись тега OpenCorpora для того же набора граммем, если она есть

    Аргументы:
        tag (str): Тег
        spellings (Mapping[frozenset[str], str]): Записи тегов OpenCorpora по набору граммем

    Вывод:
        str: Тег
    """
    return spellings.get(frozenset(tag.replace(" ", ",", 1).split(",")), tag)


def cells(entry: Mapping) -> dict[tuple[str, ...], list[str]]:
    """
    Формы таблицы по ячейкам: число или род, падеж и прочие теги; без ударений, в нижнем регистре

    Аргументы:
        entry (Mapping): Статья

    Вывод:
        dict[tuple[str, ...], list[str]]: Формы по отсортированным тегам ячейки
    """
    result: dict[tuple[str, ...], list[str]] = {}
    for form, tags, raw in entry["forms"]:
        if match := PREPOSITION.fullmatch(form):
            form = match.group(1)
        if form.startswith("*") or " " in form or "…" in form:
            continue
        if any(mark in raw for mark in DOUBTFUL) or "stressed" in tags:
            continue
        tags = list(tags) + [RAW_CASES[mark] for mark in raw if mark in RAW_CASES]
        plain = form.lower().translate(PLAIN)
        found = result.setdefault(tuple(sorted(tags)), [])
        if plain and plain not in found:
            found.append(plain)
    return result


def noun(entry: Mapping) -> Lexeme | None:
    """
    Лексема существительного или имени собственного

    Аргументы:
        entry (Mapping): Статья

    Вывод:
        Lexeme | None: Формы и теги или None, если это не нарицательное слово и не имя из категорий
            собственных
    """
    word, tags, categories = entry["word"], set(entry["tags"]), set(entry["categories"])
    # Тематические категории нарицательных слов похожи на категории имен: «Моряки/ru», «Имена/ru»
    proper = proper_kind(categories) if word[:1].isupper() else None
    if word[:1].isupper() and proper is None:
        return None
    if proper == "Geox":
        tags.discard("animate")
    lemma = word.lower()
    table = noun_table(entry, by_gender=proper == "Surn")
    if proper == "Surn":
        return surname(lemma, table)
    if proper == "Geox" and any(number == "sing" for number, _ in table):
        table = {cell: forms for cell, forms in table.items() if cell[0] == "sing"}
    numbers = {number for number, _ in table}
    base = noun_base(lemma, tags, numbers)
    if base is None:
        return None
    first = "plur" if "Pltm" in base else "sing"
    put_first(table.setdefault((first, "nomn"), []), lemma)
    # Ячейку, равную заголовку, wiktextract не пишет («грог»): винительный - как именительный
    if "inan" in base and ("Pltm" in base or "femn" not in base or lemma.endswith("ь")):
        accusative = table.setdefault((first, "accs"), [])
        if not accusative:
            accusative.append(lemma)
    forms = [form for cell in table.values() for form in cell]
    if INDECLINABLE in categories or (len(forms) > 1 and set(forms) == {lemma}):
        base.append("Fixd")
    if proper:
        base.append(proper)
    return [
        (form, f"{','.join(base)} {grammemes}")
        for number in ("sing", "plur")
        for case in CASE_ORDER
        for form, grammemes in variants(table.get((number, case), []), f"{number},{case}")
    ]


def noun_table(entry: Mapping, by_gender: bool) -> dict[tuple[str, str], list[str]]:
    """
    Формы существительного по числу и падежу; у фамилии единственное число - по роду

    Аргументы:
        entry (Mapping): Статья
        by_gender (bool): Делить единственное число по роду

    Вывод:
        dict[tuple[str, str], list[str]]: Формы по числу или роду и падежу
    """
    table: dict[tuple[str, str], list[str]] = {}
    for key, forms in cells(entry).items():
        number = next((NUMBERS[tag] for tag in key if tag in NUMBERS), None)
        case = next(
            (CASES.get(tag, tag) for tag in key if tag in CASES or tag in RAW_CASES.values()), None
        )
        if number is None or case is None:
            continue
        group = number
        if by_gender and number == "sing":
            group = next((GENDERS[tag] for tag in key if tag in GENDERS), "")
            if not group:
                continue
        table.setdefault((group, case), []).extend(forms)
    return table


def noun_base(lemma: str, tags: set[str], numbers: set[str]) -> list[str] | None:
    """
    Граммемы лексемы существительного: часть речи, одушевленность, род и число

    Аргументы:
        lemma (str): Лемма
        tags (set[str]): Теги статьи
        numbers (set[str]): Числа форм таблицы

    Вывод:
        list[str] | None: Граммемы или None, если форм нет или род не указан
    """
    if not numbers:
        return None
    base = ["NOUN", "anim" if "animate" in tags else "inan"]
    if numbers == {"plur"}:
        return [*base, "GNdr", "Pltm"]
    if "common" in tags or {"masculine", "feminine"} <= tags:
        base.append("ms-f")
    else:
        gender = next(
            (GENDERS[tag] for tag in ("masculine", "feminine", "neuter") if tag in tags), None
        )
        if gender is None:
            return None
        base.append(gender)
    return [*base, "Sgtm"] if numbers == {"sing"} else base


def put_first(forms: list[str], lemma: str) -> None:
    """
    Лемма - первая форма ячейки: она начальная форма лексемы

    Аргументы:
        forms (list[str]): Формы ячейки, меняются на месте
        lemma (str): Лемма
    """
    if lemma in forms:
        forms.remove(lemma)
    forms.insert(0, lemma)


def surname(lemma: str, table: dict[tuple[str, str], list[str]]) -> Lexeme | None:
    """
    Лексема фамилии: мужские формы, женские и множественное число, как в OpenCorpora

    Аргументы:
        lemma (str): Мужская форма в нижнем регистре
        table (dict[tuple[str, str], list[str]]): Формы по роду или числу и падежу

    Вывод:
        Lexeme | None: Формы и теги или None, если нет мужских форм
    """
    if not any(key == "masc" for key, _ in table):
        return None
    put_first(table.setdefault(("masc", "nomn"), []), lemma)
    # Ячейки, равные заголовку, wiktextract не пишет: без женских форм женская фамилия несклоняемая («Ковалёнок»)
    if not any(key == "femn" for key, _ in table):
        table.update(
            {("femn", case): [lemma] for case in ("nomn", "gent", "datv", "accs", "ablt", "loct")}
        )
    if {form for forms in table.values() for form in forms} == {lemma}:
        return [
            (lemma, f"NOUN,anim,ms-f,Fixd,Surn {number},{case}")
            for number in ("sing", "plur")
            for case in ("nomn", "gent", "datv", "accs", "ablt", "loct")
        ]
    feminine = {form for (key, _), forms in table.items() if key == "femn" for form in forms}
    groups = [
        ("masc", "sing", "NOUN,anim,masc,Sgtm,Surn"),
        (
            "femn",
            "sing",
            "NOUN,anim,femn,Sgtm,Fixd,Surn" if len(feminine) == 1 else "NOUN,anim,femn,Sgtm,Surn",
        ),
        ("plur", "plur", "NOUN,anim,ms-f,Pltm,Surn"),
    ]
    return [
        (form, f"{base} {grammemes}")
        for key, number, base in groups
        for case in CASE_ORDER
        for form, grammemes in variants(table.get((key, case), []), f"{number},{case}")
    ]


def proper_kind(categories: set[str]) -> str | None:
    """
    Граммема имени собственного по категориям статьи

    Аргументы:
        categories (set[str]): Категории

    Вывод:
        str | None: Surn, Patr, Name, Geox или None
    """
    lowered = {category.lower() for category in categories} - {"имена собственные/ru"}
    for grammeme, ending in PROPER:
        if any(category.endswith(ending) for category in lowered):
            return grammeme
    if any(category.startswith(PLACES) for category in categories):
        return "Geox"
    return None


def variants(forms: list[str], grammemes: str) -> list[tuple[str, str]]:
    """
    Формы ячейки с тегами: второй творительный на -ою, -ею и -ёю (и перед -ся) - с пометой варианта

    Аргументы:
        forms (list[str]): Формы ячейки
        grammemes (str): Граммемы формы

    Вывод:
        list[tuple[str, str]]: Формы и граммемы
    """
    result = []
    for number, form in enumerate(forms):
        stem = form.removesuffix("ся")
        if number and grammemes.endswith("ablt") and stem.endswith(("ою", "ею", "ёю")):
            result.append((form, grammemes + (",V-oy" if stem.endswith("ою") else ",V-ey")))
        else:
            result.append((form, grammemes))
    return result


def adjective(entry: Mapping) -> Lexeme | None:
    """
    Лексема прилагательного: полные формы, краткие и сравнительная степень, как в OpenCorpora

    Аргументы:
        entry (Mapping): Статья

    Вывод:
        Lexeme | None: Формы и теги или None, если нет полных форм
    """
    word, tags = entry["word"], set(entry["tags"])
    if not word.islower():
        return None
    full, short, comparative = adjective_cells(entry)
    if not full.get(("masc", "nomn", "")):
        return None
    lemma = word.lower()
    put_first(full[("masc", "nomn", "")], lemma)
    inanimate = full.setdefault(("masc", "accs", "inan"), [])
    if not inanimate:
        inanimate.append(lemma)
    qualities = ["Qual"] if "qualitative" in tags else []
    # Притяжательное на -ий («лисий» - «лисье») Викисловарь помечает не всегда
    neuter = full.get(("neut", "nomn", ""), [])
    if "possessive" in tags or (
        lemma.endswith("ий") and any(form.endswith("ье") for form in neuter)
    ):
        qualities.append("Poss")
    # Краткие формы и сравнительная степень относительных прилагательных («геномен», «ваттнее») - шаблон
    # таблицы, в языке их нет
    if "relative" in tags:
        short, comparative = {}, []
    lexeme = []
    base = ",".join(["ADJF", *qualities])
    for group in ("masc", "femn", "neut", "plur"):
        number = "plur" if group == "plur" else "sing"
        for case in ("nomn", "gent", "datv", "accs", "ablt", "loct"):
            animacies = ("anim", "inan") if case == "accs" and group in ("masc", "plur") else ("",)
            for animacy in animacies:
                grammemes = ",".join(
                    part
                    for part in (animacy, group if number == "sing" else "", number, case)
                    if part
                )
                cell = full.get((group, case, animacy), [])
                lexeme.extend((form, f"{base} {tag}") for form, tag in variants(cell, grammemes))
    short_base = ",".join(["ADJS", *qualities])
    for group in ("masc", "femn", "neut", "plur"):
        grammemes = "plur" if group == "plur" else f"{group},sing"
        for form in short.get(group, []):
            variant = ",V-en" if group == "masc" and form.endswith("енен") else ""
            lexeme.append((form, f"{short_base} {grammemes}{variant}"))
    return lexeme + comparatives(comparative, ",".join(["COMP", *qualities]))


def adjective_cells(
    entry: Mapping,
) -> tuple[dict[tuple[str, str, str], list[str]], dict[str, list[str]], list[str]]:
    """
    Формы прилагательного: полные по роду или числу, падежу и одушевленности, краткие по роду
    или числу, сравнительная степень

    Аргументы:
        entry (Mapping): Статья

    Вывод:
        tuple[dict[tuple[str, str, str], list[str]], dict[str, list[str]], list[str]]: Полные,
            краткие формы и сравнительная степень
    """
    full: dict[tuple[str, str, str], list[str]] = {}
    short: dict[str, list[str]] = {}
    comparative: list[str] = []
    for key, forms in cells(entry).items():
        if "comparative" in key:
            comparative.extend(forms)
            continue
        number = next((NUMBERS[tag] for tag in key if tag in NUMBERS), None)
        group = (
            "plur"
            if number == "plur"
            else next((GENDERS[tag] for tag in key if tag in GENDERS), None)
        )
        case = next((CASES[tag] for tag in key if tag in CASES), None)
        if group is None:
            continue
        if "short-form" in key:
            short.setdefault(group, []).extend(forms)
        elif case == "accs" and group in ("masc", "plur"):
            # Винительный мужского рода и множественного числа - по одушевленности
            if ("animate" in key) != ("inanimate" in key):
                animacy = "anim" if "animate" in key else "inan"
                full.setdefault((group, case, animacy), []).extend(forms)
        elif case is not None:
            full.setdefault((group, case, ""), []).extend(forms)
    return full, short, comparative


def comparatives(forms: list[str], base: str) -> Lexeme:
    """
    Сравнительная степень с пометами OpenCorpora: на по- - Cmp2, на -ей - V-ej

    Аргументы:
        forms (list[str]): Формы
        base (str): Граммемы лексемы

    Вывод:
        Lexeme: Формы и теги
    """
    plain = {form for form in forms if not form.startswith("по")}
    lexeme = []
    for form in forms:
        marks = []
        if form.startswith("по") and form[2:] in plain:
            marks.append("Cmp2")
        if form.endswith("ей"):
            marks.append("V-ej")
        lexeme.append((form, f"{base} {','.join(marks)}".strip()))
    return lexeme
