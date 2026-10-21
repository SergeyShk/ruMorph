"""Проверка установленного пакета перед публикацией: разбор, словоизменение, согласование, pickle
и команда rumorph

Запуск: `python scripts/smoke.py [версия]`; из каталога `scripts` пакет берется установленный,
а не исходники из репозитория.
"""

import pickle
import shutil
import subprocess
import sys
import sysconfig
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
