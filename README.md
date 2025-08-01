[![Helios FHIR Server](https://github.com/HeliosSoftware/hfs/blob/main/github-banner.png)](https://heliossoftware.com)

# Helios FHIR Server

The Helios FHIR Server is an implementation of the [HL7® FHIR®](https://hl7.org/fhir) standard, built in Rust for high performance and optimized for clinical analytics workloads. It is composed of modular components that can be run as standalone command-line tools, integrated as microservices, and embedded directly into your data analytics pipeline. Whether you're working on a laptop or scaling up to petabyte-scale, serverless cloud clusters, the Helios FHIR Server helps you quickly develop high-quality healthcare solutions.


# Quick Start

1. Install [Rust](https://www.rust-lang.org/tools/install)
```
# recommended approach from https://www.rust-lang.org/tools/install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

2. Install [LDD](https://lld.llvm.org/)

Linux (Ubuntu)
```
sudo apt install clang lld
```
Mac
```
brew install ldd
```
3.  Modify `~/.cargo/confix.toml` to use ldd
```
[target.x86_64-unknown-linux-gnu]
linker = "clang"
rustflags = ["-C", "link-arg=-fuse-ld=lld"]
```
4.  Modify your `~/.bashrc` (Linux) or `~/.zshrc` (Mac) and add:
````
export RUST_MIN_STACK=8388608
````
Then `source` your `~./bashrc` or `~/.zshrc`

5.  Clone this repo:
```
git clone git@github.com:HeliosSoftware/hfs.git
```
6.  Build
This command will build the R4 version only by default.  See features to build with other FHIR version support.
```
cargo build
```
7.  Run
```
TODO
```

NOTE:  If you find you are running out of memory on a Linux machine during complilation, limit the number of build jobs using:
```
export CARGO_BUILD_JOBS=4
```

# Architecture Overview


# Features

List of FHIR versions supported
FHIRPath
FHIR Rest API
SQL On FHIR

- Document: Build process:
```
cargo build -p fhir_gen --features R6
./target/debug/fhir_gen --all
cargo test --features R4,R4B,R5,R6
```
# Documentation

## Product Documentation

TODO

## Source Code Documentation

To view our [rustdoc](https://doc.rust-lang.org/rustdoc/) documentation in a browser, run
```
cargo doc --no-deps --open
```
# Running Tests
```
cargo test
```
