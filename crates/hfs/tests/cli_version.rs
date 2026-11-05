//! `hfs --version` identifies the binary (#992).
//!
//! A downloaded release artifact has no other way to say which build it is;
//! before this the flag was rejected with `unexpected argument '--version'`
//! even though the README documented it.

use std::process::Command;

fn hfs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hfs"))
}

fn assert_prints_version(flag: &str) {
    let output = hfs().arg(flag).output().expect("run hfs");
    assert!(
        output.status.success(),
        "`hfs {flag}` must exit 0, got {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    let expected_prefix = concat!("hfs ", env!("CARGO_PKG_VERSION"));
    assert!(
        stdout.trim_end().starts_with(expected_prefix),
        "`hfs {flag}` must print `{expected_prefix}[ (git <sha>)]`, got {stdout:?}"
    );
    // Whatever follows the version is the optional commit tag, nothing else.
    let rest = stdout.trim_end()[expected_prefix.len()..].trim();
    assert!(
        rest.is_empty() || (rest.starts_with("(git ") && rest.ends_with(')')),
        "unexpected trailer after the version: {rest:?}"
    );
}

#[test]
fn version_flag_prints_version_and_exits_zero() {
    assert_prints_version("--version");
}

#[test]
fn short_version_flag_prints_version_and_exits_zero() {
    assert_prints_version("-V");
}

#[test]
fn help_advertises_version_flag() {
    let output = hfs().arg("--help").output().expect("run hfs");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("Helios FHIR Server"),
        "about line: {stdout:?}"
    );
    assert!(
        stdout.contains("Usage: hfs"),
        "usage names the binary: {stdout:?}"
    );
    assert!(
        stdout.contains("-V, --version"),
        "help must list --version as the README does: {stdout:?}"
    );
}
