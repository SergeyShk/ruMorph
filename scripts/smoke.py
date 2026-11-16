"""Проверка установленного пакета перед публикацией: разбор, словоизменение, согласование, pickle,
проверка словаря и команда rumorph

Запуск: `python scripts/smoke.py [версия]`; из каталога `scripts` пакет берется установленный,
а не исходники из репозитория.
"""

import pickle
import shutil
import subprocess
import sys
import sysconfig
import tempfile
import zlib
from pathlib import Path

import rumorph
from rumorph import Analyzer

morph = Analyzer()
analysis = morph.analyze("стали")[0]
assert (analysis.lemma, analysis.pos) == ("стать", "VERB"), analysis
assert morph.analyze("кот")[0].inflect(Case="Gen") == "кота"
assert morph.analyze("рубль")[0].agree(5) == "рублей"
assert morph.lemmatize(["Мы", "стали", "друзьями", "в", "ковиде"]) == [
    "мы",
    "стать",
    "друг",
    "в",
    "ковид",
]
assert pickle.loads(pickle.dumps(analysis)) == analysis
assert Analyzer(verify=True).analyze("стол") == morph.analyze("стол")
# Испорченный корень автомата словоформ (секция 11) роняет обход паникой: verify дает ValueError,
# только если колесо собрано с panic = "unwind"
data = morph.path.read_bytes()
at, sections = 16, []
while at < len(data):
    size = int.from_bytes(data[at : at + 8], "little")
    sections.append((at + 8, size))
    at = (at + 8 + size + 7) // 8 * 8
start, size = sections[10]
root = start + int.from_bytes(data[start + size - 12 : start + size - 4], "little")
with tempfile.TemporaryDirectory() as directory:
    for flip in (1, 2, 4, 8, 16, 32, 64, 128):
        damaged = bytearray(data)
        damaged[root] ^= flip
        damaged[12:16] = zlib.crc32(damaged[16:]).to_bytes(4, "little")
        path = Path(directory) / f"root-{flip}.dict"
        path.write_bytes(damaged)
        try:
            Analyzer(path, verify=True)
        except ValueError:
            continue
        raise AssertionError(f"испорченный словарь принят: бит {flip} корня")
assert "site-packages" in rumorph.__file__, rumorph.__file__
# В Python без GIL импорт модуля не должен включать GIL обратно
if sysconfig.get_config_var("Py_GIL_DISABLED"):
    assert not sys._is_gil_enabled()
# Команда: модулем и консольным скриптом рядом с интерпретатором
result = subprocess.run(
    [sys.executable, "-m", "rumorph", "-f", "conllu"],
    input="Мы стали\n",
    capture_output=True,
    encoding="utf-8",
    check=True,
)
assert "2\tстали\tстать\tVERB" in result.stdout, result
script = shutil.which("rumorph", path=str(Path(sys.executable).parent))
assert script is not None, Path(sys.executable).parent
result = subprocess.run([script, "-V"], capture_output=True, encoding="utf-8", check=True)
assert result.stdout == f"rumorph {rumorph.__version__}\n", result
if len(sys.argv) > 1:
    assert rumorph.__version__ == sys.argv[1], (rumorph.__version__, sys.argv[1])
print(f"rumorph {rumorph.__version__}: {morph.path}")
