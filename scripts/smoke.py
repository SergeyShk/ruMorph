"""Проверка установленного пакета перед публикацией: разбор, словоизменение, согласование и pickle

Запуск: `python scripts/smoke.py [версия]`; из каталога `scripts` пакет берется установленный,
а не исходники из репозитория.
"""

import pickle
import sys
import sysconfig

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
if len(sys.argv) > 1:
    assert rumorph.__version__ == sys.argv[1], (rumorph.__version__, sys.argv[1])
print(f"rumorph {rumorph.__version__}: {morph.path}")
