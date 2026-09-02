//! Dirty this crate whenever the embedded inputs change.
//!
//! `rust-embed` (with `debug-embed`), askama, and `fluent-templates` all bake
//! their inputs in at compile time, but cargo's incremental tracking does not
//! reliably see asset-only edits — on a persistent CI target dir the stale
//! rlib gets reused and the binary serves whatever assets were on disk the
//! last time this crate actually compiled.
//!
//! The `Assets` embed points at `../ui/assets`, so we also watch the sibling
//! ui crate's asset directory. That line **stays**: `helios-ui-chrome` now
//! exists (#799), but it owns shared *markup* only — the asset embed is
//! unchanged, and moving those bytes is still gated on #543.
//!
//! No watch for `../ui-chrome` is needed, and adding one would be noise:
//! askama expands to an `include_bytes!` per template inside that crate, so
//! rustc records each one as a dependency file and cargo rebuilds this crate's
//! dependents on its own when a shared partial changes.

fn main() {
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=templates");
    println!("cargo:rerun-if-changed=../../locales");
    println!("cargo:rerun-if-changed=../ui/assets");
}
