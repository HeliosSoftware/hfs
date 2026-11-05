//! The bundled Keycloak realm lets a user signed in through `hfs-web` import.
//!
//! Since #1488 the UI's Import page submits `$bulk-submit` as the signed-in
//! user, so that user's token must carry `system/bulk-submit`. The realm in
//! `docker/keycloak/realm.json` lacked it until #1634 (issue #1633), and the
//! documented local setup (`HFS_UI_LOGIN_CLIENT_ID=hfs-web`) could not import.
//!
//! This test pins the fix (#1668) without running Keycloak. Two things must
//! hold, and each is needed on its own:
//!
//! - `hfs-web`'s default client scopes grant the `bulk-submit` operation,
//!   checked with the same `ScopeSet::grants_operation` that `$bulk-submit`
//!   enforces, so the test follows what the server requires.
//! - The realm defines a `system/bulk-submit` client scope with
//!   `include.in.token.scope` set to `"true"`. Keycloak silently ignores a
//!   default scope that does not exist, and without that attribute the scope
//!   never reaches the token's `scope` claim.
//!
//! The mutation tests below edit copies of the realm to prove the check
//! fails when either half is broken.

use helios_auth::ScopeSet;
use serde_json::Value;

const REALM_JSON: &str = include_str!("../../../docker/keycloak/realm.json");
const WEB_CLIENT_ID: &str = "hfs-web";
const SUBMIT_OPERATION: &str = "bulk-submit";
const SUBMIT_SCOPE: &str = "system/bulk-submit";

fn bundled_realm() -> Value {
    serde_json::from_str(REALM_JSON).expect("docker/keycloak/realm.json is valid JSON")
}

/// Checks that `realm` lets a user signed in through `hfs-web` call `$bulk-submit`.
fn check_realm(realm: &Value) -> Result<(), String> {
    let client = realm["clients"]
        .as_array()
        .ok_or("realm has no `clients` array")?
        .iter()
        .find(|c| c["clientId"] == WEB_CLIENT_ID)
        .ok_or_else(|| format!("realm has no `{WEB_CLIENT_ID}` client"))?;

    let default_scopes: Vec<String> = client["defaultClientScopes"]
        .as_array()
        .ok_or_else(|| format!("`{WEB_CLIENT_ID}` has no `defaultClientScopes` array"))?
        .iter()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect();
    if !ScopeSet::parse_array(&default_scopes).grants_operation(SUBMIT_OPERATION) {
        return Err(format!(
            "`{WEB_CLIENT_ID}` default scopes {default_scopes:?} do not grant `{SUBMIT_OPERATION}`"
        ));
    }

    let scope = realm["clientScopes"]
        .as_array()
        .ok_or("realm has no `clientScopes` array")?
        .iter()
        .find(|s| s["name"] == SUBMIT_SCOPE)
        .ok_or_else(|| format!("realm defines no `{SUBMIT_SCOPE}` client scope"))?;
    let in_token = &scope["attributes"]["include.in.token.scope"];
    if *in_token != "true" {
        return Err(format!(
            "`{SUBMIT_SCOPE}` has `include.in.token.scope` = {in_token}, expected \"true\""
        ));
    }

    Ok(())
}

fn web_client_mut(realm: &mut Value) -> &mut Value {
    realm["clients"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["clientId"] == WEB_CLIENT_ID)
        .unwrap()
}

fn submit_scope_mut(realm: &mut Value) -> &mut Value {
    realm["clientScopes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|s| s["name"] == SUBMIT_SCOPE)
        .unwrap()
}

#[test]
fn bundled_realm_lets_hfs_web_import() {
    check_realm(&bundled_realm()).unwrap();
}

#[test]
fn fails_without_the_scope_on_hfs_web() {
    let mut realm = bundled_realm();
    web_client_mut(&mut realm)["defaultClientScopes"]
        .as_array_mut()
        .unwrap()
        .retain(|s| *s != SUBMIT_SCOPE);
    let err = check_realm(&realm).unwrap_err();
    assert!(err.contains("do not grant"), "{err}");
}

#[test]
fn fails_without_the_client_scope() {
    let mut realm = bundled_realm();
    realm["clientScopes"]
        .as_array_mut()
        .unwrap()
        .retain(|s| s["name"] != SUBMIT_SCOPE);
    let err = check_realm(&realm).unwrap_err();
    assert!(err.contains("defines no"), "{err}");
}

#[test]
fn fails_when_the_scope_is_kept_out_of_the_token() {
    let mut realm = bundled_realm();
    submit_scope_mut(&mut realm)["attributes"]["include.in.token.scope"] = "false".into();
    let err = check_realm(&realm).unwrap_err();
    assert!(err.contains("include.in.token.scope"), "{err}");
}

#[test]
fn fails_without_the_include_in_token_attribute() {
    let mut realm = bundled_realm();
    submit_scope_mut(&mut realm)["attributes"]
        .as_object_mut()
        .unwrap()
        .remove("include.in.token.scope");
    let err = check_realm(&realm).unwrap_err();
    assert!(err.contains("include.in.token.scope"), "{err}");
}
