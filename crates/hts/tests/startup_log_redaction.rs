//! `hts` logs its database URL without the credentials in it (#2139).
//!
//! Each test starts the real binary with credentials in `HTS_DATABASE_URL`,
//! reads its output until a given log line, and stops it.
//!
//! - **`hts run`** against the shared PostgreSQL testcontainer, so both the
//!   startup line and the "backend initialized" line are written.
//! - **`hts import`** against an unreachable PostgreSQL: its start line is
//!   written before it connects.
//!
//! The first test needs Docker. Run with:
//! `cargo test -p helios-hts --features postgres --test startup_log_redaction`

#![cfg(feature = "postgres")]

mod common;

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const SECRET: &str = "redaction-canary-2139";

/// `hts` output, stdout and stderr together, up to and including the first
/// line containing `marker`, or everything it wrote before exiting or before
/// 60 seconds passed.
fn output_until(mut command: Command, marker: &str) -> String {
    let mut child = command
        .env("HTS_LOG_LEVEL", "info")
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start hts");

    let (sender, lines) = mpsc::channel();
    let readers = [
        forward_lines(child.stdout.take().expect("stdout"), sender.clone()),
        forward_lines(child.stderr.take().expect("stderr"), sender),
    ];

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut output = String::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match lines.recv_timeout(left) {
            Ok(line) => {
                output.push_str(&line);
                output.push('\n');
                if line.contains(marker) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    for reader in readers {
        let _ = reader.join();
    }
    output
}

fn forward_lines(
    stream: impl Read + Send + 'static,
    sender: mpsc::Sender<String>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    })
}

fn hts() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hts"))
}

/// The container's credentials are `postgres:postgres`; neither the pair nor
/// the URL as configured may appear, and the redacted URL must.
#[tokio::test]
async fn run_logs_the_database_url_without_credentials() {
    const STARTING: &str = "Starting Helios Terminology Server";
    const INITIALIZED: &str = "PostgreSQL terminology backend initialized";
    let url = common::pg_http_url().await;
    let (_, host_and_database) = url.split_once('@').expect("credentials in the test URL");

    let mut command = hts();
    command
        .arg("run")
        .env("HTS_STORAGE_BACKEND", "postgres")
        .env("HTS_DATABASE_URL", url)
        .env("HTS_SERVER_HOST", "127.0.0.1")
        .env("HTS_SERVER_PORT", "0");
    let output = tokio::task::spawn_blocking(move || output_until(command, INITIALIZED))
        .await
        .expect("read hts output");

    for marker in [STARTING, INITIALIZED] {
        assert!(
            output.contains(marker),
            "hts never logged `{marker}`; output:\n{output}"
        );
    }
    assert!(
        !output.contains("postgres:postgres@"),
        "hts logged the database credentials; output:\n{output}"
    );
    let redacted = format!("postgres://{host_and_database}");
    assert_eq!(
        output.matches(&format!("database_url={redacted}")).count(),
        2,
        "both lines must carry the redacted URL `{redacted}`; output:\n{output}"
    );
}

fn assert_database_log(output: &str, marker: &str, expected: &str) {
    let line = output
        .lines()
        .find(|line| line.contains(marker))
        .unwrap_or_else(|| panic!("hts never logged `{marker}`; output:\n{output}"));
    assert!(
        !output.contains(SECRET),
        "hts logged a password; output:\n{output}"
    );
    assert!(
        line.split_whitespace()
            .any(|field| field == format!("database_url={expected}")),
        "expected database_url={expected} in startup log; output:\n{output}"
    );
}

/// Exercise the pre-connection log without Docker or a reachable database.
#[test]
fn run_startup_redacts_keyword_values_and_password_delimiters() {
    const MARKER: &str = "Starting Helios Terminology Server";
    for (url, expected) in regression_inputs() {
        let mut command = hts();
        command
            .arg("run")
            .env("HTS_STORAGE_BACKEND", "postgres")
            .env("HTS_DATABASE_URL", url)
            .env("HTS_SERVER_HOST", "127.0.0.1")
            .env("HTS_SERVER_PORT", "0");
        let output = output_until(command, MARKER);
        assert_database_log(&output, MARKER, expected);
    }
}

fn regression_inputs() -> Vec<(String, &'static str)> {
    vec![
        (
            format!("postgres://hts:{SECRET}@127.0.0.1:9/hts?application_name={SECRET}"),
            "postgres://127.0.0.1:9/hts",
        ),
        (
            format!("host=127.0.0.1 port=9 user=hts password='{SECRET} with spaces' dbname=hts"),
            "postgres://127.0.0.1:9/hts",
        ),
        (
            format!("postgres://hts:{SECRET}/suffix@127.0.0.1:9/hts"),
            "postgres://127.0.0.1:9/hts",
        ),
        (
            format!("postgres://hts:{SECRET}?suffix@127.0.0.1:9/hts"),
            "postgres://127.0.0.1:9/hts",
        ),
        (
            format!("postgres://hts:{SECRET}@127.0.0.1:9/hts#private"),
            "[redacted]",
        ),
        (format!("password='{SECRET}"), "[redacted]"),
        (
            format!("postgres://hts:prefix@{SECRET}@127.0.0.1:9/hts"),
            "[redacted]",
        ),
    ]
}

#[test]
fn import_logs_the_database_url_without_credentials() {
    const MARKER: &str = "Starting bulk import";
    let input = tempfile::tempdir().expect("import directory");
    for (url, expected) in regression_inputs() {
        let mut command = hts();
        command
            .arg("import")
            .arg(input.path())
            .arg("--storage-backend")
            .arg("postgres")
            .arg("--database-url")
            .arg(url);
        let output = output_until(command, MARKER);
        assert_database_log(&output, MARKER, expected);
    }
}
