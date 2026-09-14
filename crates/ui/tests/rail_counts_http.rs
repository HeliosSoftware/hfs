//! The shared type rail's server-rendered instance counts (#541), on
//! Resources, Search, and Saved Queries. The dashboard snapshot provider is
//! process-global (see `subscriptions_http.rs`), so the unavailable state
//! must be asserted before any provider is registered, and every phase runs
//! inside one test.
//!
//! #1078: counts from an approximate snapshot render as "≈N" and say so; a
//! snapshot with no figures to give (the tenant still being seeded, or a
//! backend that cannot count) shows no count at all, never a zero (#1082).

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

use helios_observability::dashboard::{
    DashboardProvider, DashboardSnapshot, DashboardWindow, Figures, TypeCount, set_provider,
};

/// A tenant whose snapshot is counted from recent writes (approximate).
const APPROXIMATE_TENANT: &str = "rail-approximate";
/// A tenant the provider is still seeding: no figures yet.
const SEEDING_TENANT: &str = "rail-seeding";
/// A tenant whose storage backend cannot count at all.
const UNSUPPORTED_TENANT: &str = "rail-unsupported";

fn app_as(tenant: &str) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        helios_ui::NlSearch {
            enabled: true,
            configured: true,
            model: "test-model".to_string(),
        },
        None,
        None,
        tenant.to_string(),
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

async fn get(path: &str) -> String {
    get_as("default", path).await
}

async fn get_as(tenant: &str, path: &str) -> String {
    let response = app_as(tenant)
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Slices out one rail item's `<a ...>...</a>` markup so assertions about its
/// count/aria-current do not accidentally match a neighboring item.
fn rail_item<'a>(html: &'a str, resource_type: &str) -> &'a str {
    let needle = format!(r#"data-type="{resource_type}""#);
    let start = html
        .find(&needle)
        .unwrap_or_else(|| panic!("rail item for {resource_type} not found"));
    let end = html[start..]
        .find("</a>")
        .map(|i| start + i)
        .unwrap_or(html.len());
    &html[start..end]
}

struct Fixed;

#[async_trait::async_trait]
impl DashboardProvider for Fixed {
    async fn snapshot(
        &self,
        _window: DashboardWindow,
        tenant: &str,
        _types: &[String],
        _include_empty: bool,
    ) -> DashboardSnapshot {
        let read_at = chrono::Utc::now();
        let flags = |snapshot: DashboardSnapshot| match tenant {
            APPROXIMATE_TENANT => DashboardSnapshot {
                figures: Figures::Approximate {
                    read_at,
                    reconciled_at: read_at,
                },
                ..snapshot
            },
            // The provider contract: these two come with nothing measured.
            SEEDING_TENANT => DashboardSnapshot {
                figures: Figures::Pending,
                ..DashboardSnapshot::default()
            },
            UNSUPPORTED_TENANT => DashboardSnapshot {
                figures: Figures::Unsupported,
                ..DashboardSnapshot::default()
            },
            _ => snapshot,
        };
        let known = [
            "default",
            APPROXIMATE_TENANT,
            SEEDING_TENANT,
            UNSUPPORTED_TENANT,
        ];
        if !known.contains(&tenant) {
            return DashboardSnapshot::default();
        }
        flags(DashboardSnapshot {
            available: vec![
                TypeCount {
                    resource_type: "Patient".into(),
                    total: 42,
                },
                TypeCount {
                    resource_type: "Observation".into(),
                    total: 7,
                },
            ],
            figures: Figures::Exact { read_at },
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn the_rail_goes_from_no_counts_to_server_rendered_counts() {
    // Phase 1 — no provider registered: every page's rail links to
    // `?type=<name>` and marks the deep-linked entry current, but no page
    // renders a single count span — never a mix of real counts and blanks.
    for (path, base) in [
        ("/ui/resources?type=Observation", "/ui/resources"),
        ("/ui/search?type=Observation", "/ui/search"),
        ("/ui/queries?type=Observation", "/ui/queries"),
    ] {
        let html = get(path).await;
        assert!(
            html.contains(r#"id="type-rail-list""#),
            "{path}: rail present"
        );
        // The rail's chrome is unified across the four pages (#603
        // follow-up): the flat Resources look, not a bordered card.
        assert!(
            !html.contains(r#"class="card filter-rail""#),
            "{path}: no bordered card around the type rail"
        );
        let item = rail_item(&html, "Observation");
        assert!(
            item.contains(&format!(r#"href="{base}?type=Observation""#)),
            "{path}: {item}"
        );
        assert!(
            item.contains(r#"aria-current="true""#),
            "{path}: the deep-linked type is marked current: {item}"
        );
        assert!(
            item.contains(r#"title="Observation""#),
            "{path}: the full type name is accessible via title (#604): {item}"
        );
        assert!(
            !html.contains(r#"class="count""#),
            "{path}: no provider means no count span at all"
        );
    }

    // Phase 2 — provider registered: counts render for the types it knows
    // about, and an explicit "0" (never a blank) for the ones it does not.
    set_provider(Arc::new(Fixed));
    for (path, base) in [
        ("/ui/resources", "/ui/resources"),
        ("/ui/search", "/ui/search"),
        ("/ui/queries", "/ui/queries"),
    ] {
        let html = get(path).await;
        let patient = rail_item(&html, "Patient");
        assert!(
            patient.contains(&format!(r#"href="{base}?type=Patient""#)) && patient.contains(">42<"),
            "{path}: {patient}"
        );
        let observation = rail_item(&html, "Observation");
        assert!(observation.contains(">7<"), "{path}: {observation}");
        let encounter = rail_item(&html, "Encounter");
        assert!(
            encounter.contains(">0<"),
            "{path}: a type with no stored resources still renders 0: {encounter}"
        );
        let long_name = "MedicinalProductUndesirableEffect";
        let long_item = rail_item(&html, long_name);
        assert!(
            long_item.contains(&format!(r#"data-full-name="{long_name}""#)),
            "{path}: the full name feeds the shared hover/focus tooltip: {long_item}"
        );
        assert!(
            long_item.contains(&format!(r#"title="{long_name}""#)),
            "{path}: the no-JS tooltip fallback remains available: {long_item}"
        );
        assert!(
            long_item.contains(&format!(r#">{long_name}</span>"#))
                && long_item.contains(r#"<span class="count">0</span>"#),
            "{path}: the accessible name and count remain separate: {long_item}"
        );
        assert!(
            !patient.contains("≈") && !patient.contains("count--approximate"),
            "{path}: exact counts carry no approximation mark: {patient}"
        );
    }

    // Phase 3 (#1078) — an approximate snapshot: every count is still shown,
    // prefixed "≈", with the reason as its title and as screen-reader text.
    for path in ["/ui/resources", "/ui/search", "/ui/queries"] {
        let html = get_as(APPROXIMATE_TENANT, path).await;
        let patient = rail_item(&html, "Patient");
        assert!(patient.contains(">≈42<"), "{path}: {patient}");
        assert!(
            patient.contains(
                r#"title="Approximate: counted from recent writes and still being reconciled with storage.""#
            ),
            "{path}: the count says why it is approximate: {patient}"
        );
        assert!(
            patient.contains(
                r#"<span class="visually-hidden"> Approximate: counted from recent writes"#
            ),
            "{path}: and says so to assistive technology: {patient}"
        );
        let encounter = rail_item(&html, "Encounter");
        assert!(encounter.contains(">≈0<"), "{path}: {encounter}");
        assert!(
            !html.contains(r#"<span class="count">"#),
            "{path}: no count on an approximate page reads as exact"
        );
    }

    // Phase 4 — snapshots with no counts to give (#1082): a tenant still being
    // seeded, and a backend that cannot count. Neither renders a count span,
    // so neither shows a zero.
    for tenant in [SEEDING_TENANT, UNSUPPORTED_TENANT] {
        for path in ["/ui/resources", "/ui/search", "/ui/queries"] {
            let html = get_as(tenant, path).await;
            assert!(
                html.contains(r#"id="type-rail-list""#),
                "{tenant} {path}: rail present"
            );
            let patient = rail_item(&html, "Patient");
            assert!(
                !html.contains(r#"class="count"#),
                "{tenant} {path}: no count span at all: {patient}"
            );
        }
    }
}
