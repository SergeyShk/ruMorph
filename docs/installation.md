# Установка

Требуется Python 3.11 или новее.

``` bash
pip install rumorph
```

## Готовые колеса

На PyPI публикуются колеса для Linux (x86_64 и aarch64, manylinux), macOS (Intel и Apple Silicon) и Windows (x64). Одно колесо подходит для всех версий Python начиная с 3.11, Rust для установки не нужен. Для Python 3.14t без GIL готовых колес пока нет: пакет собирается из исходников, как описано ниже; Python 3.13t без GIL не поддерживается.

Предварительные версии `0.1.0.devN` публикуются только на TestPyPI:

``` bash
pip install --index-url https://test.pypi.org/simple/ --extra-index-url https://pypi.org/simple/ rumorph==0.1.0.dev1
```

## Сборка из исходников

Для других платформ и Python 3.14t без GIL pip соберет пакет из исходников; для этого нужен [Rust](https://www.rust-lang.org/tools/install) 1.88 или новее:

``` bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
pip install rumorph
```

Окружение для разработки описано в [CONTRIBUTING.md](https://github.com/SergeyShk/ruMorph/blob/master/CONTRIBUTING.md).
