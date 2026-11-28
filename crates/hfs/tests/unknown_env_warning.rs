//! `hfs` warns at startup about a set `HFS_*` variable no component reads,
//! naming the one it resembles (#2707). A misspelt `HFS_AUTH_TENANT_CLAIMS`
//! used to start cleanly and leave the tenant claim at its default.
//!
//! The test starts the real binary, reads its output up to the startup line,
//! and stops it.

#![cfg(feature = "sqlite")]

use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// `hfs` output, stdout and stderr together, up to and including the first
/// line containing `marker`, or everything it wrote before exiting or before
/// 60 seconds passed.
fn startup_output(env: &[(&str, &str)], marker: &str) -> String {
    let data = std::env::temp_dir().join(format!("hfs-unknown-env-{}", std::process::id()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_hfs"))
        .envs(env.iter().copied())
        .env("HFS_SERVER_HOST", "127.0.0.1")
        .env("HFS_SERVER_PORT", free_port().to_string())
        .env("HFS_DATA_DIR", &data)
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
    let _ = std::fs::remove_dir_all(&data);
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

#[test]
fn a_misspelt_variable_is_warned_about_with_the_name_it_resembles() {
    let output = startup_output(
        &[("HFS_AUTH_TENANT_CLAIMS", "canary-2707")],
        "Starting Helios FHIR Server",
    );
    assert!(
        output.contains(
            "HFS_AUTH_TENANT_CLAIMS is set but no Helios component reads it; \
             did you mean HFS_AUTH_TENANT_CLAIM?"
        ),
        "{output}"
    );
    assert!(
        !output.contains("canary-2707"),
        "the value is not logged: {output}"
    );
}
