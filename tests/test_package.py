import importlib
import importlib.metadata
import re
import shutil
import subprocess
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


def test_version_fallback(monkeypatch):
    def missing(name):
        raise importlib.metadata.PackageNotFoundError(name)

    monkeypatch.setattr(importlib.metadata, "version", missing)
    try:
        assert importlib.reload(rumorph).__version__ == "0.0.0"
    finally:
        monkeypatch.undo()
        importlib.reload(rumorph)
    assert rumorph.__version__ != "0.0.0"


def test_extension():
    extension = importlib.import_module("rumorph._rumorph")
    assert Path(extension.__file__).parent == Path(rumorph.__file__).parent


def test_package_data():
    assert (Path(rumorph.__file__).parent / "py.typed").is_file()


@pytest.mark.skipif(shutil.which("uv") is None, reason="колесо собирает uv")
def test_wheel_contents(tmp_path):
    subprocess.run(
        ["uv", "build", "--wheel", "--out-dir", str(tmp_path), str(ROOT)],
        capture_output=True,
        check=True,
    )
    (wheel,) = tmp_path.glob("*.whl")
    with zipfile.ZipFile(wheel) as archive:
        names = set(archive.namelist())
    assert "rumorph/py.typed" in names
    assert any(re.fullmatch(r"rumorph/_rumorph\.(abi3\.so|pyd)", name) for name in names)
    assert all(name.startswith(("rumorph/", "rumorph-")) for name in names)
