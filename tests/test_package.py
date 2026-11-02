import importlib
import importlib.metadata
import re
import shutil
import subprocess
import sys
import sysconfig
import tomllib
import zipfile
from pathlib import Path

import pytest

import rumorph

ROOT = Path(__file__).parents[1]


def test_version():
    assert re.fullmatch(r"\d+\.\d+\.\d+(\.dev\d+)?", rumorph.__version__)
    assert rumorph.__version__ != "0.0.0"


def test_metadata():
    assert "морфологический анализатор" in rumorph.__description__
    assert "__version__" in rumorph.__all__
    assert rumorph.__all__ == sorted(rumorph.__all__)


def test_version_matches_metadata():
    # Версия берется из модуля расширения, без медленного importlib.metadata при импорте
    assert rumorph.__version__ == importlib.metadata.version("rumorph")


def test_crate_version():
    with (ROOT / "Cargo.toml").open("rb") as file:
        crate = tomllib.load(file)["workspace"]["package"]["version"]
    with (ROOT / "pyproject.toml").open("rb") as file:
        package = tomllib.load(file)["project"]["version"]
    assert crate == re.sub(r"\.dev(\d+)$", r"-dev.\1", package)
    # Формат словаря меняется вместе с ядром: rumorph требует словарь своей версии
    with (ROOT / "crates" / "rumorph" / "Cargo.toml").open("rb") as file:
        dictionary = tomllib.load(file)["dependencies"]["rumorph-dict-ru"]["version"]
    assert dictionary == f"={crate}"


def test_extension():
    extension = importlib.import_module("rumorph._rumorph")
    assert Path(extension.__file__).parent == Path(rumorph.__file__).parent


def test_package_data():
    assert (Path(rumorph.__file__).parent / "py.typed").is_file()


@pytest.mark.skipif(shutil.which("uv") is None, reason="колесо собирает uv")
def test_wheel_contents(tmp_path):
    subprocess.run(
        [
            "uv",
            "build",
            "--wheel",
            "--python",
            sys.executable,
            "--out-dir",
            str(tmp_path),
            str(ROOT),
        ],
        capture_output=True,
        check=True,
    )
    (wheel,) = tmp_path.glob("*.whl")
    assert ("-abi3-" in wheel.name) != bool(sysconfig.get_config_var("Py_GIL_DISABLED"))
    with zipfile.ZipFile(wheel) as archive:
        names = set(archive.namelist())
    assert {"rumorph/py.typed", "rumorph/data/ru.dict"} <= names
    modules = [
        name for name in names if re.fullmatch(r"rumorph/_rumorph(\.[\w-]+)?\.(so|pyd)", name)
    ]
    assert len(modules) == 1
    assert all(name.startswith(("rumorph/", "rumorph-")) for name in names)
    licenses = {name.rsplit("/", 1)[1] for name in names if "/licenses/" in name}
    assert licenses == {"LICENSE.txt", "LICENSE-DICTIONARY.txt", "THIRD-PARTY-NOTICES.txt"}


@pytest.mark.skipif(shutil.which("cargo") is None, reason="крейты читает cargo")
def test_third_party_notices(tmp_path):
    # Лицензии крейтов модуля следуют Cargo.lock: после его изменения - python scripts/notices.py
    path = tmp_path / "notices.txt"
    subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "notices.py"), str(path)],
        capture_output=True,
        check=True,
    )
    expected = (ROOT / "THIRD-PARTY-NOTICES.txt").read_text(encoding="utf-8")
    assert path.read_text(encoding="utf-8") == expected, "python scripts/notices.py"
