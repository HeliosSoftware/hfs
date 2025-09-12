# Wheel Building and Distribution

This document describes how to build and distribute Python wheels for the pysof package.

## Overview

The pysof package uses [maturin](https://github.com/PyO3/maturin) to build Python wheels from Rust code. This enables cross-platform distribution of the compiled Rust extension module.

## Supported Platforms

- **Linux**: x86_64 (glibc and musl)
- **Windows**: x86_64 (MSVC)
- **macOS**: x86_64 and ARM64 (when self-hosted runners are available)

## Local Development

### Prerequisites

- Rust toolchain (stable)
- Python 3.11
- [uv](https://github.com/astral-sh/uv) package manager
- [maturin](https://github.com/PyO3/maturin)

### Building Wheels Locally

#### Using the Makefile (Linux/macOS)

```bash
# Build release wheel
make build

# Build debug wheel
make build-debug

# Build and test wheel
make test-wheel

# Clean build artifacts
make clean
```

#### Using maturin directly

```bash
# Build release wheel
uv run maturin build --release --out dist

# Build debug wheel
uv run maturin build --out dist

# Build for specific target
uv run maturin build --release --target x86_64-unknown-linux-gnu --out dist
```

#### Using the build script

```bash
# Build wheel
python scripts/build-wheels.py

# Build and test
python scripts/build-wheels.py --test

# Build for specific target
python scripts/build-wheels.py --target x86_64-unknown-linux-gnu

# Clean build artifacts
python scripts/build-wheels.py --clean
```

### Testing Wheels

After building a wheel, you can test it:

```bash
# Install the wheel
uv pip install dist/*.whl --force-reinstall

# Test basic functionality
python -c "import pysof; print(f'pysof version: {pysof.__version__}')"

# Run tests
python -m pytest tests/test_import.py tests/test_package_metadata.py -v
```

## CI/CD Pipeline

### GitHub Actions Workflow

The wheel building is automated via GitHub Actions in `.github/workflows/pysof-wheels.yml`.

#### Triggers

- **Push to main/epic/pysof branches**: Builds wheels for testing
- **Push tags starting with `pysof-v*`**: Creates releases and optionally publishes to PyPI
- **Pull requests**: Builds and tests wheels
- **Manual dispatch**: Allows manual triggering with PyPI publishing option

#### Build Matrix

The workflow builds wheels for multiple platforms:

- Linux x86_64 (glibc and musl)
- Windows x86_64
- macOS x86_64 and ARM64

#### Artifacts

- Individual wheel files for each platform
- Combined release archive
- GitHub release with all wheels

### Publishing to PyPI

#### Prerequisites

1. PyPI API token in repository secrets as `PYPI_API_TOKEN`
2. Tagged release with format `pysof-v*`

#### Publishing Process

1. Create a tag: `git tag pysof-v0.1.0`
2. Push the tag: `git push origin pysof-v0.1.0`
3. The workflow will automatically:
   - Build wheels for all platforms
   - Create a GitHub release
   - Optionally publish to PyPI (if manually triggered with publish option)

#### Manual PyPI Publishing

To publish to PyPI manually:

1. Go to the GitHub Actions tab
2. Find the "Build and Publish pysof Wheels" workflow
3. Click "Run workflow"
4. Check the "Publish to PyPI" option
5. Click "Run workflow"

## Configuration

### pyproject.toml

The wheel building is configured in `pyproject.toml`:

```toml
[build-system]
requires = ["maturin>=1.0,<2.0"]
build-backend = "maturin"

[tool.maturin]
features = ["pyo3/extension-module"]
module-name = "pysof._pysof"
python-source = "src"
```

### Maturin Options

Key maturin options used:

- `--release`: Build in release mode (optimized)
- `--target`: Cross-compile for specific target
- `--out dist`: Output directory for wheels
- `--compatibility`: Platform compatibility settings

## Cross-Compilation

### Linux Targets

```bash
# Standard glibc
maturin build --target x86_64-unknown-linux-gnu

# Musl (for Alpine Linux)
maturin build --target x86_64-unknown-linux-musl
```

### Windows Targets

```bash
# MSVC toolchain
maturin build --target x86_64-pc-windows-msvc
```

### macOS Targets

```bash
# Intel Macs
maturin build --target x86_64-apple-darwin

# Apple Silicon Macs
maturin build --target aarch64-apple-darwin
```

## Troubleshooting

### Common Issues

1. **TOML parse errors**: Check `pyproject.toml` syntax
2. **Missing Rust targets**: Install with `rustup target add <target>`
3. **Cross-compilation failures**: Ensure appropriate toolchains are installed
4. **Wheel installation failures**: Check Python version compatibility

### Debug Mode

For debugging, build in debug mode:

```bash
maturin build --out dist  # Debug mode
```

### Verbose Output

Add `--verbose` flag for detailed build output:

```bash
maturin build --release --verbose --out dist
```

## File Structure

```
crates/pysof/
├── dist/                    # Built wheels
├── scripts/
│   └── build-wheels.py     # Build script
├── Makefile                # Build commands
├── pyproject.toml          # Package configuration
└── WHEEL_BUILDING.md       # This file
```

## Best Practices

1. **Always test wheels** before publishing
2. **Use release mode** for production wheels
3. **Clean build artifacts** between builds
4. **Verify platform compatibility** on target systems
5. **Keep dependencies minimal** in wheel builds
6. **Use semantic versioning** for releases

## Security Considerations

- PyPI API tokens should be stored as repository secrets
- Wheels are built in isolated CI environments
- Source code is verified before building
- All dependencies are pinned to specific versions
