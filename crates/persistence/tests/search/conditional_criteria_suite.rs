//! Backend-agnostic conditional-criteria suite (issue #1312).
//!
//! `If-None-Exist`, conditional update / delete, and a transaction entry's
//! `ifNoneExist` hand a backend a raw `name=value` string. Each backend used
//! to type that value for itself with `SearchValue::parse`, which strips any
//! leading two letters spelling a comparator (`eq ne gt lt ge le sa eb ap`)
//! whatever the parameter's type — so `family=Neal` searched for `al` and
//! `identifier=ne123` for `123`.
//!
//! What a backend then *did* with the stray comparator differed, which is why
//! this is driven through each one's real `ConditionalStorage` path rather
//! than asserted once on the shared builder:
//!
//! * SQLite and PostgreSQL ignore a comparator on string / token rows and
//!   search for the remainder. The named resource is missed (duplicate
//!   create) **and** an unrelated one carrying the remainder is hit — `Allen`
//!   for `al`, the identifier `123` — so a conditional update or delete lands
//!   on the wrong resource.
//! * MongoDB refuses a non-`eq` comparator on those types, so most criteria
//!   failed outright instead; `eq…` values still mis-hit.
//!
//! Every scenario seeds the decoy the old parser would have found, and the
//! suite opens with positive controls: a backend built without the spec
//! search parameters knows neither `family` nor `identifier`, never matches
//! anything, and would satisfy every "did not touch the decoy" assertion
//! vacuously.
//!
//! Included by `#[path]` into each backend's test binary, the same
//! arrangement as `date_boundary_suite.rs`.

#![allow(dead_code)]

use serde_json::{Value, json};

use helios_fhir::FhirVersion;
use helios_persistence::core::{
    BundleEntry, BundleMethod, BundleProvider, ConditionalCreateResult, ConditionalDeleteResult,
    ConditionalStorage, ConditionalUpdateResult, ResourceStorage,
};
use helios_persistence::error::TransactionError;
use helios_persistence::tenant::{TenantContext, TenantId, TenantPermissions};

fn tenant(base: &str, label: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(format!("{base}-{label}")),
        TenantPermissions::full_access(),
    )
}

fn patient(id: Option<&str>, family: &str, identifier: &str) -> Value {
    let mut resource = json!({
        "resourceType": "Patient",
        "name": [{"family": family}],
        "identifier": [{"system": "http://example.org/mrn", "value": identifier}]
    });
    if let Some(id) = id {
        resource["id"] = json!(id);
    }
    resource
}

async fn seed<S: ResourceStorage>(
    backend: &S,
    tenant: &TenantContext,
    rows: &[(&str, &str, &str)],
) {
    for (id, family, identifier) in rows {
        backend
            .create(
                tenant,
                "Patient",
                patient(Some(id), family, identifier),
                FhirVersion::default(),
            )
            .await
            .expect("seed patient");
    }
}

const TARGET: (&str, &str, &str) = ("target", "Neal", "ne123");
/// What the old parser searched for: `Allen` starts with `al`, and `123` is
/// `ne123` minus `ne`.
const DECOY: (&str, &str, &str) = ("decoy", "Allen", "123");
const BYSTANDER: (&str, &str, &str) = ("bystander", "Wilson", "zz9");

/// The family name a patient carries afterwards, or how it went missing — a
/// deleted resource reads back as a `Gone` error rather than `None`.
async fn family_of<S: ResourceStorage>(backend: &S, tenant: &TenantContext, id: &str) -> String {
    match backend.read(tenant, "Patient", id).await {
        Ok(Some(stored)) => stored.content()["name"][0]["family"]
            .as_str()
            .unwrap_or("?")
            .to_string(),
        Ok(None) => "<absent>".to_string(),
        Err(e) => format!("<{e}>"),
    }
}

/// Collects one line per scenario, so a failing backend reports the whole
/// table rather than its first row.
#[derive(Default)]
struct Report {
    lines: Vec<String>,
    failures: usize,
}

impl Report {
    fn record(&mut self, scenario: &str, expected: &str, actual: String) {
        let ok = expected == actual;
        if !ok {
            self.failures += 1;
        }
        self.lines.push(format!(
            "{} | {scenario} | expected {expected} | actual {actual}",
            if ok { "ok  " } else { "FAIL" }
        ));
    }
}

/// Drives the conditional interactions with criteria whose values begin with
/// comparator letters. `run_transactions` adds the in-transaction
/// `ifNoneExist` resolver, a separate code path on every backend; a backend
/// whose topology cannot run transactions reports that and the rows are
/// skipped.
pub async fn prefix_like_criteria_name_the_right_resource<S>(
    backend: &S,
    tenant_base: &str,
    run_transactions: bool,
) where
    S: ResourceStorage + ConditionalStorage + BundleProvider,
{
    let mut report = Report::default();
    let version = FhirVersion::default();

    // ---- conditional create -------------------------------------------------
    for (label, criteria, rows, expected) in [
        // Positive controls: the registry knows these parameters.
        (
            "cc-control-family",
            "family=Wilson",
            &[BYSTANDER][..],
            "Exists(bystander)",
        ),
        (
            "cc-control-system",
            "identifier=http://example.org/mrn|ne123",
            &[TARGET, DECOY][..],
            "Exists(target)",
        ),
        ("cc-family", "family=Neal", &[TARGET][..], "Exists(target)"),
        (
            "cc-token",
            "identifier=ne123",
            &[TARGET][..],
            "Exists(target)",
        ),
        (
            "cc-token-eq",
            "identifier=eq77",
            &[("target", "Wilson", "eq77")][..],
            "Exists(target)",
        ),
        ("cc-family-decoy", "family=Neal", &[DECOY][..], "Created"),
        (
            "cc-token-decoy",
            "identifier=ne123",
            &[DECOY][..],
            "Created",
        ),
        (
            "cc-or-list",
            "identifier=ne123,zz9",
            &[TARGET, BYSTANDER][..],
            "MultipleMatches(2)",
        ),
        (
            "cc-modifier",
            "family:exact=Neal",
            &[TARGET, DECOY][..],
            "Exists(target)",
        ),
    ] {
        let tenant = tenant(tenant_base, label);
        seed(backend, &tenant, rows).await;
        let actual = match backend
            .conditional_create(
                &tenant,
                "Patient",
                patient(None, "Incoming", "incoming-1"),
                criteria,
                version,
            )
            .await
        {
            Ok(ConditionalCreateResult::Created(_)) => "Created".to_string(),
            Ok(ConditionalCreateResult::Exists(found)) => format!("Exists({})", found.id()),
            Ok(ConditionalCreateResult::MultipleMatches(n)) => format!("MultipleMatches({n})"),
            Err(e) => format!("Err({e})"),
        };
        report.record(
            &format!("conditional_create {criteria} [{label}]"),
            expected,
            actual,
        );
    }

    // ---- conditional update -------------------------------------------------
    for (label, criteria, rows, expected) in [
        (
            "cu-control",
            "family=Wilson",
            &[TARGET, DECOY, BYSTANDER][..],
            "Updated(bystander)",
        ),
        (
            "cu-family",
            "family=Neal",
            &[TARGET, DECOY, BYSTANDER][..],
            "Updated(target)",
        ),
        (
            "cu-token",
            "identifier=ne123",
            &[TARGET, DECOY, BYSTANDER][..],
            "Updated(target)",
        ),
        ("cu-family-decoy", "family=Neal", &[DECOY][..], "Created"),
        (
            "cu-token-decoy",
            "identifier=ne123",
            &[DECOY][..],
            "Created",
        ),
    ] {
        let tenant = tenant(tenant_base, label);
        seed(backend, &tenant, rows).await;
        let outcome = match backend
            .conditional_update(
                &tenant,
                "Patient",
                patient(None, "Rewritten", "ne123"),
                criteria,
                true,
                version,
            )
            .await
        {
            Ok(ConditionalUpdateResult::Updated(stored)) => format!("Updated({})", stored.id()),
            Ok(ConditionalUpdateResult::Created(_)) => "Created".to_string(),
            Ok(ConditionalUpdateResult::NoMatch) => "NoMatch".to_string(),
            Ok(ConditionalUpdateResult::MultipleMatches(n)) => format!("MultipleMatches({n})"),
            Err(e) => format!("Err({e})"),
        };
        report.record(
            &format!("conditional_update {criteria} [{label}]"),
            expected,
            outcome,
        );

        // Whatever the call answered, the decoy must still be the decoy.
        if rows.contains(&DECOY) {
            let decoy = family_of(backend, &tenant, "decoy").await;
            report.record(
                &format!("conditional_update {criteria} [{label}] decoy afterwards"),
                "Allen",
                decoy,
            );
        }
    }

    // ---- conditional delete -------------------------------------------------
    for (label, criteria, rows, expected) in [
        (
            "cd-control",
            "family=Wilson",
            &[TARGET, DECOY, BYSTANDER][..],
            "Deleted(bystander)",
        ),
        (
            "cd-family",
            "family=Neal",
            &[TARGET, DECOY, BYSTANDER][..],
            "Deleted(target)",
        ),
        (
            "cd-token",
            "identifier=ne123",
            &[TARGET, DECOY, BYSTANDER][..],
            "Deleted(target)",
        ),
        ("cd-family-decoy", "family=Neal", &[DECOY][..], "NoMatch"),
        (
            "cd-token-decoy",
            "identifier=ne123",
            &[DECOY][..],
            "NoMatch",
        ),
        (
            "cd-token-eq-decoy",
            "identifier=eq77",
            &[("decoy", "Allen", "77")][..],
            "NoMatch",
        ),
    ] {
        let tenant = tenant(tenant_base, label);
        seed(backend, &tenant, rows).await;
        let outcome = match backend
            .conditional_delete(&tenant, "Patient", criteria)
            .await
        {
            Ok(ConditionalDeleteResult::Deleted(stored)) => format!("Deleted({})", stored.id()),
            Ok(ConditionalDeleteResult::NoMatch) => "NoMatch".to_string(),
            Ok(ConditionalDeleteResult::MultipleMatches(n)) => format!("MultipleMatches({n})"),
            Err(e) => format!("Err({e})"),
        };
        report.record(
            &format!("conditional_delete {criteria} [{label}]"),
            expected,
            outcome,
        );

        if rows.iter().any(|(id, _, _)| *id == "decoy") {
            let decoy = family_of(backend, &tenant, "decoy").await;
            report.record(
                &format!("conditional_delete {criteria} [{label}] decoy afterwards"),
                "Allen",
                decoy,
            );
        }
    }

    // ---- transaction ifNoneExist -------------------------------------------
    if run_transactions {
        for (label, criteria, rows, expected) in [
            (
                "tx-control",
                "family=Wilson",
                &[BYSTANDER][..],
                "200 Patient/bystander",
            ),
            (
                "tx-family",
                "family=Neal",
                &[TARGET][..],
                "200 Patient/target",
            ),
            (
                "tx-token",
                "identifier=ne123",
                &[TARGET][..],
                "200 Patient/target",
            ),
            ("tx-family-decoy", "family=Neal", &[DECOY][..], "201"),
            ("tx-token-decoy", "identifier=ne123", &[DECOY][..], "201"),
        ] {
            let tenant = tenant(tenant_base, label);
            seed(backend, &tenant, rows).await;
            let entry = BundleEntry {
                method: BundleMethod::Post,
                url: "Patient".to_string(),
                resource: Some(patient(None, "Incoming", "incoming-1")),
                if_match: None,
                if_none_match: None,
                if_none_exist: Some(criteria.to_string()),
                full_url: Some("urn:uuid:incoming".to_string()),
            };
            let outcome = match backend
                .process_transaction(&tenant, vec![entry], version)
                .await
            {
                Ok(result) => {
                    let entry = &result.entries[0];
                    match entry.status {
                        200 => format!("200 {}", entry.location.clone().unwrap_or_default()),
                        status => status.to_string(),
                    }
                }
                Err(TransactionError::UnsupportedIsolationLevel { .. }) => {
                    eprintln!("transactions unsupported by this topology; skipping {label}");
                    continue;
                }
                Err(e) => format!("Err({e})"),
            };
            let outcome = outcome
                .split("/_history")
                .next()
                .unwrap_or_default()
                .to_string();
            report.record(
                &format!("transaction ifNoneExist {criteria} [{label}]"),
                expected,
                outcome,
            );
        }
    }

    println!(
        "\n#1312 conditional criteria on {}:",
        backend.backend_name()
    );
    for line in &report.lines {
        println!("  {line}");
    }
    assert_eq!(
        report.failures,
        0,
        "{} conditional-criteria scenario(s) misbehaved on {}:\n{}",
        report.failures,
        backend.backend_name(),
        report.lines.join("\n")
    );
}
