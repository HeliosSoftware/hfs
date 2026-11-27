//! A set `HFS_*` setting that is not a value of its type fails startup with
//! its name and raw value (#2728), rather than leaving its default in force.
//! A limit set as `64MiB` used to read as the built-in limit, silently.
//!
//! The only test in its binary: it sets process environment variables, which
//! would race any other test reading the environment.

use helios_rest::ServerConfig;

#[test]
fn an_unparsable_setting_fails_startup_naming_it_and_its_value() {
    unsafe {
        std::env::set_var("HFS_BULK_SUBMIT_MAX_MANIFEST_BYTES", "64MiB");
        std::env::set_var("HFS_TENANT_STRICT_VALIDATION", "yes");
    }

    let error = ServerConfig::try_parse_from_with_provenance(["hfs"])
        .expect_err("startup refused")
        .to_string();

    assert!(
        error.contains(
            r#"HFS_BULK_SUBMIT_MAX_MANIFEST_BYTES="64MiB" is not a non-negative integer"#
        ),
        "{error}"
    );
    assert!(
        error.contains(r#"HFS_TENANT_STRICT_VALIDATION="yes" is not a boolean"#),
        "{error}"
    );
}
