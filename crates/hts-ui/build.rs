//! Dirty this crate whenever the embedded inputs change.
//!
//! `rust-embed` (with `debug-embed`), askama, and `fluent-templates` all bake
//! their inputs in at compile time, but cargo's incremental tracking does not
//! reliably see asset-only edits — on a persistent CI target dir the stale
//! rlib gets reused and the binary serves whatever assets were on disk the
//! last time this crate actually compiled.
//!
//! During Phase 1 (pre-#543) the `Assets` embed points at `../ui/assets`, so
//! we also watch the sibling ui crate's asset directory for changes. Phase 8
//! (post-#543) unifies these under `helios-ui-chrome` and this extra watch
//! line can be dropped.

fn main() {
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=templates");
    println!("cargo:rerun-if-changed=../../locales");
    println!("cargo:rerun-if-changed=../ui/assets");
}
