# Установка

Требуется Python 3.11 или новее.

``` bash
pip install rumorph
```

## Готовые колеса

На PyPI публикуются колеса для Linux (x86_64 и aarch64; manylinux и musllinux - для Alpine), macOS (Intel и Apple Silicon) и Windows (x64 и arm64). Одно колесо подходит для всех версий Python начиная с 3.11, для Python 3.14t без GIL - отдельное колесо; Rust для установки не нужен. Словарь входит в пакет, отдельно его устанавливать не нужно. Python 3.13t без GIL не поддерживается.

Предварительные версии `X.Y.Z.devN` публикуются только на TestPyPI:

``` bash
pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple/ --pre rumorph
```

## Сборка из исходников

Для других платформ pip соберет пакет из исходников; для этого нужен [Rust](https://www.rust-lang.org/tools/install) 1.88 или новее:

``` bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
pip install rumorph
```

Окружение для разработки описано в [CONTRIBUTING.md](https://github.com/SergeyShk/ruMorph/blob/master/CONTRIBUTING.md).

## Rust { #rust }

Ядро анализатора публикуется на crates.io крейтом [`rumorph`](https://crates.io/crates/rumorph), словарь - крейтом [`rumorph-dict-ru`](https://crates.io/crates/rumorph-dict-ru): он подключается по умолчанию и загружается `Dictionary::bundled()`. Документация API - на [docs.rs](https://docs.rs/rumorph).

``` bash
cargo add rumorph
```

[Команду rumorph](cli.md) без Python ставит `cargo install rumorph --features cli`.
