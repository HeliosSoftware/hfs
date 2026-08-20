//! The shared type rail's server-rendered instance counts (#541), on
//! Resources, Search, and Saved Queries. The dashboard snapshot provider is
//! process-global (see `subscriptions_http.rs`), so the unavailable state
//! must be asserted before any provider is registered, and both phases run
//! inside one test.

use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

use helios_observability::dashboard::{
    DashboardProvider, DashboardSnapshot, DashboardWindow, TypeCount, set_provider,
};

fn app() -> Router {
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
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
    )
}

async fn get(path: &str) -> String {
    let response = app()
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
    ) -> DashboardSnapshot {
        if tenant != "default" {
            return DashboardSnapshot::default();
        }
        DashboardSnapshot {
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
            ..Default::default()
        }
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
    }
}
