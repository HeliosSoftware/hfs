//! `sof-cli --version` identifies the binary (#992).

use std::process::Command;

#[test]
fn version_flag_prints_version_and_exits_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_sof-cli"))
        .arg("--version")
        .output()
        .expect("run sof-cli");
    assert!(output.status.success(), "{:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim_end(),
        concat!("sof-cli ", env!("CARGO_PKG_VERSION"))
    );
}
