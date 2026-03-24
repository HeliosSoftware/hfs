# Installation

## Prerequisites: Rust, LLD, Cargo

To build HFS from source you need:

1. **Rust** (edition 2024, minimum version 1.90) — installed via `rustup`
2. **LLD** linker — required on Linux and Windows for fast incremental builds (not required on macOS)
3. **Cargo** — comes with Rust automatically

Alternatively, use [pre-built binaries](#option-1-pre-built-binaries) or [Docker](#option-2-docker) to skip the build entirely.

---

## Option 1: Pre-built Binaries

Download the latest release archive for your platform from the [GitHub Releases](https://github.com/HeliosSoftware/hfs/releases) page. Extract it; the binaries (`hfs`, `fhirpath-cli`, `fhirpath-server`, `sof-cli`, `sof-server`) are ready to use with no build step.

---

## Option 2: Docker

See the [Docker page](getting-started/docker.md) for pull-and-run instructions that require no build step.

---

## Option 3: Build from Source

### Installing Rust via rustup

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Follow the on-screen prompts. After installation, open a new terminal and verify:

```bash
rustc --version   # should print 1.90 or newer
cargo --version
```

### Installing LLD

LLD is the LLVM linker. It dramatically reduces link times on Linux and Windows.

**Linux (Ubuntu / Debian):**
```bash
sudo apt install clang lld
```

**Linux (Fedora / RHEL):**
```bash
sudo dnf install clang lld
```

**Windows:**
Download a pre-built binary from the [LLVM project GitHub releases](https://github.com/llvm/llvm-project/releases). Choose the `LLVM-*-win64.exe` installer and ensure `lld-link.exe` is on your `PATH`.

**macOS:**
LLD is not required. The default Apple linker works well. No extra installation needed.

---

### Configuring `~/.cargo/config.toml`

Add the following to `~/.cargo/config.toml` (create the file if it does not exist) to enable LLD on Linux and Windows and tune stack sizes:

```toml
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=lld", "-C", "link-arg=-Wl,-zstack-size=8388608"]

[target.aarch64-apple-darwin]
linker = "clang"
rustflags = [
  "-C", "link-arg=-Wl,-dead_strip",
  "-C", "link-arg=-undefined",
  "-C", "link-arg=dynamic_lookup"
]

[target.x86_64-pc-windows-msvc]
linker = "lld-link.exe"
rustflags = ["-C", "link-arg=/STACK:8388608"]
```

---

### Setting Environment Variables

On memory-constrained machines (less than ~8 GB RAM), limit the number of parallel compile jobs to avoid OOM errors:

```bash
export CARGO_BUILD_JOBS=4
```

---

### Cloning the Repository

```bash
git clone https://github.com/HeliosSoftware/hfs.git
cd hfs
```

---

### Running Your First Build

**Default build** (R4 only, all crates except `pysof`):
```bash
cargo build --release
```

**Build with all FHIR versions:**
```bash
cargo build --release --features R4,R4B,R5,R6
```

Release binaries land in `target/release/`:
- `hfs` — FHIR REST server
- `fhirpath-cli` — FHIRPath expression evaluator
- `fhirpath-server` — FHIRPath HTTP server
- `sof-cli` — SQL-on-FHIR CLI
- `sof-server` — SQL-on-FHIR HTTP server
- `config-advisor` — Storage configuration advisor

---

### Troubleshooting Common Errors

**Build takes more than 10 minutes**
This is expected on a first build. The `helios-fhir` crate contains large auto-generated files. Subsequent builds are much faster. Use `--features R4` (the default) to avoid compiling the R4B/R5/R6 models unless you need them.

**R6 spec download fails during build**
R6 StructureDefinition files are downloaded automatically from `https://build.fhir.org/` when the `R6` feature is enabled. This requires internet access. If you are in an air-gapped environment, use `--features R4,R4B,R5` and omit R6, or add the `skip-r6-download` feature flag.

**`linker 'cc' not found` on Linux**
Install build essentials:
```bash
sudo apt install build-essential
```

**`error: linking with 'lld-link.exe' failed` on Windows**
Ensure `lld-link.exe` is on your `PATH`. Re-check the LLVM installer and open a fresh terminal.

**`cargo: command not found`**
`rustup` installs Cargo into `~/.cargo/bin`. Add it to your shell path:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
```
Add this line to your `~/.bashrc` or `~/.zshrc`.
