//! Dirty this crate whenever the embedded inputs change.
//!
//! `rust-embed` (with `debug-embed`), askama, and `fluent-templates` all bake
//! their inputs in at compile time, but cargo's incremental tracking does not
//! reliably see asset-only edits — on a persistent CI target dir the stale
//! rlib gets reused and the binary serves whatever assets were on disk the
//! last time this crate actually compiled.

fn main() {
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=templates");
    println!("cargo:rerun-if-changed=../../locales");
}
