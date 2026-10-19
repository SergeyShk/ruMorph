# Installation

Python 3.11 or newer is required.

``` bash
pip install rumorph
```

## Prebuilt wheels

PyPI hosts wheels for Linux (x86_64 and aarch64, manylinux), macOS (Intel and Apple Silicon) and Windows (x64). One wheel serves every Python version starting with 3.11; Rust is not needed to install it. The dictionary is part of the package and needs no separate installation. There are no prebuilt wheels for the free-threaded Python 3.14t yet: the package is built from source as described below; the free-threaded Python 3.13t is not supported.

Pre-release versions `0.1.0.devN` are published only on TestPyPI:

``` bash
pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple/ --pre rumorph
```

## Building from source

On other platforms and the free-threaded Python 3.14t pip builds the package from source, which requires [Rust](https://www.rust-lang.org/tools/install) 1.88 or newer:

``` bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
pip install rumorph
```

The development environment is described in [CONTRIBUTING.md](https://github.com/SergeyShk/ruMorph/blob/master/CONTRIBUTING.md).
