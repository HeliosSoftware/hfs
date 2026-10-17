//! Dirty this crate whenever the embedded inputs change.
//!
//! `rust-embed` (with `debug-embed`), askama, and `fluent-templates` all bake
//! their inputs in at compile time, but cargo's incremental tracking does not
//! reliably see asset-only edits — on a persistent CI target dir the stale
//! rlib gets reused and the binary serves whatever assets were on disk the
//! last time this crate actually compiled.

//!
//! It also picks the path the compile-time embeds read. `locales` is a git
//! symlink to the workspace-root catalogs so `cargo publish` packages them, but
//! a Windows checkout without `core.symlinks` materializes it as a plain text
//! file holding the link target (#1257). When the in-crate path is not a
//! directory we set `helios_workspace_locales`, which switches the embed back
//! to the workspace-relative path.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=templates");
    println!("cargo:rerun-if-changed=locales");

    println!("cargo:rustc-check-cfg=cfg(helios_workspace_locales)");
    if !Path::new("locales").is_dir() {
        println!("cargo:rerun-if-changed=../../locales");
        println!("cargo:rustc-cfg=helios_workspace_locales");
    }
}
