# Installation

Python 3.11 or newer is required.

``` bash
pip install rumorph
```

## Prebuilt wheels

PyPI hosts wheels for Linux (x86_64 and aarch64; manylinux and musllinux for Alpine), macOS (Intel and Apple Silicon) and Windows (x64 and arm64). One wheel serves every Python version starting with 3.11, the free-threaded Python 3.14t has a wheel of its own; Rust is not needed to install them. The dictionary is part of the package and needs no separate installation. The free-threaded Python 3.13t is not supported.

Pre-release versions `X.Y.Z.devN` are published only on TestPyPI:

``` bash
pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple/ --pre rumorph
```

uv takes a package from the first index that has it, so it needs another order of indexes:

``` bash
uv pip install --extra-index-url https://test.pypi.org/simple/ --index-strategy unsafe-best-match --pre rumorph
```

## Building from source

On other platforms pip builds the package from source, which requires [Rust](https://www.rust-lang.org/tools/install) 1.88 or newer:

``` bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
pip install rumorph
```

The development environment is described in [CONTRIBUTING.md](https://github.com/SergeyShk/ruMorph/blob/master/CONTRIBUTING.md).

## Rust { #rust }

The analyzer core is published on crates.io as the [`rumorph`](https://crates.io/crates/rumorph) crate and the dictionary as the [`rumorph-dict-ru`](https://crates.io/crates/rumorph-dict-ru) crate: it is enabled by default and loaded with `Dictionary::bundled()`. The API documentation is on [docs.rs](https://docs.rs/rumorph).

``` bash
cargo add rumorph
```

`cargo install rumorph --features cli` installs [the rumorph command](cli.md) without Python.
