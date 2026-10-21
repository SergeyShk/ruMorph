import copy
import errno
import json
import math
import os
import pickle
import shutil
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

import rumorph
from rumorph import Analysis, Analyzer


def tag_text(analysis):
    # Строка признаков из состояния pickle - объект тега, общий для разборов одного словаря
    return analysis.__reduce__()[1][1][3]


@pytest.fixture(scope="module")
def morph():
    return Analyzer()


def test_default_dictionary(morph):
    assert morph.path == Path(rumorph.__file__).parent / "data" / "ru.dict"
    meta = json.loads(morph._meta())
    assert meta["source"] == "pymorphy3-dicts-ru"
    assert meta["opencorpora_revision"] == "417150"


def test_path_argument(morph):
    assert Analyzer(str(morph.path)).path == morph.path
    assert Analyzer(path=morph.path).analyze("стали") == morph.analyze("стали")


def test_missing_dictionary(tmp_path):
    path = tmp_path / "нет.dict"
    with pytest.raises(FileNotFoundError, match="Нет файла словаря") as error:
        Analyzer(path)
    assert error.value.errno == errno.ENOENT
    assert error.value.filename == str(path)


def test_directory_instead_of_dictionary(tmp_path):
    with pytest.raises((IsADirectoryError, PermissionError)) as error:
        Analyzer(tmp_path)
    assert error.value.filename == str(tmp_path)


@pytest.mark.skipif(not hasattr(os, "mkfifo"), reason="именованные каналы есть только в POSIX")
def test_dictionary_from_pipe(morph, tmp_path):
    pipe = tmp_path / "ru.dict"
    os.mkfifo(pipe)
    writer = threading.Thread(target=pipe.write_bytes, args=(morph.path.read_bytes(),))
    writer.start()
    try:
        assert Analyzer(pipe).lemmatize(["стали"]) == ["стать"]
    finally:
        writer.join()


@pytest.mark.skipif(
    not Path("/dev/zero").exists(), reason="устройство /dev/zero есть только в POSIX"
)
def test_endless_device():
    with pytest.raises(ValueError, match="нет сигнатуры словаря"):
        Analyzer("/dev/zero")


@pytest.mark.parametrize("content", [b"", b"not a dictionary", b"RUMORPH\x00\x01\x00\x00\x00"])
def test_broken_dictionary(tmp_path, content):
    path = tmp_path / "broken.dict"
    path.write_bytes(content)
    with pytest.raises(ValueError, match="неверный формат словаря"):
        Analyzer(path)


def test_analyze(morph):
    analyses = morph.analyze("Стали")
    assert [analysis.lemma for analysis in analyses] == ["стать"] + ["сталь"] * 5
    assert {analysis.word for analysis in analyses} == {"стали"}
    assert {analysis.source for analysis in analyses} == {"dictionary"}
    probs = [analysis.prob for analysis in analyses]
    assert probs == sorted(probs, reverse=True)
    assert math.isclose(sum(probs), 1.0)
    assert probs[0] == pytest.approx(0.975, abs=1e-3)


@pytest.mark.parametrize("word", ["еж", "ёж", "ЕЖ", "Ёж"])
def test_yo_and_case(morph, word):
    assert {(analysis.word, analysis.lemma) for analysis in morph.analyze(word)} == {("ёж", "ёж")}


def test_e_matches_both_spellings(morph):
    assert {analysis.word for analysis in morph.analyze("все")} == {"все", "всё"}
    assert {analysis.word for analysis in morph.analyze("всё")} == {"всё"}


@pytest.mark.parametrize(
    ("word", "spelling", "lemma"),
    [("еще", "ещё", "ещё"), ("гнет", "гнёт", "гнуть"), ("замерзла", "замёрзла", "замёрзнуть")],
)
def test_frequencies_of_yo_spelling(morph, word, spelling, lemma):
    probs = [analysis.prob for analysis in morph.analyze(spelling)]
    assert [analysis.prob for analysis in morph.analyze(word)] == probs
    assert morph.lemmatize([word]) == [lemma]


def test_uniform_without_frequencies(morph):
    probs = [analysis.prob for analysis in morph.analyze("ёж")]
    assert probs == [1 / len(probs)] * len(probs)


@pytest.mark.parametrize("word", ["ежж1", "ВаЙб", "∂x", "км/ч", "РосПил"])
def test_unknown_word(morph, word):
    # Токен, который не разобрали ни словарь, ни правила, ни угадывание, - X без признаков
    analyses = morph.analyze(word)
    assert [(a.pos, a.lemma, a.source, dict(a.feats)) for a in analyses] == [
        ("X", word.lower(), "rule", {})
    ]
    assert not morph.is_known(word)


@pytest.mark.parametrize("word", ["", " ", "кот кот", "\x00", "кот\u200b"])
def test_not_a_token(morph, word):
    assert morph.analyze(word) == []


@pytest.mark.parametrize(
    ("word", "normal"),
    [
        ("мои\u0306", "мой"),
        ("е\u0308ж", "ёж"),
        ("во\u2011вторых", "во-вторых"),
        ("какой\u2010то", "какой-то"),
        ("Москва\u0301", "Москва"),
        ("за\u0300мок", "замок"),
        ("при\u00adвет", "привет"),
        ("вс\u0450", "все"),
        ("\u040dван", "Иван"),
        ("при\u2060вет", "привет"),
        ("\ufeffпривет", "привет"),
        ("cтали", "стали"),
        ("Рoccия", "Россия"),
        ("eгo", "его"),
        ("в\u2060ceх", "всех"),
    ],
)
def test_normalized_input(morph, word, normal):
    # Разложенные «й» и «ё» (NFD), неразрывный дефис, знаки ударения (и в составленных «ѐ», «ѝ»),
    # мягкий перенос, соединители слов и латинские буквы-двойники - как обычное написание
    assert morph.analyze(word) == morph.analyze(normal)
    assert morph.is_known(word)


def test_mixed_scripts(morph):
    # Кириллическая «с» в английском слове - латиница, а «ТНК-BP» - две части разной письменности
    assert morph.analyze("сedar") == morph.analyze("cedar")
    assert morph.analyze("сedar")[0].pos == "X"
    assert [a.word for a in morph.analyze("ТНК-BP")] == ["ТНК-BP"]


@pytest.mark.parametrize(
    "word", ["Ma\u0308dchen", "і\u0308жак", "у\u0306", "\u0306", "cafe\u0301"]
)
def test_combining_mark_elsewhere(morph, word):
    # Знак не после «и» или «е» ничего не составляет: слово разбирается как есть
    assert morph.analyze(word)
    assert morph.lemmatize(["кот", word])[0] == "кот"


@pytest.mark.parametrize("word", ["одной", "первого", "2005-го", "г", "млн", "РСФСР"])
def test_feats_order_and_probabilities(morph, word):
    # Признаки - по алфавиту без учета регистра, как в CoNLL-U; сумма долей не больше 1
    for analysis in morph.analyze(word):
        assert list(analysis.feats) == sorted(analysis.feats, key=str.lower)
        assert 0 < analysis.prob <= 1


@pytest.mark.parametrize(
    ("word", "pos"), [("шаров", "NOUN"), ("Шаров", "PROPN"), ("роза", "NOUN"), ("Роза", "PROPN")]
)
def test_proper_nouns_by_letter_case(morph, word, pos):
    # Строчное слово - нарицательное, с заглавной буквы чаще имя собственное
    assert morph.analyze(word)[0].pos == pos


def test_is_known(morph):
    assert morph.is_known("Ежи")
    assert morph.is_known("СТАЛИ")


def test_analyze_many(morph):
    words = ["мама", "мыла", "раму", "вайб", "Ёж"]
    assert morph.analyze_many(words) == [morph.analyze(word) for word in words]
    assert morph.analyze_many(word for word in words) == morph.analyze_many(tuple(words))
    assert morph.analyze_many([]) == []


def test_lemmatize(morph):
    words = ["Мы", "стали", "друзьями", "в", "ВАЙБЕ", "ковиде"]
    assert morph.lemmatize(words) == ["мы", "стать", "друг", "в", "вайбе", "ковид"]
    assert morph.lemmatize(iter(words)) == morph.lemmatize(words)


@pytest.mark.parametrize("method", ["analyze_many", "lemmatize"])
def test_words_must_be_strings(morph, method):
    with pytest.raises(TypeError, match="не строка"):
        getattr(morph, method)("стали")
    with pytest.raises(TypeError, match="элемент 1 - int"):
        getattr(morph, method)(["стали", 5])
    with pytest.raises(TypeError, match="элемент 0 - bytes"):
        getattr(morph, method)([b"stali"])


def test_threads(morph):
    chunks = [["мама", "мыла", "раму"] * 1000, ["стали", "все", "ёж"] * 1000] * 4
    with ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(morph.lemmatize, chunks))
    assert results == [morph.lemmatize(chunk) for chunk in chunks]


def test_analyses_of_threads(morph, tmp_path):
    # Разборы держат анализатор через ручку своего потока: в других потоках они работают,
    # а анализатор освобождается вместе с последним разбором
    path = tmp_path / "ru.dict"
    shutil.copy(morph.path, path)
    analyzer = Analyzer(path)
    references = sys.getrefcount(analyzer)
    with ThreadPoolExecutor(max_workers=4) as pool:
        analyses = list(pool.map(analyzer.analyze, ["стали", "ежа", "мама", "кошки"] * 4))
        forms = list(pool.map(lambda found: [form.word for form in found[0].forms()], analyses))
        restored = list(pool.map(lambda found: pickle.loads(pickle.dumps(found)), analyses))
    assert analyses == [analyzer.analyze(word) for word in ["стали", "ежа", "мама", "кошки"] * 4]
    assert forms == [[form.word for form in found[0].forms()] for found in analyses]
    assert restored == analyses
    assert analyses[0][0].inflect(Number="Sing", Gender="Fem") == "стала"
    assert sys.getrefcount(analyzer) > references
    del analyses, restored
    assert sys.getrefcount(analyzer) == references


def test_last_dictionary_stays_loaded(morph, tmp_path):
    # Последний словарь процесс держит и без анализаторов: объекты его тегов - те же
    path = tmp_path / "ru.dict"
    shutil.copy(morph.path, path)
    text = tag_text(Analyzer(path).analyze("стали")[0])
    assert tag_text(Analyzer(path).analyze("стали")[0]) is text


def test_more_analyzers_than_handle_slots(morph):
    # Анализаторов больше, чем ручек помнит поток: вытесненные ручки живут в своих разборах
    analyzers = [Analyzer(morph.path) for _ in range(10)]
    analyses = [analyzer.analyze("ежа")[0] for analyzer in analyzers * 2]
    expected = [form.word for form in morph.analyze("ежа")[0].forms()]
    assert all([form.word for form in analysis.forms()] == expected for analysis in analyses)
    restored = [pickle.loads(pickle.dumps(analysis)) for analysis in analyses]
    assert restored == analyses
    assert {analysis.forms()[1].word for analysis in restored} == {"ежа"}


def test_analysis(morph):
    first, second = morph.analyze("ёж")[:2]
    assert first == morph.analyze("еж")[0]
    assert first != second
    assert hash(first) == hash(morph.analyze("еж")[0])
    assert len(set(morph.analyze("стали"))) == 6
    assert repr(first) == (
        "Analysis(word='ёж', lemma='ёж', pos='NOUN', "
        "feats='Animacy=Anim|Case=Nom|Gender=Masc|Number=Sing', labels='', "
        f"prob={first.prob!r}, source='dictionary')"
    )
    assert repr(morph.analyze("мама")[0]).endswith("prob=1.0, source='dictionary')")
    assert repr(morph) == f"Analyzer(path={str(morph.path)!r})"
    with pytest.raises(AttributeError):
        first.lemma = "еж"
    with pytest.raises(TypeError):
        Analysis()


def test_ud_fields(morph):
    analysis = morph.analyze("стали")[0]
    assert analysis.pos == "VERB"
    assert analysis.feats == {
        "Aspect": "Perf",
        "Mood": "Ind",
        "Number": "Plur",
        "Tense": "Past",
        "VerbForm": "Fin",
        "Voice": "Mid",
    }
    assert analysis.labels == frozenset({"intr"})
    assert list(analysis.feats) == sorted(analysis.feats)
    feats = analysis.feats
    feats["Tense"] = "Pres"
    assert analysis.feats["Tense"] == "Past"
    assert json.loads(json.dumps(analysis.feats)) == analysis.feats
    assert copy.deepcopy(analysis) == analysis
    assert pickle.loads(pickle.dumps(analysis.feats)) == analysis.feats


@pytest.mark.parametrize(
    ("word", "lemma", "pos", "feats"),
    [
        ("который", "который", "DET", {"Case": "Nom"}),
        ("бы", "бы", "AUX", {"Mood": "Cnd"}),
        ("будет", "быть", "AUX", {"Tense": "Pres"}),
        ("однако", "однако", "ADV", {"Degree": "Pos"}),
        ("можно", "можно", "VERB", {}),
        ("кто", "кто", "PRON", {"Animacy": "Anim"}),
        ("это", "это", "PRON", {"Animacy": "Inan", "Case": "Nom"}),
        ("Ивановой", "иванова", "PROPN", {"NameType": "Sur", "Gender": "Fem"}),
        ("важнейшей", "важнейший", "ADJ", {"Degree": "Sup"}),
        ("об", "об", "ADP", {}),
        ("денег", "деньги", "NOUN", {"Number": "Plur"}),
        ("РФ", "рф", "PROPN", {"Abbr": "Yes", "NameType": "Geo"}),
        ("там", "там", "ADV", {"PronType": "Dem"}),
        ("никогда", "никогда", "ADV", {"PronType": "Neg"}),
        ("таков", "таков", "DET", {"PronType": "Dem", "Variant": "Short"}),
        ("когда-либо", "когда-либо", "ADV", {"PronType": "Ind"}),
        ("себя", "себя", "PRON", {"PronType": "Prs", "Reflex": "Yes"}),
        ("два", "два", "NUM", {"NumForm": "Word", "NumType": "Card"}),
        ("90-х", "90-й", "ADJ", {"NumForm": "Combi", "NumType": "Ord"}),
        ("кино", "кино", "NOUN", {"InflClass": "Ind"}),
        ("кстати", "кстати", "ADV", {"Degree": "Pos"}),
        ("в-четвёртых", "в-четвёртых", "ADV", {"Degree": "Pos"}),
        ("скажем", "сказать", "VERB", {}),
        ("значит", "значит", "ADV", {}),
        ("нужно", "нужный", "ADJ", {"Variant": "Short", "Gender": "Neut"}),
        ("несколько", "несколько", "NUM", {"Case": "Acc", "Animacy": "Inan"}),
        ("что-то", "что-то", "PRON", {"Case": "Nom", "PronType": "Ind"}),
        ("вуз", "вуз", "NOUN", {"Case": "Nom", "Number": "Sing"}),
        ("нечем", "нечего", "VERB", {"Case": "Ins", "Polarity": "Neg"}),
        ("никто", "никто", "PRON", {"Gender": "Masc", "PronType": "Neg"}),
        ("многие", "многие", "PRON", {"Animacy": "Anim"}),
        ("нельзя", "нельзя", "VERB", {"Polarity": "Neg"}),
        ("использовать", "использовать", "VERB", {"Aspect": "Imp"}),
        ("счастье", "счастье", "NOUN", {"Case": "Nom"}),
        ("б", "бы", "AUX", {"Mood": "Cnd"}),
        ("ж", "же", "PART", {}),
        ("чтоб", "чтобы", "SCONJ", {"Mood": "Cnd"}),
        ("дальнейший", "дальнейший", "ADJ", {"Degree": "Sup"}),
        ("наилучший", "наилучший", "ADJ", {"Degree": "Pos"}),
    ],
)
def test_syntagrus_conventions(morph, word, lemma, pos, feats):
    analysis = morph.analyze(word)[0]
    assert (analysis.lemma, analysis.pos) == (lemma, pos)
    assert feats.items() <= analysis.feats.items()


def test_possessive_determiners(morph):
    # «его», «её», «их» в SynTagRus - Poss=Yes|PronType=Prs без падежа: один разбор DET
    for word in ["его", "её", "их"]:
        determiners = [a for a in morph.analyze(word) if a.pos == "DET"]
        assert len(determiners) == 1, word
        assert dict(determiners[0].feats) == {"Poss": "Yes", "PronType": "Prs"}


@pytest.mark.parametrize(
    ("word", "lemma", "feats"),
    [
        ("того", "то", {"Case": "Gen", "Gender": "Neut", "PronType": "Dem"}),
        ("всего", "всё", {"Case": "Gen", "Animacy": "Inan", "PronType": "Tot"}),
        ("всех", "все", {"Case": "Acc", "Animacy": "Anim", "Number": "Plur"}),
    ],
)
def test_substantive_pronouns(morph, word, lemma, feats):
    pronouns = [a for a in morph.analyze(word) if a.pos == "PRON" and a.lemma == lemma]
    assert any(feats.items() <= a.feats.items() for a in pronouns), word
    assert all(form.lemma == lemma for form in pronouns[0].forms())


def test_comparative_degree(morph):
    assert {a.pos: a.feats["Degree"] for a in morph.analyze("больше")} == {
        "ADJ": "Cmp",
        "ADV": "Cmp",
    }
    # «выше» в лексеме наречия «высоко» - наречие, а не прилагательное
    assert all(a.pos != "ADJ" for a in morph.analyze("выше"))


def test_adjective_proper_names_are_labels(morph):
    analysis = morph.analyze("украинского")[0]
    assert "NameType" not in analysis.feats
    assert "Geox" in analysis.labels


def test_abbreviated_word_without_case(morph):
    # Сокращение «им.» - форма лексемы «имя»: без падежа, как в SynTagRus
    assert dict(morph.analyze("им.")[0].feats) == {"Abbr": "Yes"}


def test_female_surname_plural_forms(morph):
    female = next(a for a in morph.analyze("Иванова") if a.feats.get("Gender") == "Fem")
    words = [form.word for form in female.forms()]
    assert {"иванова", "ивановой", "ивановы", "ивановых"} <= set(words)
    # Копии форм множественного числа не меняют разбор «Ивановых»
    assert {a.lemma for a in morph.analyze("Ивановых") if a.pos == "PROPN"} == {"иванов"}


def test_female_surname_plural_pickles(morph):
    # Копии форм множественного числа в женской лексеме восстанавливаются из pickle
    female = next(a for a in morph.analyze("Иванова") if a.feats.get("Gender") == "Fem")
    plural = [form for form in female.forms() if form.feats.get("Number") == "Plur"]
    assert plural
    assert all(pickle.loads(pickle.dumps(form)) == form for form in plural)


def test_lemma_order_kept(morph):
    # Несовершенный вид раньше совершенного только внутри глагола: другие леммы не сдвигаются
    assert morph.lemmatize(["ум", "провожу", "заводил", "нагоняем"]) == [
        "ум",
        "проводить",
        "заводить",
        "нагонять",
    ]


def test_declinable_abbreviation_label(morph):
    analysis = morph.analyze("вузов")[0]
    assert "Abbr" not in analysis.feats
    assert "Abbr" in analysis.labels
    assert morph.analyze("РФ")[0].feats["Abbr"] == "Yes"


def test_yo_spellings_merged(morph):
    adjectives = [a for a in morph.analyze("недалеко") if a.pos == "ADJ"]
    assert [a.word for a in adjectives] == ["недалеко"]


def test_spelling_without_frequencies(morph):
    # Частоты есть только у «совершённого»: разборы «совершенного» получают среднюю долю
    analyses = morph.analyze("совершенного")
    assert all(a.prob > 0 for a in analyses)
    assert math.isclose(sum(a.prob for a in analyses), 1.0)


def test_clusivity_label(morph):
    assert "incl" in morph.analyze("пойдёмте")[0].labels
    assert "excl" in morph.analyze("иди")[0].labels


def test_merged_analyses(morph):
    analyses = morph.analyze("так")
    keys = [(a.word, a.lemma, a.pos, dict(a.feats)) for a in analyses]
    assert len(keys) == len({repr(key) for key in keys})
    assert analyses[0].pos == "ADV"
    assert math.isclose(sum(a.prob for a in analyses), 1.0)


def test_pickle(morph):
    analyses = morph.analyze("стали")
    restored_analyses = pickle.loads(pickle.dumps(analyses))
    assert restored_analyses == analyses
    assert [a.feats for a in restored_analyses] == [a.feats for a in analyses]
    assert [a.labels for a in restored_analyses] == [a.labels for a in analyses]
    restored = pickle.loads(pickle.dumps(morph))
    assert restored.path == morph.path
    assert restored.lemmatize(["стали"]) == ["стать"]
    assert pickle.loads(pickle.dumps(Analyzer(str(morph.path)))).path == morph.path
    hedgehog = morph.analyze("ёж")[0]
    # Копия анализатора и разбора - тот же объект: словарь не загружается еще раз
    assert copy.copy(morph) is morph
    assert copy.deepcopy(morph) is morph
    assert copy.deepcopy(hedgehog) is hedgehog
    # Вероятность не входит в равенство: разбор равен своей форме
    cat = morph.analyze("кошка")[0]
    assert cat.prob < 1
    assert cat in cat.forms()
    assert hash(cat) == hash(next(form for form in cat.forms() if form == cat))
    _, (_, state) = hedgehog.__reduce__()
    restored_analysis = rumorph._rumorph._analysis(morph, (*state[:5], 0.5, *state[6:]))
    assert (restored_analysis.prob, restored_analysis.inflect(Case="Gen")) == (0.5, "ежа")
    # Таблица и форма из другого словаря: разбор находится по написанию, лемме и тегу
    assert rumorph._rumorph._analysis(morph, (*state[:7], (0, 0))) == hedgehog
    # Признаки в другом порядке (pickle 0.1.0): «одной» - NumForm и NumType раньше Number
    one = morph.analyze("одной")[0]
    _, (_, one_state) = one.__reduce__()
    old_order = "|".join(sorted(one_state[3].split("|")))
    assert old_order != one_state[3]
    old_state = (*one_state[:3], old_order, *one_state[4:7], None)
    assert rumorph._rumorph._analysis(morph, old_state) == one
    with pytest.raises(ValueError, match="Неизвестный источник"):
        rumorph._rumorph._analysis(morph, (*state[:6], "corpus", *state[7:]))
    with pytest.raises(ValueError, match="В словаре нет разбора"):
        rumorph._rumorph._analysis(morph, (*state[:3], "Case", *state[4:]))


def test_pickled_analyses_share_tag_objects(morph):
    # Разборы из pickle берут анализатор потока с тем же словарем, а не создают свой
    first = morph.analyze("стали")[0]
    for _ in range(3):
        restored = pickle.loads(pickle.dumps(first))
        assert restored.__reduce__()[1][0] is morph
        assert restored.pos is first.pos
        # Без GIL пометы и признаки - новые объекты: общие замедляли бы потоки
        if getattr(sys, "_is_gil_enabled", lambda: True)():
            assert restored.labels is first.labels
    # Строки тега в pickle - общие объекты: pickle списка разных разборов хранит их один раз
    analyses = [morph.analyze("стали")[0] for _ in range(20)]
    assert pickle.dumps(analyses).count(tag_text(analyses[0]).encode()) == 1


def test_same_file_by_other_path(morph, tmp_path):
    # Тот же файл через «..» или симлинк не загружается еще раз
    text = tag_text(morph.analyze("стали")[0])
    paths = [morph.path.parent / ".." / morph.path.parent.name / morph.path.name]
    link = tmp_path / "link.dict"
    try:
        link.symlink_to(morph.path)
        paths.append(link)
    except OSError:
        pass
    for path in paths:
        assert tag_text(Analyzer(path).analyze("стали")[0]) is text


@pytest.mark.skipif(sys.platform == "win32", reason="Windows не заменяет отображенный файл")
def test_replaced_file_is_loaded_again(morph, tmp_path):
    # Файл, замененный другим с тем же размером и временем изменения, читается заново
    path = tmp_path / "ru.dict"
    shutil.copy(morph.path, path)
    old = Analyzer(path)
    stat = path.stat()
    new = tmp_path / "new.dict"
    new.write_bytes(bytes(stat.st_size))
    os.utime(new, ns=(stat.st_atime_ns, stat.st_mtime_ns))
    new.replace(path)
    with pytest.raises(ValueError, match="неверный формат словаря"):
        Analyzer(path)
    assert old.lemmatize(["стали"]) == ["стать"]


@pytest.mark.skipif(sys.platform == "win32", reason="Windows не удаляет отображенный файл")
def test_deleted_file_keeps_working(morph, tmp_path):
    # Словарь отображен в память: удаленный файл живет, пока жив анализатор
    path = tmp_path / "ru.dict"
    shutil.copy(morph.path, path)
    analyzer = Analyzer(path)
    analysis = analyzer.analyze("стали")[0]
    path.unlink()
    assert analyzer.lemmatize(["стали", "ёжиком"]) == ["стать", "ёжик"]
    # pickle разбора восстанавливает анализатор потока, а не читает удаленный файл
    assert pickle.loads(pickle.dumps(analysis)) == analysis
    assert pickle.loads(pickle.dumps(analyzer)) is analyzer
    # Без разборов в потоке и в другом потоке - словарь, загруженный из этого файла
    blob = pickle.dumps(analysis)
    del analysis
    assert pickle.loads(blob).lemma == "стать"
    assert pickle.loads(pickle.dumps(analyzer)).path == path
    with ThreadPoolExecutor(1) as pool:
        assert pool.submit(pickle.loads, blob).result().lemma == "стать"


@pytest.mark.skipif(not hasattr(os, "fork"), reason="нет fork")
@pytest.mark.filterwarnings("ignore::DeprecationWarning")
def test_fork_while_loading(morph, tmp_path):
    # Процесс, созданный fork, пока другой поток загружает словарь, не зависает на анализаторе
    path = tmp_path / "ru.dict"
    shutil.copy(morph.path, path)
    stop = threading.Event()

    def load():
        while not stop.is_set():
            Analyzer(path)

    thread = threading.Thread(target=load)
    thread.start()
    try:
        for _ in range(5):
            time.sleep(0.02)
            pid = os.fork()
            if pid == 0:
                os._exit(0 if Analyzer(path).lemmatize(["стали"]) == ["стать"] else 1)
            deadline = time.monotonic() + 30
            while (done := os.waitpid(pid, os.WNOHANG))[0] == 0 and time.monotonic() < deadline:
                time.sleep(0.01)
            if done[0] == 0:
                os.kill(pid, 9)
                os.waitpid(pid, 0)
            assert done[0] == pid and os.waitstatus_to_exitcode(done[1]) == 0
    finally:
        stop.set()
        thread.join()


def test_relative_path_pickles(morph, tmp_path, monkeypatch):
    # Путь хранится абсолютным: pickle работает в другом рабочем каталоге
    (tmp_path / "ru.dict").symlink_to(morph.path)
    monkeypatch.chdir(tmp_path)
    relative = Analyzer("ru.dict")
    assert relative.path == tmp_path / "ru.dict"
    data = pickle.dumps(relative.analyze("стали")[0])
    monkeypatch.chdir(tmp_path.parent)
    assert pickle.loads(data).lemma == "стать"


@pytest.mark.parametrize(
    ("call", "message"),
    [
        (lambda morph: morph.analyze(123), "Слово должно быть строкой, а не int"),
        (lambda morph: morph.is_known(None), "Слово должно быть строкой, а не NoneType"),
        (lambda morph: morph.lemmatize(5), "Ожидается набор слов, а не int"),
        (
            lambda morph: Analyzer(123),
            "Путь к словарю должен быть строкой или os.PathLike, а не int",
        ),
    ],
)
def test_type_errors_in_russian(morph, call, message):
    with pytest.raises(TypeError, match=message):
        call(morph)


def test_surrogates_replaced(morph):
    # Непарный суррогат не роняет пакетный разбор: он заменяется на U+FFFD
    assert morph.lemmatize(["кот", "\ud800", "стали"])[::2] == ["кот", "стать"]
    assert morph.analyze("кот\udc00")[0].word == "кот�"
    assert not morph.is_known("\ud800")


def test_lemmatize_shares_strings(morph):
    first, second = morph.lemmatize(["стали", "стали"])
    assert first is second
