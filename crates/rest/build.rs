//! Build script: records the git commit the crate was compiled from.
//!
//! The commit is exposed to the crate as the `HFS_BUILD_GIT_SHA` compile-time
//! environment variable, and as a ready-made ` (git <sha>)` suffix in
//! `HFS_BUILD_VERSION_SUFFIX` (see `src/build_info.rs`), so `hfs --version`
//! and `CapabilityStatement.software` can identify a build even for a
//! downloaded release artifact (#992).
//!
//! Resolution order, first hit wins:
//!
//! 1. `HFS_GIT_SHA` in the build environment — lets a packager building from
//!    a source tarball (no `.git`) inject the commit it was cut from.
//! 2. `git rev-parse --short HEAD`, when `git` is on `PATH` and the source
//!    tree is a checkout (a linked worktree counts).
//! 3. Nothing at all: `HFS_BUILD_GIT_SHA` is left unset. This is never an
//!    error — a tarball build with no `git` in it must still compile — and
//!    `src/build_info.rs` reads the variable with `option_env!`, so "unknown"
//!    is its absence rather than an empty-string sentinel.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=HFS_GIT_SHA");

    let sha = std::env::var("HFS_GIT_SHA")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(git_short_sha);

    // Emitted only when a commit is known. With an empty-string sentinel the
    // consumer had to ask `!GIT_SHA_RAW.is_empty()`, and clippy const-folds
    // `is_empty()` on anything reachable through `env!` — so that question
    // had a constant answer in every build, which `clippy::const_is_empty`
    // fails under `-D warnings` (#1185 CI).
    let suffix = match &sha {
        Some(sha) => {
            println!("cargo:rustc-env=HFS_BUILD_GIT_SHA={sha}");
            format!(" (git {sha})")
        }
        None => String::new(),
    };
    println!("cargo:rustc-env=HFS_BUILD_VERSION_SUFFIX={suffix}");
}

/// `git rev-parse --short HEAD` for the crate's own source tree, or `None`
/// when that is not a git checkout (or `git` is unavailable).
///
/// Also registers the files whose change means "HEAD moved" — the `HEAD`
/// file and the ref it points at — so the SHA baked in stays current across
/// commits without rerunning the script on every build. Only files that
/// exist are registered: cargo reruns a script unconditionally for a missing
/// `rerun-if-changed` path.
fn git_short_sha() -> Option<String> {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR")?);

    let git_dir = git_output(&manifest_dir, &["rev-parse", "--git-dir"])?;
    let git_dir = manifest_dir.join(git_dir);
    register_if_exists(&git_dir.join("HEAD"));
    if let Some(head_ref) = git_output(&manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
        // Refs live in the *common* dir when this is a linked worktree.
        let common_dir = git_output(&manifest_dir, &["rev-parse", "--git-common-dir"])
            .map(|d| manifest_dir.join(d))
            .unwrap_or_else(|| git_dir.clone());
        register_if_exists(&common_dir.join(&head_ref));
        register_if_exists(&common_dir.join("packed-refs"));
    }

    git_output(&manifest_dir, &["rev-parse", "--short=9", "HEAD"])
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn register_if_exists(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
