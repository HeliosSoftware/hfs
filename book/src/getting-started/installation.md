# Installation

## Option 1: Pre-built Binaries

Download the latest release archive for your platform from the [GitHub Releases](https://github.com/HeliosSoftware/hfs/releases) page. Extract it and the binaries (`hfs`, `fhirpath-cli`, `fhirpath-server`, `sof-cli`, `sof-server`) are ready to use.

## Option 2: Docker

See the [Docker](docker.md) page for pull-and-run instructions with no build step required.

## Option 3: Build from Source

### Prerequisites

**1. Install Rust**

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**2. Install LLD**

*Linux (Ubuntu/Debian):*
```bash
sudo apt install clang lld
```

*Windows:* Download a pre-built binary from the [llvm-project GitHub releases](https://github.com/llvm/llvm-project/releases).

*macOS:* LLD is not required.

**3. Configure `~/.cargo/config.toml`**

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

### Build

```bash
# Clone the repository
git clone https://github.com/HeliosSoftware/hfs.git
cd hfs

# Build release binaries (R4 only by default)
cargo build --release

# Build with all FHIR versions
cargo build --release --features R4,R4B,R5,R6
```

Binaries land in `target/release/`.

> **Tip:** On memory-constrained machines, limit parallel compile jobs:
> ```bash
> export CARGO_BUILD_JOBS=4
> ```

> **Note:** Build times can exceed 10 minutes for full workspace builds, especially with all FHIR versions or when running the FHIR code generator.
