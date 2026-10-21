import json
import shutil
import signal
import subprocess
import sys
import threading
from pathlib import Path

import pytest

import rumorph
from rumorph import __main__ as command

TEXT = "Мы стали друзьями в ковиде.\n"


def run(*args: str, text: str = "") -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-m", "rumorph", *args],
        input=text,
        capture_output=True,
        encoding="utf-8",
        check=False,
    )


def test_lemmas():
    result = run(text=TEXT + "\nКоты, 2005-го года!\n")
    assert (result.returncode, result.stderr) == (0, "")
    assert result.stdout == "мы стать друг в ковид .\n\nкот , 2005-й год !\n"


def test_formats(tmp_path):
    path = tmp_path / "text.txt"
    path.write_text(TEXT, encoding="utf-8")
    tsv = run("--format", "tsv", str(path)).stdout.splitlines()
    assert tsv[1].split("\t")[:3] == ["стали", "стать", "VERB"]
    conllu = run("-f", "conllu", str(path)).stdout.splitlines()
    assert conllu[:2] == ["# sent_id = 1", "# text = Мы стали друзьями в ковиде."]
    columns = conllu[6].split("\t")
    assert columns[:4] == ["5", "ковиде", "ковид", "NOUN"]
    assert (columns[5], columns[9]) == (
        "Animacy=Inan|Case=Loc|Gender=Masc|Number=Sing",
        "SpaceAfter=No",
    )
    sentence = json.loads(run("-f", "jsonl", str(path)).stdout)
    assert sentence["text"] == TEXT.strip()
    first = sentence["tokens"][1]["analyses"][0]
    assert (first["lemma"], first["pos"], first["feats"]["Tense"], first["labels"]) == (
        "стать",
        "VERB",
        "Past",
        ["intr"],
    )
    assert first["source"] == "dictionary" and 0 < first["prob"] <= 1


def test_tokens_and_conllu_input():
    assert run("-i", "tokens", text="Мы\nстали\n\nдрузьями\n").stdout == "мы стать\nдруг\n"
    conllu = "# text = Мы стали\n1\tМы\t_\t_\t_\t_\t2\tnsubj\t_\t_\n2\tстали\t_\t_\t_\t_\t0\troot\t_\t_\n"
    lines = run("-i", "conllu", "-f", "conllu", text=conllu).stdout.splitlines()
    assert lines[2].split("\t")[:4] == ["2", "стали", "стать", "VERB"]
    assert lines[2].split("\t")[6:8] == ["0", "root"]


def test_version_and_help():
    assert run("--version").stdout == f"rumorph {rumorph.__version__}\n"
    assert run("--help").stdout.startswith("rumorph - морфологический анализ русского текста")


def test_errors(tmp_path):
    result = run("--format", "xml")
    assert result.returncode == 2
    assert result.stderr == (
        "rumorph: неизвестное значение параметра --format: xml "
        "(допустимые: lemmas, conllu, tsv, jsonl)\nСправка: rumorph --help\n"
    )
    result = run(str(tmp_path / "нет.txt"))
    assert result.returncode == 1
    assert result.stderr == f"rumorph: {tmp_path / 'нет.txt'}: файл не найден\n"
    broken = tmp_path / "broken.dict"
    broken.write_bytes(b"not a dictionary")
    result = run("--dictionary", str(broken), text=TEXT)
    assert result.returncode == 1
    message = "неверный формат словаря: нет сигнатуры словаря ruMorph"
    assert result.stderr == f"rumorph: {broken}: {message}\n"


def test_dictionary_option(tmp_path):
    path = tmp_path / "ru.dict"
    shutil.copy(rumorph.Analyzer().path, path)
    assert run("-d", str(path), text=TEXT).stdout == "мы стать друг в ковид .\n"


def test_closed_pipe(tmp_path):
    path = tmp_path / "text.txt"
    path.write_text(TEXT * 20_000, encoding="utf-8")
    process = subprocess.Popen(
        [sys.executable, "-m", "rumorph", str(path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert process.stdout is not None and process.stderr is not None
    assert process.stdout.readline().decode() == "мы стать друг в ковид .\n"
    process.stdout.close()
    assert process.wait(timeout=60) == 0
    assert process.stderr.read() == b""


def test_console_script():
    script = shutil.which("rumorph", path=str(Path(sys.executable).parent))
    assert script is not None
    result = subprocess.run([script, "-V"], capture_output=True, encoding="utf-8", check=False)
    assert result.stdout == f"rumorph {rumorph.__version__}\n"


def test_main_in_process(tmp_path, monkeypatch, capfd):
    path = tmp_path / "text.txt"
    path.write_text(TEXT, encoding="utf-8")
    handlers = []
    monkeypatch.setattr(signal, "signal", lambda *args: handlers.append(args))
    monkeypatch.setattr(sys, "argv", ["rumorph", str(path)])
    with pytest.raises(SystemExit) as exit_info:
        command.main()
    assert exit_info.value.code == 0
    assert handlers == [(signal.SIGINT, signal.SIG_DFL)]
    assert capfd.readouterr().out == "мы стать друг в ковид .\n"
    # Игнорируемый SIGINT остается игнорируемым
    handlers.clear()
    monkeypatch.setattr(signal, "getsignal", lambda _: signal.SIG_IGN)
    with pytest.raises(SystemExit):
        command.main()
    assert handlers == []


def test_line_by_line():
    # Команда отвечает на каждую строку, не дожидаясь конца ввода: ее можно держать открытой
    process = subprocess.Popen(
        [sys.executable, "-m", "rumorph"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        encoding="utf-8",
    )
    assert process.stdin is not None and process.stdout is not None
    stdout = process.stdout
    try:
        for _ in range(2):
            process.stdin.write("Мы стали друзьями\n")
            process.stdin.flush()
            lines: list[str] = []
            reader = threading.Thread(target=lambda out=lines: out.append(stdout.readline()))
            reader.start()
            reader.join(timeout=60)
            assert lines == ["мы стать друг\n"]
    finally:
        process.stdin.close()
        process.wait(timeout=60)


def test_output_file_and_encoding(tmp_path):
    path, output = tmp_path / "text.txt", tmp_path / "text.conllu"
    path.write_text(TEXT, encoding="utf-8")
    result = run("-f", "conllu", "-o", str(output), str(path))
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")
    assert (
        output.read_text(encoding="utf-8").splitlines()[1]
        == "# text = Мы стали друзьями в ковиде."
    )
    # Файл в cp1251 с ASCII в начале (субтитры) не разбирается и частично
    subtitles = tmp_path / "cp1251.srt"
    subtitles.write_bytes("1\n00:00:01,000 --> 00:00:02,000\nМама мыла раму\n".encode("cp1251"))
    result = run(str(subtitles))
    assert (result.returncode, result.stdout) == (1, "")
    assert result.stderr == f"rumorph: {subtitles}: файл не в UTF-8 (строка 3)\n"
