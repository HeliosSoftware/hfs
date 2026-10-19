//! `hfs` logs its database connection string at startup without the
//! credentials in it (#2139).
//!
//! Each test starts the real binary with a password in the connection string,
//! reads its output until the startup line that names the backend, and stops
//! it. The database is never reachable: the line is written before `hfs`
//! connects. Run with:
//! `cargo test -p helios-hfs --features mongodb,postgres --test startup_log_redaction`

#![cfg(any(feature = "mongodb", feature = "postgres"))]

use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// The password every connection string carries; it must never be logged.
const SECRET: &str = "redaction-canary-2139";

/// `hfs` output, stdout and stderr together, up to and including the first
/// line containing `marker`, or everything it wrote before exiting or before
/// 60 seconds passed.
fn startup_output(env: &[(&str, &str)], marker: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hfs"))
        .envs(env.iter().copied())
        .env("HFS_SERVER_HOST", "127.0.0.1")
        .env("HFS_SERVER_PORT", free_port().to_string())
        .env("HFS_LOG_LEVEL", "info")
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start hfs");

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

/// A port nothing is listening on; `hfs` rejects port 0.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("reserve a local port")
        .port()
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

fn assert_logged_redacted(output: &str, marker: &str, redacted: &str) {
    assert!(
        output.contains(marker),
        "hfs never logged `{marker}`; output:\n{output}"
    );
    assert!(
        !output.contains(SECRET),
        "hfs logged the connection string's password; output:\n{output}"
    );
    assert!(
        output.lines().any(|line| line.contains(marker)
            && line
                .split_whitespace()
                .any(|field| field == format!("url={redacted}"))),
        "expected the redacted connection string `{redacted}`; output:\n{output}"
    );
}

/// A multi-host replica-set URI with credentials and options, the shape a
/// deployment uses.
#[cfg(feature = "mongodb")]
#[test]
fn mongodb_startup_logs_the_connection_string_without_credentials() {
    const MARKER: &str = "Initializing MongoDB backend";
    let url = format!(
        "mongodb://helios:{SECRET}@127.0.0.1:9,127.0.0.1:10/helios?authSource=admin&replicaSet=rs0"
    );
    let output = startup_output(
        &[
            ("HFS_STORAGE_BACKEND", "mongodb"),
            ("HFS_DATABASE_URL", &url),
            ("HFS_MONGODB_URL", &url),
            ("HFS_MONGODB_DATABASE", "helios"),
            ("HFS_MONGODB_CONNECT_TIMEOUT_MS", "500"),
        ],
        MARKER,
    );
    assert_logged_redacted(&output, MARKER, "mongodb://127.0.0.1:9,127.0.0.1:10/helios");
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_startup_logs_the_connection_string_without_credentials() {
    const MARKER: &str = "Initializing PostgreSQL backend from connection string";
    let url = format!("postgresql://hfs:{SECRET}@127.0.0.1:9/hfs?sslmode=disable");
    let output = startup_output(
        &[
            ("HFS_STORAGE_BACKEND", "postgres"),
            ("HFS_DATABASE_URL", &url),
        ],
        MARKER,
    );
    assert_logged_redacted(&output, MARKER, "postgresql://127.0.0.1:9/hfs");
}

#[cfg(feature = "mongodb")]
#[test]
fn malformed_mongodb_startup_url_is_redacted_wholesale() {
    const MARKER: &str = "Initializing MongoDB backend";
    for url in [
        format!("helios:{SECRET}@127.0.0.1:9/helios"),
        format!("mongodb://helios:{SECRET}/suffix@127.0.0.1:9/helios"),
    ] {
        let output = startup_output(
            &[
                ("HFS_STORAGE_BACKEND", "mongodb"),
                ("HFS_DATABASE_URL", &url),
                ("HFS_MONGODB_URL", &url),
                ("HFS_MONGODB_DATABASE", "helios"),
            ],
            MARKER,
        );
        assert_logged_redacted(&output, MARKER, "[redacted]");
    }
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_startup_omits_password_delimiters() {
    const MARKER: &str = "Initializing PostgreSQL backend from connection string";
    for separator in ["/", "?"] {
        let url = format!("postgres://hfs:{SECRET}{separator}suffix@127.0.0.1:9/hfs");
        let output = startup_output(
            &[
                ("HFS_STORAGE_BACKEND", "postgres"),
                ("HFS_DATABASE_URL", &url),
            ],
            MARKER,
        );
        assert_logged_redacted(&output, MARKER, "postgres://127.0.0.1:9/hfs");
    }
}

#[cfg(feature = "postgres")]
#[test]
fn ambiguous_postgres_userinfo_is_redacted_wholesale() {
    const MARKER: &str = "Initializing PostgreSQL backend from connection string";
    let url = format!("postgres://hfs:prefix@{SECRET}@127.0.0.1:9/hfs");
    let output = startup_output(
        &[
            ("HFS_STORAGE_BACKEND", "postgres"),
            ("HFS_DATABASE_URL", &url),
        ],
        MARKER,
    );
    assert_logged_redacted(&output, MARKER, "[redacted]");
}
