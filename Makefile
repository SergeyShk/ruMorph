.PHONY: help uv deps dictionary lock lint ruff format mypy rust-lint test test-cov clean clean-build clean-pyc clean-test clean-rust build docs-build docs-serve docs-deploy
.DEFAULT_GOAL := help
SRC_PATH := python
TESTS_PATH := tests
SCRIPTS_PATH := scripts
PACKAGE := rumorph
DICTIONARY := $(SRC_PATH)/$(PACKAGE)/data/ru.dict
EXPORT_PATH := target/opencorpora
EXPORT_STAMP := $(EXPORT_PATH)/.exported
UD_PATH := target/ud
UD_STAMP := $(UD_PATH)/.exported
CONTEXT := data/ru-context.bin

help: ## Показать список команд
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "%-20s %s\n", $$1, $$2}'

uv: ## Проверить наличие uv и Rust
	@which uv >/dev/null 2>&1 || { \
		echo "uv не установлен. Выполните 'curl -LsSf https://astral.sh/uv/install.sh | sh' или 'brew install uv'"; \
		exit 1; \
	}
	@which cargo >/dev/null 2>&1 || { \
		echo "Rust не установлен. Выполните 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh'"; \
		exit 1; \
	}

deps: uv ## Установить зависимости и собрать модуль расширения
ifeq ($(MODE), ci)
	uv sync --locked --all-groups
else
	uv sync --all-groups
endif

dictionary: $(DICTIONARY) ## Собрать словарь из pymorphy3-dicts-ru и трибанков UD

$(EXPORT_STAMP): $(SCRIPTS_PATH)/export_pymorphy.py uv.lock | deps
	uv run python $(SCRIPTS_PATH)/export_pymorphy.py $(EXPORT_PATH)
	touch $@

$(UD_STAMP): $(SCRIPTS_PATH)/export_ud.py | deps
	uv run python $(SCRIPTS_PATH)/export_ud.py $(UD_PATH)
	touch $@

$(DICTIONARY): $(EXPORT_STAMP) $(UD_STAMP) $(CONTEXT) $(wildcard crates/rumorph/src/*.rs crates/rumorph/src/dictionary/*.rs crates/rumorph-build/src/*.rs)
	cargo run --release --locked -p rumorph-build -- $(EXPORT_PATH) $(UD_PATH) $(DICTIONARY) $(CONTEXT)

lock: uv ## Обновить lock-файлы до последних версий зависимостей
	uv lock --upgrade
	cargo update

lint: ruff mypy rust-lint ## Запустить все проверки кода

ruff: deps ## Проверить и отформатировать код Python с помощью ruff
ifeq ($(MODE), ci)
	uv run ruff check $(SRC_PATH) $(SCRIPTS_PATH) $(TESTS_PATH)
	uv run ruff format $(SRC_PATH) $(SCRIPTS_PATH) $(TESTS_PATH) --check
else
	uv run ruff check $(SRC_PATH) $(SCRIPTS_PATH) $(TESTS_PATH) --fix
	uv run ruff format $(SRC_PATH) $(SCRIPTS_PATH) $(TESTS_PATH)
endif

format: deps ## Отформатировать код
	uv run ruff format $(SRC_PATH) $(SCRIPTS_PATH) $(TESTS_PATH)
	cargo fmt --all

mypy: deps ## Проверить типы с помощью mypy
	uv run mypy

rust-lint: uv ## Проверить код Rust с помощью rustfmt и clippy
ifeq ($(MODE), ci)
	cargo fmt --all --check
else
	cargo fmt --all
endif
	cargo clippy --workspace --all-targets -- -D warnings

test: deps dictionary ## Запустить тесты
	uv run pytest
	cargo test -p $(PACKAGE) -p rumorph-build -p rumorph-dict-ru --all-features

test-cov: deps dictionary ## Запустить тесты с проверкой покрытия
	uv run pytest --cov $(PACKAGE)
	cargo test -p $(PACKAGE) -p rumorph-build -p rumorph-dict-ru --all-features

clean: clean-build clean-pyc clean-test ## Удалить все артефакты, кроме сборки Rust
	rm -f .coverage coverage.xml

clean-build: ## Удалить артефакты сборки пакета
	rm -fr build/ dist/ .eggs/
	find . -name '*.egg-info' -not -path './target/*' -exec rm -fr {} +

clean-pyc: ## Удалить артефакты компиляции Python
	find . -name '*.pyc' -not -path './target/*' -delete
	find . -name '*.pyo' -not -path './target/*' -delete
	find . -name '__pycache__' -type d -not -path './target/*' -exec rm -rf {} + 2>/dev/null || true

clean-test: ## Удалить артефакты тестирования и линтинга
	rm -fr .pytest_cache .mypy_cache .ruff_cache

clean-rust: ## Удалить сборку Rust
	cargo clean

build: clean uv dictionary ## Собрать дистрибутив для текущей платформы
	uv build
	ls -l dist

docs-build: deps ## Собрать документацию
	rm -fr site/
	uv run mkdocs build --strict

docs-serve: deps ## Запустить сервер документации
	uv run mkdocs serve

docs-deploy: deps ## Задеплоить документацию
	uv run mkdocs gh-deploy
