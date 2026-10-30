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
//! The owner label value is unique per process, so the hook never removes
//! containers owned by another test binary running concurrently on the same
//! Docker host.
//!
//! # Exit paths
//!
//! The hook is a CRT `atexit` handler (`ctor` 0.2). It runs when `main`
//! returns, and on Unix also from `std::process::exit`, but never for a killed
//! process. It does not run on Windows when any test fails either: libtest then
//! calls `std::process::exit(101)`, which on Windows ends the process through
//! `ExitProcess`. That skips the CRT `atexit` table where ctor 0.2's `#[dtor]`
//! lives (#2404).
//!
//! A failing run is deliberately not fixed at exit:
//!
//! - After `ExitProcess`, the only user code that still runs is DLL and TLS
//!   detach callbacks. They run under the loader lock after every other thread
//!   has been terminated, so spawning `docker` and waiting on it there can
//!   deadlock.
//! - A reaper child process would be killed together with cargo's job object.
//!
//! So the start-of-run sweep below is the fix for failing and killed runs, and
//! the `#[dtor]` covers normal exits.
//!
//! # Start-of-run sweep
//!
//! The first [`with_cleanup_label`] call in each process removes labelled
//! containers whose owner process is no longer running, provided that owner
//! lived in this process's PID namespace.
//!
//! Every labelled container carries two labels:
//!
//! - `OWNER_LABEL_KEY`: `<pid>-<nanos>`, unique per process.
//! - `PIDNS_LABEL_KEY`: the process-ID namespace the PID belongs to, see
//!   [`pid_namespace_id`]: `windows:<computer name>`,
//!   `linux:<hostname>:<pid namespace link, e.g. pid:[4026531836]>`, or
//!   `<os>:<hostname>` elsewhere.
//!
//! A PID only means something inside its own namespace. Docker Desktop gives
//! every WSL2 distro the same engine as Windows (`unix:///var/run/docker.sock`
//! inside the distro), and a dev container that mounts the host's Docker socket
//! shares the engine too. Both pass [`is_local_docker_endpoint`], but each side
//! looks the other's PIDs up in its own process table. Without the namespace
//! label a Windows run could remove a live WSL run's shared container, or the
//! other way round.
//!
//! The selection rule: a container is removed only when its namespace label
//! EQUALS this process's namespace AND its owner is dead (see
//! [`dead_owner_ids`]). A container with no namespace label, made before the
//! label existed or by another branch's binary, is never swept.
//!
//! The sweep is skipped when `CI` is set or when the docker CLI's daemon is not
//! a local `unix://` / `npipe://` endpoint: the CI Docker host is shared by
//! runners on other machines, and their PIDs mean nothing here. It costs one
//! `docker ps`, one PID listing and, when something is stale, one
//! `docker rm -fv`.
//!
//! Remaining caveats:
//!
//! - With `hidepid` on `/proc` (Linux), `ps` does not show other users' PIDs, so
//!   their live containers in the same namespace could look dead.
//! - A dead owner's PID reused by a live process keeps its container. That is
//!   the safe direction: the container is removed by a later sweep.
//!
//! `crates/persistence/tests/common/container_cleanup.rs` and
//! `crates/rest/tests/common/container_cleanup.rs` are the same file apart from
//! `OWNER_LABEL_KEY` and `PIDNS_LABEL_KEY`, and must be changed together.
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

const OWNER_LABEL_KEY: &str = "io.helios.persistence.test-owner";
const PIDNS_LABEL_KEY: &str = "io.helios.persistence.test-pidns";

static OWNER_LABEL_VALUE: LazyLock<String> = LazyLock::new(|| {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
});

static PIDNS_LABEL_VALUE: LazyLock<String> = LazyLock::new(pid_namespace_id);

/// Set once a labeled container may exist, so binaries that never start one
/// (e.g. filtered or skipped runs) do not shell out to `docker` at exit.
static CONTAINER_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Runs [`remove_containers_of_exited_owners`] once per process, before its
/// first label.
static SWEPT: Once = Once::new();

/// Labels a container request so it is force-removed when this process exits.
///
/// Adds the owner label and the PID-namespace label (see the module docs). The
/// first call in a process also sweeps containers left behind by exited test
/// processes of the same namespace.
pub fn with_cleanup_label<I: Image>(request: impl ImageExt<I>) -> ContainerRequest<I> {
    SWEPT.call_once(remove_containers_of_exited_owners);
    CONTAINER_REQUESTED.store(true, Ordering::SeqCst);
    request
        .with_label(OWNER_LABEL_KEY, OWNER_LABEL_VALUE.as_str())
        .with_label(PIDNS_LABEL_KEY, PIDNS_LABEL_VALUE.as_str())
}

/// Identifies the process-ID namespace this process's PIDs belong to.
///
/// - Windows: `windows:<COMPUTERNAME>`.
/// - Linux: `linux:<hostname>:<target of /proc/self/ns/pid>`, for example
///   `linux:box:pid:[4026531836]`. A WSL2 distro and a dev container then differ
///   from each other and from Windows.
/// - Elsewhere: `<os>:<hostname>`.
///
/// Never panics. A part that cannot be read becomes `unknown`, so the value
/// still differs per OS. The result contains no whitespace (see [`label_safe`]),
/// which the `docker ps` listing format relies on.
fn pid_namespace_id() -> String {
    let host = hostname();
    if cfg!(windows) {
        format!("windows:{host}")
    } else if cfg!(target_os = "linux") {
        let namespace = std::fs::read_link("/proc/self/ns/pid")
            .ok()
            .and_then(|link| link.to_str().map(label_safe))
            .unwrap_or_else(|| "unknown".to_owned());
        format!("linux:{host}:{namespace}")
    } else {
        format!("{}:{host}", std::env::consts::OS)
    }
}

/// This machine's (or container's) host name, or `unknown`.
fn hostname() -> String {
    let from_env = |name: &str| std::env::var(name).ok();
    let raw = if cfg!(windows) {
        from_env("COMPUTERNAME")
    } else {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
            .or_else(|| from_env("HOSTNAME"))
    }
    .or_else(|| command_stdout("hostname", &[]));
    label_safe(raw.as_deref().unwrap_or_default())
}

/// Reduces `raw` to a label fragment without whitespace or other surprising
/// characters; empty input becomes `unknown`.
fn label_safe(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '[' | ']') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "unknown".to_owned()
    } else {
        cleaned
    }
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
/// A container is removed only when its PID-namespace label equals this
/// process's AND its owner label parses as `<pid>-<nanos>` AND either that PID
/// is not running, or it is this process's own PID under a different label
/// (this process reused the PID, so the process that set the label is gone).
/// This process's own label, containers with a live owner, containers from
/// another namespace or without a namespace label, and labels that do not parse
/// are always kept (see [`dead_owner_ids`]).
///
/// Costs one `docker ps` (filtered on [`OWNER_LABEL_KEY`], so a container
/// without that label is never listed, let alone removed), one PID listing and
/// at most one `docker rm -fv`.
///
/// SAFETY NOTE: PIDs are only meaningful inside one process namespace on one
/// machine, so the sweep runs only for a local daemon outside CI (see
/// [`sweep_allowed`]) and only touches containers labelled with this process's
/// namespace. CI points `DOCKER_HOST` at one Docker host shared by runners on
/// several machines; sweeping there would remove other runs' live containers
/// mid-suite. If the gate were ever fooled, for example a remote daemon's
/// socket forwarded to a local `unix://` path with `CI` unset, the namespace
/// label would still keep out every run whose host name or namespace differs.
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
    // The only `docker ps` of the sweep: `--filter label=<owner key>` means a
    // container without our label is never listed.
    let filter = format!("label={OWNER_LABEL_KEY}");
    let format = format!(
        "{{{{.ID}}}} {{{{.Label \"{OWNER_LABEL_KEY}\"}}}} {{{{.Label \"{PIDNS_LABEL_KEY}\"}}}}"
    );
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
        PIDNS_LABEL_VALUE.as_str(),
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

/// One labelled container from the `docker ps` listing.
#[derive(Debug, PartialEq, Eq)]
struct LabelledContainer<'a> {
    id: &'a str,
    /// The `<pid>-<nanos>` owner label.
    owner: &'a str,
    /// The PID-namespace label; `None` for a container made without one.
    pidns: Option<&'a str>,
}

/// Parses the sweep's `docker ps --format` output: one line per container,
/// `<id> <owner label> <pidns label>`. Fields are separated by a single space,
/// and label values never contain whitespace (see [`label_safe`] and the
/// `<pid>-<nanos>` format), so an empty label leaves an empty field.
/// Containers without an owner label value are skipped; an empty namespace
/// label becomes `None`.
fn parse_owner_listing(listing: &str) -> Vec<LabelledContainer<'_>> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.trim_end().splitn(3, ' ');
            let id = fields.next()?.trim_start();
            let owner = fields.next()?;
            let pidns = fields.next().filter(|namespace| !namespace.is_empty());
            (!id.is_empty() && !owner.is_empty()).then_some(LabelledContainer { id, owner, pidns })
        })
        .collect()
}

/// The PID from a `<pid>-<nanos>` label value.
fn owner_pid(owner: &str) -> Option<u32> {
    owner.split_once('-')?.0.parse().ok()
}

/// Picks the container ids to remove, in input order.
///
/// An entry is selected only when its namespace label EQUALS `own_pidns` AND its
/// owner has exited. An entry with no namespace label, or another one, is never
/// selected, even when its owner PID is dead here: that PID is not from this
/// process table, so "dead" proves nothing.
///
/// Within the namespace it skips `own_owner` (this process's label) and labels
/// that are not `<pid>-<nanos>`. It selects an entry when its PID is not alive,
/// or when it is `own_pid` under a foreign label: a PID belongs to one live
/// process at a time, so that label was set by a process that has since exited
/// and whose PID this process reused. A dead owner's PID reused by some other
/// live process only keeps its container, which is the safe direction.
fn dead_owner_ids<'a>(
    entries: &[LabelledContainer<'a>],
    own_owner: &str,
    own_pid: u32,
    own_pidns: &str,
    is_alive: impl Fn(u32) -> bool,
) -> Vec<&'a str> {
    entries
        .iter()
        .filter_map(|entry| {
            if entry.pidns != Some(own_pidns) || entry.owner == own_owner {
                return None;
            }
            let pid = owner_pid(entry.owner)?;
            (pid == own_pid || !is_alive(pid)).then_some(entry.id)
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

    const NS_A: &str = "linux:vps:pid:[4026531836]";
    const NS_B: &str = "linux:vps:pid:[4026532555]";

    fn container<'a>(id: &'a str, owner: &'a str, pidns: Option<&'a str>) -> LabelledContainer<'a> {
        LabelledContainer { id, owner, pidns }
    }

    #[test]
    fn parse_owner_listing_reads_id_owner_and_namespace() {
        let listing = concat!(
            "0ca87ccb9f8f 24556-1793387658255337500 windows:DOUG\r\n",
            "d9a018e14415 37472-1 linux:box:pid:[4026531836]\n",
            "\n",
            "abc123 \n",
            "def456  windows:DOUG\n",
            "old789 99-1 \n",
            "old790 99-2\n",
        );
        assert_eq!(
            parse_owner_listing(listing),
            vec![
                container(
                    "0ca87ccb9f8f",
                    "24556-1793387658255337500",
                    Some("windows:DOUG")
                ),
                container(
                    "d9a018e14415",
                    "37472-1",
                    Some("linux:box:pid:[4026531836]")
                ),
                container("old789", "99-1", None),
                container("old790", "99-2", None),
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
            container("own", "100-5", Some(NS_A)),
            container("live", "200-1", Some(NS_A)),
            container("dead", "300-1", Some(NS_A)),
            container("reused", "100-4", Some(NS_A)),
            container("junk", "not-a-pid", Some(NS_A)),
            container("nodash", "400", Some(NS_A)),
        ];
        let stale = dead_owner_ids(&entries, "100-5", 100, NS_A, |pid| pid == 100 || pid == 200);
        assert_eq!(stale, vec!["dead", "reused"]);
    }

    #[test]
    fn dead_owner_ids_keeps_containers_whose_pid_is_alive() {
        let entries = [
            container("a", "200-1", Some(NS_A)),
            container("b", "300-2", Some(NS_A)),
            container("c", "400-3", Some(NS_A)),
        ];
        assert!(dead_owner_ids(&entries, "100-5", 100, NS_A, |_| true).is_empty());
    }

    #[test]
    fn dead_owner_ids_removes_a_dead_owner_of_the_same_namespace() {
        let entries = [container("same", "300-1", Some(NS_A))];
        assert_eq!(
            dead_owner_ids(&entries, "100-5", 100, NS_A, |_| false),
            vec!["same"]
        );
    }

    #[test]
    fn dead_owner_ids_keeps_another_namespace_even_when_the_owner_is_dead() {
        // A Windows run sees a WSL run's container (and the reverse): the PID is
        // absent from this process table, but that proves nothing.
        let entries = [
            container("wsl", "300-1", Some("linux:wsl:pid:[4026531836]")),
            container("windows", "300-2", Some("windows:DOUG")),
            // This process's own PID under a foreign namespace is no proof either.
            container("same-pid", "100-4", Some("linux:wsl:pid:[4026531836]")),
        ];
        assert_eq!(
            dead_owner_ids(&entries, "100-5", 100, "windows:DOUG", |_| false),
            vec!["windows"]
        );
        assert_eq!(
            dead_owner_ids(&entries, "100-5", 100, "linux:wsl:pid:[4026531836]", |_| {
                false
            }),
            vec!["wsl", "same-pid"]
        );
    }

    #[test]
    fn dead_owner_ids_keeps_containers_without_a_namespace_label() {
        // Made before the label existed, or by another branch's binary.
        let entries = [
            container("old-dead", "300-1", None),
            container("old-own-pid", "100-4", None),
        ];
        assert!(dead_owner_ids(&entries, "100-5", 100, NS_A, |_| false).is_empty());
    }

    #[test]
    fn dead_owner_ids_separates_dev_containers_with_overlapping_pids() {
        // A VPS where each cargo test run is its own `docker run` dev container
        // with the host's docker.sock mounted: the daemon is `unix://`, `CI` is
        // unset, and the runs' PIDs overlap. Both namespaces own a container
        // with PID 42.
        let entries = [
            container("ns-a", "42-1000", Some(NS_A)),
            container("ns-b", "42-2000", Some(NS_B)),
        ];
        // Sweeping from ns A with PID 42 gone here: only A's container goes.
        assert_eq!(
            dead_owner_ids(&entries, "7-3000", 7, NS_A, |pid| pid != 42),
            vec!["ns-a"]
        );
        // Sweeping from ns B is the mirror image.
        assert_eq!(
            dead_owner_ids(&entries, "7-3000", 7, NS_B, |pid| pid != 42),
            vec!["ns-b"]
        );
        // PID 42 live in ns A: nothing is removed, B's container included.
        assert!(dead_owner_ids(&entries, "7-3000", 7, NS_A, |pid| pid == 42).is_empty());
        // An owner-labelled container with no namespace label is kept, whatever
        // its PID is.
        let unlabelled = [
            container("no-ns-42", "42-1000", None),
            container("no-ns-dead", "9999-1", None),
        ];
        assert!(dead_owner_ids(&unlabelled, "7-3000", 7, NS_A, |_| false).is_empty());
    }

    #[test]
    fn pid_namespace_id_is_one_token_that_names_the_os() {
        let namespace = pid_namespace_id();
        assert!(
            !namespace.chars().any(char::is_whitespace),
            "namespace {namespace:?} must not contain whitespace"
        );
        let prefix = if cfg!(windows) {
            "windows:".to_owned()
        } else if cfg!(target_os = "linux") {
            "linux:".to_owned()
        } else {
            format!("{}:", std::env::consts::OS)
        };
        assert!(namespace.starts_with(&prefix), "{namespace:?}");
        assert!(namespace.len() > prefix.len(), "{namespace:?}");
        assert_eq!(namespace, PIDNS_LABEL_VALUE.as_str());
    }

    #[test]
    fn label_safe_keeps_namespace_characters_and_replaces_the_rest() {
        assert_eq!(label_safe("pid:[4026531836]\n"), "pid:[4026531836]");
        assert_eq!(label_safe(" my host\t1 "), "my_host_1");
        assert_eq!(label_safe("DESKTOP-AB.c_d"), "DESKTOP-AB.c_d");
        assert_eq!(label_safe(""), "unknown");
        assert_eq!(label_safe("  \n"), "unknown");
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

    // This file is `#[path]`-included into many test binaries, and this test runs
    // in each of them. Where the platform tool (`ps` / `tasklist`) is missing,
    // such as a minimal container, `live_pids()` is `None` and there is nothing
    // to check, so the test skips instead of failing unrelated suites.
    #[test]
    fn live_pids_lists_this_process() {
        let Some(pids) = live_pids() else {
            return;
        };
        assert!(pids.contains(&std::process::id()));
    }

    #[test]
    fn command_stdout_is_none_for_a_missing_program() {
        assert_eq!(command_stdout("hfs-no-such-program-2404", &[]), None);
    }
}
