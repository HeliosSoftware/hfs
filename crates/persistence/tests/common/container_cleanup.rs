//! Process-exit cleanup for testcontainers held in `static`s.
//!
//! Shared containers live in a `static OnceCell` so every test in a binary
//! reuses one instance. `static` values are never dropped, so
//! `ContainerAsync`'s `Drop` cleanup never runs and the container outlives the
//! test process (testcontainers-rs has no Ryuk reaper; the `watchdog` feature
//! only covers SIGINT/SIGTERM). Label every shared container with
//! [`with_cleanup_label`] and the `#[ctor::dtor]` hook below force-removes
//! them via the `docker` CLI when the process exits.
//!
//! The label value is unique per process, so the hook never removes containers
//! owned by another test binary running concurrently on the same Docker host.
//!
//! `#[path]`-include this file from a test binary; it is not part of
//! `common/mod.rs`.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use testcontainers::{ContainerRequest, Image, ImageExt};

const OWNER_LABEL_KEY: &str = "io.helios.persistence.test-owner";

static OWNER_LABEL_VALUE: LazyLock<String> = LazyLock::new(|| {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
});

/// Set once a labeled container may exist, so binaries that never start one
/// (e.g. filtered or skipped runs) do not shell out to `docker` at exit.
static CONTAINER_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Labels a container request so it is force-removed when this process exits.
pub fn with_cleanup_label<I: Image>(request: impl ImageExt<I>) -> ContainerRequest<I> {
    CONTAINER_REQUESTED.store(true, Ordering::SeqCst);
    request.with_label(OWNER_LABEL_KEY, OWNER_LABEL_VALUE.as_str())
}

#[ctor::dtor]
fn remove_labeled_containers() {
    if !CONTAINER_REQUESTED.load(Ordering::SeqCst) {
        return;
    }
    let filter = format!("label={OWNER_LABEL_KEY}={}", *OWNER_LABEL_VALUE);
    let Ok(listing) = std::process::Command::new("docker")
        .args(["ps", "-aq", "--filter", &filter])
        .output()
    else {
        return;
    };
    let listing = String::from_utf8_lossy(&listing.stdout);
    let ids: Vec<&str> = listing.split_whitespace().collect();
    if ids.is_empty() {
        return;
    }
    let _ = std::process::Command::new("docker")
        .args(["rm", "-f"])
        .args(&ids)
        .output();
}
