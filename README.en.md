<h1 align="center">ruMorph</h1>

<p align="center">
  A fast Russian morphological analyzer written in Rust with a Python API
</p>

<p align="center">
  <a href="https://sergeyshk.github.io/ruMorph/en/">Documentation</a> ·
  <a href="https://github.com/SergeyShk/ruMorph/blob/master/README.md">Русский</a>
</p>

<p align="center">
  <a href="https://github.com/SergeyShk/ruMorph/actions/workflows/ci.yml"><img src="https://github.com/SergeyShk/ruMorph/actions/workflows/ci.yml/badge.svg" alt="Build"></a>
  <a href="https://github.com/astral-sh/ruff"><img src="https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json" alt="Ruff"></a>
  <a href="https://github.com/SergeyShk/ruMorph/blob/master/LICENSE.txt"><img src="https://img.shields.io/github/license/sergeyshk/rumorph.svg" alt="License"></a>
</p>

---

**ruMorph** is a morphological analyzer of Russian: a Rust core with a Python API. For a word form the analyzer finds its analyses in the OpenCorpora dictionary - the normal form (lemma), the part of speech and the grammatical features - and orders them by probability.

> [!WARNING]
> Work in progress: so far the package contains only the scaffolding; the analyzer comes in the next versions. Versions `0.1.0.devN` are published on TestPyPI.

## Installation

Python 3.11 or newer is required.

```bash
pip install rumorph
```

Wheels are built for Linux, macOS and Windows; on other platforms building from source requires [Rust](https://www.rust-lang.org/tools/install). See the [Installation](https://sergeyshk.github.io/ruMorph/en/installation/) page.

## Development

The project uses [uv](https://docs.astral.sh/uv/) to manage dependencies, [maturin](https://www.maturin.rs/) to build the Rust module and [ruff](https://docs.astral.sh/ruff/) for linting and formatting. [Rust](https://www.rust-lang.org/tools/install) 1.88 or newer is required.

```bash
git clone https://github.com/SergeyShk/ruMorph.git
cd ruMorph

make deps        # create the environment, install the dependencies and build the Rust module
make test        # run the Python tests, the docstring examples (doctest) and the Rust tests
make lint        # ruff, mypy, rustfmt and clippy
```

The full list of commands is `make help`. The documentation is bilingual: Russian pages `docs/*.md` with English `docs/*.en.md` next to them; update both versions when editing a page.

Before submitting changes, install the hooks that run the linters on commit and the tests on push:

```bash
uv run pre-commit install
```

## Contributing

Bug reports, ideas and pull requests are welcome - the [issues](https://github.com/SergeyShk/ruMorph/issues) are open. The workflow and the checks to run before submitting a pull request are in [CONTRIBUTING.md](https://github.com/SergeyShk/ruMorph/blob/master/CONTRIBUTING.md); the rules of conduct are in the [code of conduct](https://github.com/SergeyShk/ruMorph/blob/master/CODE_OF_CONDUCT.md).

<details>
<summary><b>Project structure</b></summary>

<br>

*   **crates/rumorph** - the analyzer core in Rust
*   **crates/rumorph-py** - the Python module `rumorph._rumorph` built with PyO3
*   **python/rumorph** - the Python package
*   **docs** - MkDocs documentation in Russian and English
*   **tests** - tests of the Python package

</details>

## Authors

*   Sergey Shkarin (kouki.sergey@gmail.com)

## License

The code is distributed under the [MIT](https://github.com/SergeyShk/ruMorph/blob/master/LICENSE.txt) license.
