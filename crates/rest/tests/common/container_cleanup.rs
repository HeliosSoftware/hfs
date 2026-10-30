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
//! The hook is a CRT `atexit` handler (`ctor` 0.2). It runs when `main`
//! returns, and on Unix also from `std::process::exit`, but never for a killed
//! process, and not on Windows when any test fails: libtest then calls
//! `std::process::exit(101)`, which on Windows ends the process through
//! `ExitProcess` and skips `atexit` handlers (#2404). So the first
//! [`with_cleanup_label`] call in each process also removes labelled
//! containers whose owner process (the PID before the `-` in the label value)
//! is no longer running.
//!
//! That sweep is skipped when `CI` is set or when the docker CLI's daemon is
//! not a local `unix://` / `npipe://` endpoint: the CI Docker host is shared by
//! runners on other machines, and their PIDs mean nothing here. It also assumes
//! that every process labelling containers on this daemon shares this machine's
//! PID namespace; a dev container that mounts the host's Docker socket breaks
//! that assumption.
//!
//! `crates/persistence/tests/common/container_cleanup.rs` and
//! `crates/rest/tests/common/container_cleanup.rs` are the same file apart from
//! `OWNER_LABEL_KEY`, and must be changed together.
//!
//! `#[path]`-include this file from a test binary; it is not part of
//! `common/mod.rs`.

use std::collections::HashSet;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Once};
use std::time::{SystemTime, UNIX_EPOCH};

use testcontainers::{ContainerRequest, Image, ImageExt};

const OWNER_LABEL_KEY: &str = "io.helios.rest.test-owner";

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

/// Runs [`remove_containers_of_exited_owners`] once per process, before its
/// first label.
static SWEPT: Once = Once::new();

/// Labels a container request so it is force-removed when this process exits.
///
/// The first call in a process also sweeps containers left behind by exited
/// test processes (see the module docs).
pub fn with_cleanup_label<I: Image>(request: impl ImageExt<I>) -> ContainerRequest<I> {
    SWEPT.call_once(remove_containers_of_exited_owners);
    CONTAINER_REQUESTED.store(true, Ordering::SeqCst);
    request.with_label(OWNER_LABEL_KEY, OWNER_LABEL_VALUE.as_str())
}

#[ctor::dtor]
fn remove_labeled_containers() {
    if !CONTAINER_REQUESTED.load(Ordering::SeqCst) {
        return;
    }
    let filter = format!("label={OWNER_LABEL_KEY}={}", *OWNER_LABEL_VALUE);
    let Ok(listing) = Command::new("docker")
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
    // `-v` also removes the container's anonymous volumes (the postgres, mongo
    // and elasticsearch images declare a VOLUME); named volumes are never
    // touched. This matches the CI reap step.
    let _ = Command::new("docker")
        .args(["rm", "-fv"])
        .args(&ids)
        .output();
}

/// Removes labelled containers whose owner process has exited.
///
/// A killed test process runs no exit hook on any OS, and on Windows a run with
/// a failing test skips it too (see the module docs), so those containers would
/// stay for good. Their label value starts with the owner's PID, which tells us
/// whether the owner is gone.
///
/// Runs under [`SWEPT`], so it must never panic: a panic would poison the
/// `Once` and make every later [`with_cleanup_label`] call panic too. Hence no
/// `unwrap`, `expect`, `eprintln!` (it panics when the stderr write fails) or
/// indexing; every failure just skips the sweep.
///
/// A container is removed only when its label parses as `<pid>-<nanos>` and
/// either that PID is not running, or it is this process's own PID under a
/// different label (this process reused the PID, so the process that set the
/// label is gone). This process's own label, containers with a live owner, and
/// labels that do not parse are always kept.
///
/// SAFETY NOTE: PIDs are only meaningful on the machine that runs the tests, so
/// the sweep runs only for a local daemon outside CI (see [`sweep_allowed`]).
/// CI points `DOCKER_HOST` at one Docker host shared by runners on several
/// machines; sweeping there would remove other runs' live containers
/// mid-suite. If that gate were ever fooled, for example a remote daemon's
/// socket forwarded to a local `unix://` path with `CI` unset, the same harm
/// becomes possible.
fn remove_containers_of_exited_owners() {
    let allowed = sweep_allowed(
        std::env::var("CI").ok().as_deref(),
        std::env::var("DOCKER_HOST").ok().as_deref(),
        || {
            command_stdout(
                "docker",
                &[
                    "context",
                    "inspect",
                    "--format",
                    "{{.Endpoints.docker.Host}}",
                ],
            )
        },
    );
    if !allowed {
        return;
    }
    let filter = format!("label={OWNER_LABEL_KEY}");
    let format = format!("{{{{.ID}}}} {{{{.Label \"{OWNER_LABEL_KEY}\"}}}}");
    let Some(listing) = command_stdout(
        "docker",
        &["ps", "-a", "--filter", &filter, "--format", &format],
    ) else {
        return;
    };
    let entries = parse_owner_listing(&listing);
    if entries.is_empty() {
        return;
    }
    let Some(live) = live_pids() else {
        return;
    };
    let stale = dead_owner_ids(
        &entries,
        OWNER_LABEL_VALUE.as_str(),
        std::process::id(),
        |pid| live.contains(&pid),
    );
    if stale.is_empty() {
        return;
    }
    let removed = Command::new("docker")
        .args(["rm", "-fv"])
        .args(&stale)
        .output();
    if removed.is_ok_and(|output| output.status.success()) {
        let _ = writeln!(
            std::io::stderr(),
            "[hfs-test-container-sweep] removed {} container(s) left by exited test processes: {}",
            stale.len(),
            stale.join(" ")
        );
    }
}

/// Whether the start-of-process sweep may run.
///
/// Never in CI (`ci` set and non-empty). Otherwise only when the daemon the
/// docker CLI talks to is local: `docker_host` (the `DOCKER_HOST` variable)
/// when set, else the endpoint of the current docker context, obtained lazily
/// through `current_context_endpoint`. testcontainers ignores CLI contexts, but
/// the sweep drives the CLI, so the CLI's own endpoint is what gets checked.
fn sweep_allowed(
    ci: Option<&str>,
    docker_host: Option<&str>,
    current_context_endpoint: impl FnOnce() -> Option<String>,
) -> bool {
    if ci.is_some_and(|value| !value.trim().is_empty()) {
        return false;
    }
    match docker_host.map(str::trim).filter(|host| !host.is_empty()) {
        Some(host) => is_local_docker_endpoint(host),
        None => current_context_endpoint()
            .is_some_and(|endpoint| is_local_docker_endpoint(endpoint.trim())),
    }
}

/// A Unix socket or a Windows named pipe: an endpoint on this machine.
fn is_local_docker_endpoint(endpoint: &str) -> bool {
    endpoint.starts_with("unix://") || endpoint.starts_with("npipe://")
}

/// Runs `program` and returns its stdout, or `None` when it cannot be started
/// or exits non-zero.
fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// PIDs of every process running on this machine, or `None` when they cannot
/// be listed reliably.
///
/// One listing for the whole machine instead of one lookup per owner PID.
/// `cfg!` instead of `#[cfg]` keeps both branches compiled, and linted, on
/// every OS.
fn live_pids() -> Option<HashSet<u32>> {
    let pids = if cfg!(windows) {
        parse_tasklist_pids(&command_stdout("tasklist", &["/NH", "/FO", "CSV"])?)
    } else {
        parse_ps_pids(&command_stdout("ps", &["-A", "-o", "pid="])?)
    };
    // A listing that is missing this very process was not parsed correctly
    // (locale, format or tool differences), so none of it is trusted.
    pids.contains(&std::process::id()).then_some(pids)
}

/// Parses `docker ps --format '{{.ID}} {{.Label "<key>"}}'` output into
/// `(container id, owner label)` pairs. Containers without a label value are
/// skipped.
fn parse_owner_listing(listing: &str) -> Vec<(&str, &str)> {
    listing
        .lines()
        .filter_map(|line| {
            let (id, owner) = line.trim().split_once(' ')?;
            let owner = owner.trim();
            (!owner.is_empty()).then_some((id, owner))
        })
        .collect()
}

/// The PID from a `<pid>-<nanos>` label value.
fn owner_pid(owner: &str) -> Option<u32> {
    owner.split_once('-')?.0.parse().ok()
}

/// Picks the container ids whose owner has exited, in input order.
///
/// Skips `own_owner` (this process's label) and labels that are not
/// `<pid>-<nanos>`. Selects an entry when its PID is not alive, or when it is
/// `own_pid` under a foreign label: a PID belongs to one live process at a
/// time, so that label was set by a process that has since exited and whose PID
/// this process reused. A dead owner's PID reused by some other live process
/// only keeps its container, which is the safe direction.
fn dead_owner_ids<'a>(
    entries: &[(&'a str, &str)],
    own_owner: &str,
    own_pid: u32,
    is_alive: impl Fn(u32) -> bool,
) -> Vec<&'a str> {
    entries
        .iter()
        .filter_map(|&(id, owner)| {
            if owner == own_owner {
                return None;
            }
            let pid = owner_pid(owner)?;
            (pid == own_pid || !is_alive(pid)).then_some(id)
        })
        .collect()
}

/// Parses `tasklist /NH /FO CSV` output: the PID is the second quoted column.
/// Lines that are not CSV rows, such as the `INFO:` line, are skipped, and
/// image names that contain commas are handled by splitting on `","`.
fn parse_tasklist_pids(listing: &str) -> HashSet<u32> {
    listing
        .lines()
        .filter_map(|line| line.split("\",\"").nth(1)?.trim().parse().ok())
        .collect()
}

/// Parses `ps -A -o pid=` output: one PID per line.
fn parse_ps_pids(listing: &str) -> HashSet<u32> {
    listing
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_owner_listing_reads_id_and_owner() {
        let listing = "0ca87ccb9f8f 24556-1793387658255337500\r\nd9a018e14415 37472-1\n\nabc123 \n";
        assert_eq!(
            parse_owner_listing(listing),
            vec![
                ("0ca87ccb9f8f", "24556-1793387658255337500"),
                ("d9a018e14415", "37472-1"),
            ]
        );
    }

    #[test]
    fn owner_pid_reads_the_pid_before_the_dash() {
        assert_eq!(owner_pid("8328-1793385186889305400"), Some(8328));
        assert_eq!(owner_pid("8328"), None);
        assert_eq!(owner_pid("x-1"), None);
        assert_eq!(owner_pid("-5"), None);
        assert_eq!(owner_pid(""), None);
    }

    #[test]
    fn dead_owner_ids_selects_only_exited_owners() {
        let entries = [
            ("own", "100-5"),
            ("live", "200-1"),
            ("dead", "300-1"),
            ("reused", "100-4"),
            ("junk", "not-a-pid"),
            ("nodash", "400"),
        ];
        let stale = dead_owner_ids(&entries, "100-5", 100, |pid| pid == 100 || pid == 200);
        assert_eq!(stale, vec!["dead", "reused"]);
    }

    #[test]
    fn dead_owner_ids_keeps_containers_whose_pid_is_alive() {
        let entries = [("a", "200-1"), ("b", "300-2"), ("c", "400-3")];
        assert!(dead_owner_ids(&entries, "100-5", 100, |_| true).is_empty());
    }

    #[test]
    fn parse_tasklist_pids_reads_the_pid_column() {
        let listing = concat!(
            "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n",
            "\"a,b.exe\",\"1234\",\"Console\",\"1\",\"12,345 K\"\r\n",
            "INFO: No tasks are running which match the specified criteria.\r\n",
        );
        assert_eq!(parse_tasklist_pids(listing), HashSet::from([0, 1234]));
    }

    #[test]
    fn parse_ps_pids_reads_one_pid_per_line() {
        assert_eq!(
            parse_ps_pids("    1\n   42\n\n  PID\n"),
            HashSet::from([1, 42])
        );
    }

    #[test]
    fn local_docker_endpoints_are_unix_sockets_and_named_pipes() {
        assert!(is_local_docker_endpoint("unix:///var/run/docker.sock"));
        assert!(is_local_docker_endpoint(
            "unix:///run/user/1000/docker.sock"
        ));
        assert!(is_local_docker_endpoint(
            "npipe:////./pipe/dockerDesktopLinuxEngine"
        ));
        assert!(!is_local_docker_endpoint("tcp://10.0.0.5:2375"));
        assert!(!is_local_docker_endpoint("ssh://ci@docker-host"));
        assert!(!is_local_docker_endpoint(""));
    }

    #[test]
    fn sweep_is_skipped_in_ci_and_for_remote_daemons() {
        assert!(!sweep_allowed(Some("true"), None, || Some(
            "unix:///var/run/docker.sock".into()
        )));
        assert!(!sweep_allowed(None, Some("tcp://10.0.0.5:2375"), || {
            unreachable!("DOCKER_HOST wins")
        }));
        assert!(sweep_allowed(None, None, || Some(
            "npipe:////./pipe/dockerDesktopLinuxEngine\n".into()
        )));
        assert!(!sweep_allowed(None, Some("  "), || None));
        assert!(sweep_allowed(
            Some(""),
            Some("unix:///var/run/docker.sock"),
            || None
        ));
    }

    #[test]
    fn live_pids_lists_this_process() {
        let pids = live_pids().expect("the platform process listing should parse");
        assert!(pids.contains(&std::process::id()));
    }

    #[test]
    fn command_stdout_is_none_for_a_missing_program() {
        assert_eq!(command_stdout("hfs-no-such-program-2404", &[]), None);
    }
}
