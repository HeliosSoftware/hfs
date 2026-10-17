//! Build script for the `pysof` Python extension module.

fn main() {
    // `pysof` is a Python extension module: it intentionally does not link
    // libpython and resolves the `Py*` symbols from the host interpreter at
    // import time. macOS rejects undefined symbols in a dylib unless the
    // linker is told `-undefined dynamic_lookup`. maturin passes that itself,
    // but a plain `cargo build --workspace` (the release build) does not, so
    // emit it here. It is scoped to this crate's cdylib link only — applying it
    // globally via rustflags breaks executables on Apple Silicon. No-op on
    // Linux and Windows.
    pyo3_build_config::add_extension_module_link_args();
}
