# Makefile for pysof wheel building and development

.PHONY: help build test clean install dev-install lint format typecheck

# Default target
help:
	@echo "Available targets:"
	@echo "  build          - Build wheel in release mode"
	@echo "  build-debug    - Build wheel in debug mode"
	@echo "  test           - Run tests"
	@echo "  test-wheel     - Build wheel and test it"
	@echo "  clean          - Clean build artifacts"
	@echo "  install        - Install wheel in current environment"
	@echo "  dev-install    - Install in development mode"
	@echo "  lint           - Run linting checks"
	@echo "  format         - Format code"
	@echo "  typecheck      - Run type checking"

# Build wheel in release mode
build:
	maturin build --release --out dist

# Build wheel in debug mode
build-debug:
	maturin build --out dist

# Run tests
test:
	python -m pytest tests/ -v

# Build wheel and test it
test-wheel: build
	pip install dist/*.whl --force-reinstall
	python -c "import pysof; print(f'pysof version: {pysof.__version__}')"
	python -m pytest tests/test_import.py tests/test_package_metadata.py -v

# Clean build artifacts
clean:
	rm -rf dist/ target/ build/ .pytest_cache/ .mypy_cache/ .ruff_cache/

# Install wheel in current environment
install: build
	pip install dist/*.whl --force-reinstall

# Install in development mode
dev-install:
	maturin develop

# Run linting checks
lint:
	ruff check src tests
	ruff format --check src tests

# Format code
format:
	ruff format src tests

# Run type checking
typecheck:
	mypy src

# Cross-compilation targets (requires appropriate Rust targets)
build-linux:
	maturin build --release --target x86_64-unknown-linux-gnu --out dist

build-windows:
	maturin build --release --target x86_64-pc-windows-msvc --out dist

build-macos:
	maturin build --release --target x86_64-apple-darwin --out dist

# Build all platforms (if you have cross-compilation set up)
build-all: build-linux build-windows build-macos
