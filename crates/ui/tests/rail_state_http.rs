//! Integration coverage for the rail-state contract at the level only a real
//! mounted router can prove, using the shared [`support::InMemorySettingsStore`]
//! double:
//!
//! - that `resolve_prefs` reads the settings store **exactly once** per
//!   request;
//! - that Resources, Search, Saved Queries, and Search Parameters actually
//!   resolve and persist through `rail_state` — an explicit selection that
//!   wins and is recorded, a stored `last` that restores and falls back
//!   silently when stale, and the "Recently used" group staying resolved
//!   against the live rail — the behaviors a page struct's fields alone
//!   (covered in `router_http.rs`) cannot exercise, since they need a real
//!   store round trip;
//! - the same, for the three SQL rails (View Definitions, SQL Queries, SQL
//!   Views) — including their `{id, name, meta}` snapshot, pruning a stale
//!   explicit selection, and the "Recently used" group staying immune to
//!   the rail's own `?filter=`/pagination.
//! - Compartments' `def` restore — no "Recently used" group here (only 4-5
//!   definitions), so there is nothing to render, only
//!   `rails.compartments` to resolve and record. The "no settings store"
//!   case is already covered by `router_http.rs`'s
//!   `compartments_page_defaults_to_patient`, which needs no real store.
//!
//! The pure logic (`RailPage`, `RailEntry`, `RailState`, `select`/
//! `select_all`/`prune`, `resolve_recents`) and the `RequestSettings`
//! extraction itself live behind `pub(crate)` in `helios_ui::rail_state`, so
//! they are unit-tested inside that module (`cargo test -p helios-ui --lib`)
//! instead — an integration test here is a separate crate and cannot reach
//! `pub(crate)` items.

mod support;

use axum::{Router, body::Body, http::Request, response::Response};
use helios_persistence::core::SettingsStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

use support::InMemorySettingsStore;

fn nl() -> helios_ui::NlSearch {
    helios_ui::NlSearch {
        enabled: true,
        configured: true,
        model: "test-model".to_string(),
    }
}

fn app_with(settings: Arc<InMemorySettingsStore>) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(),
        None,
        Some(settings),
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::from_data_dir(
            std::path::Path::new("../../data"),
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

async fn get(app: Router, path: &str) {
    let response = app
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(
        response.status().is_success() || response.status().is_redirection(),
        "unexpected status {} for {path}",
        response.status()
    );
}

/// "Tests esperados" #6: a page load reads the settings store exactly once,
/// the same cost model `main` already had — rail state rides the one read
/// `resolve_prefs` already performs, adding none.
#[tokio::test]
async fn a_page_load_reads_settings_exactly_once() {
    let store = Arc::new(InMemorySettingsStore::new());
    get(app_with(store.clone()), "/ui/resources").await;
    assert_eq!(store.get_settings_calls(), 1);
}

/// Two separate page loads each pay exactly one read — the cost is per
/// request, not amortized or doubled across pages.
#[tokio::test]
async fn each_of_several_page_loads_reads_settings_exactly_once() {
    let store = Arc::new(InMemorySettingsStore::new());
    get(app_with(store.clone()), "/ui/resources").await;
    get(app_with(store.clone()), "/ui/search-parameters").await;
    get(app_with(store.clone()), "/ui/compartments").await;
    assert_eq!(store.get_settings_calls(), 3);
}

// ── Ticket 02: Resources, Search, Saved Queries, Search Parameters ────────

async fn body_text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    // Normalized to LF, matching `router_http.rs`'s own helper: what line
    // endings the response carries depends on the build checkout (#671),
    // which these assertions must not depend on.
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace("\r\n", "\n")
}

/// Asserts `200 OK` and returns the body — every test below drives a full
/// page GET, never a redirect or an error.
async fn get_ok_html(app: Router, path: &str) -> String {
    let response = app
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "unexpected status {} for {path}",
        response.status()
    );
    body_text(response).await
}

/// Seeds `rails.<page>` at the normalized location — `byTenant.default`,
/// exactly what `rail_state::persist` itself writes for the default tenant
/// every test app here runs under — so a restore test needs no write-path
/// normalization of its own to read the seed back.
async fn seed_rail(store: &InMemorySettingsStore, page: &str, value: Value) {
    let mut rails = serde_json::Map::new();
    rails.insert(page.to_string(), value);
    let mut tenant = serde_json::Map::new();
    tenant.insert("rails".to_string(), Value::Object(rails));
    let mut by_tenant = serde_json::Map::new();
    by_tenant.insert("default".to_string(), Value::Object(tenant));
    let mut patch = serde_json::Map::new();
    patch.insert("byTenant".to_string(), Value::Object(by_tenant));
    store
        .patch_settings("l2:", Value::Object(patch), None)
        .await
        .unwrap();
}

/// The `rails.<page>` slice of a document `store.peek` returned — `Value::Null`
/// (never a panic) when the tenant, `rails`, or `page` is absent, so a test can
/// index straight through it.
fn stored_rail<'a>(doc: &'a Value, page: &str) -> &'a Value {
    &doc["byTenant"]["default"]["rails"][page]
}

/// The `id="type-rail-recent"` (or `id="sp-rail-recent"`) group's own HTML —
/// its opening tag plus every entry, up to the `filter-rail__divider` that
/// always follows it — so a test can assert what is (and is not) painted
/// inside the group without also matching the scrollable list below it.
fn recent_group_html<'a>(html: &'a str, group_id: &str) -> &'a str {
    let start = html
        .find(&format!(r#"id="{group_id}""#))
        .unwrap_or_else(|| panic!("{group_id} present"));
    let end = html[start..]
        .find("filter-rail__divider")
        .map(|i| i + start)
        .unwrap_or_else(|| panic!("{group_id} divider present"));
    &html[start..end]
}

/// "Tests esperados" #1: a stored `last` is restored when the request itself
/// carries no explicit `?type=` — marking the rail current and naming the
/// type in "Create new".
#[tokio::test]
async fn resources_restores_the_stored_last_selection() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "resources",
        json!({"last": "Observation", "recent": [{"id": "Observation"}]}),
    )
    .await;

    let html = get_ok_html(app_with(store), "/ui/resources").await;
    assert!(html.contains(r#"data-selected-type="Observation""#));
    assert!(html.contains("Create new Observation"));
}

/// "Tests esperados" #2: an explicit `?type=` always wins over a stored
/// selection, and — because it resolves — is recorded as the new `last` and
/// `recent[0]`.
#[tokio::test]
async fn resources_explicit_type_wins_over_stored_and_is_recorded() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "resources",
        json!({"last": "Observation", "recent": [{"id": "Observation"}]}),
    )
    .await;

    let html = get_ok_html(app_with(store.clone()), "/ui/resources?type=Encounter").await;
    assert!(html.contains(r#"data-selected-type="Encounter""#));

    let doc = store.peek("l2:").expect("settings stored");
    let rail = stored_rail(&doc, "resources");
    assert_eq!(rail["last"], "Encounter");
    assert_eq!(rail["recent"][0]["id"], "Encounter");
}

/// "Tests esperados" #3: a stored `last` that no longer names a real type
/// falls back to the page default in silence — no error, and the stale value
/// is left untouched rather than overwritten by the fallback.
#[tokio::test]
async fn resources_a_stale_stored_last_falls_back_silently_and_writes_nothing() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "resources",
        json!({"last": "NoLongerAType", "recent": [{"id": "NoLongerAType"}]}),
    )
    .await;

    let html = get_ok_html(app_with(store.clone()), "/ui/resources").await;
    assert!(html.contains(r#"data-selected-type="Patient""#));

    let doc = store.peek("l2:").expect("settings stored");
    assert_eq!(
        stored_rail(&doc, "resources")["last"],
        "NoLongerAType",
        "a silent fallback must not overwrite the stale value"
    );
}

/// "Tests esperados" #5: repeating the same explicit, already-current
/// selection is a no-op write — `RailState::select` reports no change, so
/// the stored document is untouched (not merely re-written identically).
#[tokio::test]
async fn resources_repeating_the_current_selection_writes_nothing() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "resources",
        json!({"last": "Encounter", "recent": [{"id": "Encounter"}]}),
    )
    .await;
    let before = store.peek("l2:").expect("settings stored");

    get_ok_html(app_with(store.clone()), "/ui/resources?type=Encounter").await;

    let after = store.peek("l2:").expect("settings stored");
    assert_eq!(before, after);
}

/// "Tests esperados" #6: the "Recently used" group renders present-but-hidden
/// with nothing stored, and — once something is — its entries render in MRU
/// order (a plain rail item per entry, the same shape the scrollable list
/// uses).
#[tokio::test]
async fn resources_recent_group_is_hidden_when_empty_and_ordered_when_not() {
    let store = Arc::new(InMemorySettingsStore::new());

    let html = get_ok_html(app_with(store.clone()), "/ui/resources").await;
    let group = recent_group_html(&html, "type-rail-recent");
    assert!(
        group[..group.find('>').unwrap()].contains("hidden"),
        "empty group stays hidden"
    );

    seed_rail(
        &store,
        "resources",
        json!({"last": "Observation", "recent": [{"id": "Observation"}, {"id": "Patient"}]}),
    )
    .await;
    let html = get_ok_html(app_with(store.clone()), "/ui/resources").await;
    let group = recent_group_html(&html, "type-rail-recent");
    assert!(
        !group[..group.find('>').unwrap()].contains("hidden"),
        "a non-empty group is shown"
    );
    assert_eq!(group.matches(r#"class="filter-rail__item""#).count(), 2);
    assert!(
        group.find(r#"data-type="Observation""#).unwrap()
            < group.find(r#"data-type="Patient""#).unwrap(),
        "recent[0] (the stored order) renders first"
    );
}

/// "Tests esperados" #7: a recent id that no longer names a real type is
/// hidden from the group, not pruned from the stored state — a filtered
/// render costs nothing, and a stale `last` already falls back on its own
/// with no help from this filter.
#[tokio::test]
async fn resources_recent_entry_no_longer_a_real_type_is_hidden_not_pruned() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "resources",
        json!({"last": "RetiredType", "recent": [{"id": "RetiredType"}, {"id": "Patient"}]}),
    )
    .await;

    let html = get_ok_html(app_with(store.clone()), "/ui/resources").await;
    let group = recent_group_html(&html, "type-rail-recent");
    assert!(!group.contains(r#"data-type="RetiredType""#));
    assert!(group.contains(r#"data-type="Patient""#));

    let doc = store.peek("l2:").expect("settings stored");
    assert_eq!(
        stored_rail(&doc, "resources")["recent"][0]["id"],
        "RetiredType",
        "hiding an invalid entry must not prune it from storage"
    );
}

/// "Tests esperados" #4: Search Parameters restores a stored `base` the same
/// way a type rail restores `last`.
#[tokio::test]
async fn search_parameters_restores_the_stored_base() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "searchParameters",
        json!({"last": "Encounter", "recent": [{"id": "Encounter"}]}),
    )
    .await;

    let html = get_ok_html(app_with(store), "/ui/search-parameters").await;
    assert!(html.contains(r#"name="base" value="Encounter""#));
}

/// "Tests esperados" #4: `?base=` (explicit, empty) opens "All types" and
/// persists `last: ""` without touching `recent` — and, once that is the
/// resolved state, every link the page itself generates keeps carrying the
/// explicit `base=` marker, so "All types" stays one click away even
/// though a real type is still remembered in `recent`.
#[tokio::test]
async fn search_parameters_explicit_all_types_persists_and_stays_marked() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "searchParameters",
        json!({"last": "Encounter", "recent": [{"id": "Encounter"}]}),
    )
    .await;

    let html = get_ok_html(app_with(store.clone()), "/ui/search-parameters?base=").await;
    assert!(html.contains(r#"name="base" value="""#));

    let doc = store.peek("l2:").expect("settings stored");
    let rail = stored_rail(&doc, "searchParameters");
    assert_eq!(rail["last"], "");
    assert_eq!(
        rail["recent"][0]["id"], "Encounter",
        "select_all leaves recent untouched"
    );

    // A later request with no `?base=` at all must still land on All types —
    // reading the stored `last: ""` — and its own "All types" link must
    // carry the marker rather than a bare `/ui/search-parameters`, which
    // would otherwise be indistinguishable from "never selected" and fall
    // back through `last` again.
    let html = get_ok_html(app_with(store.clone()), "/ui/search-parameters").await;
    assert!(html.contains(r#"name="base" value="""#));
    // Askama HTML-escapes the `&` query separator; `&#38;base=""` is what a
    // browser actually receives (and decodes back to a bare `&`).
    assert!(html.contains(r#"href="/ui/search-parameters?version=R4&#38;base=""#));
}

/// "Tests esperados" #9 (Search Parameters variant of #6): the group renders
/// a resolved base with its live count and marks it `aria-current` when it is
/// also the page's current selection.
#[tokio::test]
async fn search_parameters_recent_group_renders_live_count_and_current() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "searchParameters",
        json!({"last": "Encounter", "recent": [{"id": "Encounter"}]}),
    )
    .await;

    let html = get_ok_html(app_with(store), "/ui/search-parameters").await;
    let group = recent_group_html(&html, "sp-rail-recent");
    assert!(!group[..group.find('>').unwrap()].contains("hidden"));
    assert!(group.contains(r#"data-type="Encounter""#));
    assert!(group.contains(r#"aria-current="true""#));
    assert!(
        group.contains(r#"class="count""#),
        "the live parameter count for Encounter, not a stale snapshot"
    );
}

/// "Tests esperados" #9: Search and Saved Queries restore and record through
/// their own `rails.<page>` keys — never Resources', confirming recents are
/// per page (#603's shared-across-pages model is gone).
#[tokio::test]
async fn search_and_queries_restore_and_record_their_own_rail_state() {
    for (path, page_key) in [("/ui/search", "search"), ("/ui/queries", "queries")] {
        let store = Arc::new(InMemorySettingsStore::new());
        seed_rail(
            &store,
            page_key,
            json!({"last": "Observation", "recent": [{"id": "Observation"}]}),
        )
        .await;

        let html = get_ok_html(app_with(store.clone()), path).await;
        assert!(
            html.contains(r#"data-selected-type="Observation""#),
            "{path} restores its own stored type"
        );

        let html = get_ok_html(app_with(store.clone()), &format!("{path}?type=Encounter")).await;
        assert!(
            html.contains(r#"data-selected-type="Encounter""#),
            "{path} explicit type wins"
        );
        let doc = store.peek("l2:").expect("settings stored");
        assert_eq!(
            stored_rail(&doc, page_key)["last"],
            "Encounter",
            "{path} records the explicit selection"
        );

        // A fresh Resources request under the same document must not see
        // this page's selection: recents are per page, not shared (#603).
        let html = get_ok_html(app_with(store.clone()), "/ui/resources").await;
        assert!(
            html.contains(r#"data-selected-type="Patient""#),
            "{path}'s selection must not leak into Resources"
        );
    }
}

// ── Ticket 03: View Definitions, SQL Queries, SQL Views ───────────────────

fn vd_app_with(settings: Arc<InMemorySettingsStore>, vds: Vec<Value>) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(),
        None,
        Some(settings),
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty().with(
            "ViewDefinition",
            helios_fhir::FhirVersion::R4,
            vds,
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

fn lib_app_with(settings: Arc<InMemorySettingsStore>, libs: Vec<Value>) -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        Some(std::path::PathBuf::from("../../data")),
        nl(),
        None,
        Some(settings),
        "default".to_string(),
        Arc::new(helios_ui::StaticConformanceSource::empty().with(
            "Library",
            helios_fhir::FhirVersion::R4,
            libs,
        )),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

/// The `id="vd-rail-list"` (or `id="lib-rail-list"`) scrollable list's own
/// HTML — mirrors `router_http.rs`'s own `rail_list_html` helper, kept as a
/// separate copy since the two files are separate integration-test crates
/// and cannot share a `fn` without a `support`-style module of their own.
fn rail_list_html<'a>(html: &'a str, list_id: &str) -> &'a str {
    let start = html
        .find(&format!(r#"id="{list_id}""#))
        .unwrap_or_else(|| panic!("{list_id} present"));
    let end = html[start..]
        .find("</div>")
        .map(|i| i + start)
        .unwrap_or_else(|| panic!("{list_id} closing tag present"));
    &html[start..end]
}

/// "Tests esperados" #1 (View Definitions): a stored `last` is restored
/// whether it names an entry on the rail's current page or one that has to
/// be read directly (#741) — and, when it names nothing real at all, the
/// page falls back to the rail's first visible entry, silently (no write).
#[tokio::test]
async fn view_definitions_restores_a_stored_last_on_or_off_the_visible_page_or_falls_back() {
    let vds: Vec<Value> = (1..=51)
        .map(|n| {
            json!({"resourceType": "ViewDefinition", "id": format!("vd{n:03}"),
                "name": format!("vd_{n:03}"), "resource": "Patient"})
        })
        .collect();

    // On the rail's current page (page 1 holds vd001..vd050).
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "viewDefinitions",
        json!({"last": "vd010", "recent": [{"id": "vd010", "name": "vd_010", "meta": "Patient"}]}),
    )
    .await;
    let html = get_ok_html(vd_app_with(store, vds.clone()), "/ui/sql/view-definitions").await;
    assert!(html.contains(r#"<h2 class="page-head__title">vd_010</h2>"#));

    // Off the current page: a direct read by id opens it exactly like an
    // explicit `?vd=` would — still one read, not a search.
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "viewDefinitions",
        json!({"last": "vd051", "recent": [{"id": "vd051", "name": "vd_051", "meta": "Patient"}]}),
    )
    .await;
    let html = get_ok_html(vd_app_with(store, vds.clone()), "/ui/sql/view-definitions").await;
    assert!(html.contains(r#"<h2 class="page-head__title">vd_051</h2>"#));

    // Names nothing real anywhere: falls back to the rail's first visible
    // entry (name-sorted), and the stale value is left untouched.
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "viewDefinitions",
        json!({"last": "ghost", "recent": [{"id": "ghost", "name": "ghost", "meta": "Patient"}]}),
    )
    .await;
    let html = get_ok_html(vd_app_with(store.clone(), vds), "/ui/sql/view-definitions").await;
    assert!(html.contains(r#"<h2 class="page-head__title">vd_001</h2>"#));
    let doc = store.peek("l2:").expect("settings stored");
    assert_eq!(
        stored_rail(&doc, "viewDefinitions")["last"],
        "ghost",
        "a silent fallback must not overwrite the stale value"
    );
}

/// "Tests esperados" #2 (View Definitions): an explicit `?vd=` selection
/// that resolves is recorded with its `{id, name, meta}` snapshot at
/// `recent[0]`/`last`; `?vd=new` never touches the registry; and repeating
/// the same explicit selection writes nothing further.
#[tokio::test]
async fn view_definitions_explicit_selection_is_recorded_with_its_snapshot() {
    let vds = vec![
        json!({"resourceType": "ViewDefinition", "id": "vd1", "name": "active_patients", "resource": "Patient"}),
        json!({"resourceType": "ViewDefinition", "id": "vd2", "name": "blood_pressure", "resource": "Observation"}),
    ];
    let store = Arc::new(InMemorySettingsStore::new());

    get_ok_html(
        vd_app_with(store.clone(), vds.clone()),
        "/ui/sql/view-definitions?vd=vd2",
    )
    .await;
    let doc = store.peek("l2:").expect("settings stored");
    let rail = stored_rail(&doc, "viewDefinitions");
    assert_eq!(rail["last"], "vd2");
    assert_eq!(
        rail["recent"][0],
        json!({"id": "vd2", "name": "blood_pressure", "meta": "Observation"})
    );

    // "Create New" is never a selection.
    get_ok_html(
        vd_app_with(store.clone(), vds.clone()),
        "/ui/sql/view-definitions?vd=new",
    )
    .await;
    let doc = store.peek("l2:").expect("settings stored");
    assert_eq!(
        stored_rail(&doc, "viewDefinitions")["last"],
        "vd2",
        "?vd=new must not touch the registry"
    );

    // Repeating the same, already-current selection writes nothing further.
    let before = store.peek("l2:").expect("settings stored");
    get_ok_html(
        vd_app_with(store.clone(), vds),
        "/ui/sql/view-definitions?vd=vd2",
    )
    .await;
    let after = store.peek("l2:").expect("settings stored");
    assert_eq!(before, after);
}

/// "Tests esperados" #3 (View Definitions): an explicit `?vd=` naming a
/// deleted or mistyped id prunes it from the registry (and `last`, when it
/// named it); the page keeps its current "no selection" render either way.
#[tokio::test]
async fn view_definitions_explicit_stale_selection_is_pruned() {
    let vds = vec![
        json!({"resourceType": "ViewDefinition", "id": "vd1", "name": "active_patients", "resource": "Patient"}),
    ];
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "viewDefinitions",
        json!({
            "last": "gone",
            "recent": [
                {"id": "gone", "name": "gone", "meta": "Patient"},
                {"id": "vd1", "name": "active_patients", "meta": "Patient"}
            ]
        }),
    )
    .await;

    let html = get_ok_html(
        vd_app_with(store.clone(), vds),
        "/ui/sql/view-definitions?vd=gone",
    )
    .await;
    assert!(
        !html.contains(r#"name="json""#),
        "a stale explicit id keeps the page's no-selection render"
    );

    let doc = store.peek("l2:").expect("settings stored");
    let rail = stored_rail(&doc, "viewDefinitions");
    assert_eq!(
        rail["last"],
        Value::Null,
        "last named the pruned id, so it is cleared too"
    );
    assert_eq!(
        rail["recent"],
        json!([{"id": "vd1", "name": "active_patients", "meta": "Patient"}])
    );
}

/// "Tests esperados" #4 (View Definitions): the "Recently used" group is
/// immune to the rail's own `?filter=` — an entry on the current page
/// renders with its live name/meta, overriding a stale snapshot; one the
/// filter excludes falls back to that very snapshot, `href` included.
#[tokio::test]
async fn view_definitions_recent_group_ignores_the_filter_and_falls_back_to_the_snapshot() {
    let vds = vec![
        json!({"resourceType": "ViewDefinition", "id": "keep", "name": "keep_me", "resource": "Encounter"}),
        json!({"resourceType": "ViewDefinition", "id": "other", "name": "other_thing", "resource": "Patient"}),
    ];
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "viewDefinitions",
        json!({
            "last": "keep",
            "recent": [
                {"id": "keep", "name": "old_keep_name", "meta": "Patient"},
                {"id": "other", "name": "stale_other_name", "meta": "Patient"}
            ]
        }),
    )
    .await;

    let html = get_ok_html(
        vd_app_with(store, vds),
        "/ui/sql/view-definitions?filter=keep",
    )
    .await;

    // The rail's own scrollable list only shows what the filter matches.
    let list = rail_list_html(&html, "vd-rail-list");
    assert!(list.contains(r#"data-type="keep""#));
    assert!(!list.contains(r#"data-type="other""#));

    let group = recent_group_html(&html, "vd-rail-recent");
    // "keep" is on the current page: the group shows its live name/meta —
    // not the stale snapshot — and marks it current.
    assert!(group.contains("keep_me"));
    assert!(!group.contains("old_keep_name"));
    assert!(group.contains(r#"class="filter-rail__meta">Encounter<"#));
    assert!(group.contains(r#"data-type="keep""#) && group.contains(r#"aria-current="true""#));
    // "other" is off the current page (the filter excludes it): the group
    // falls back to its stored snapshot, with a working href.
    assert!(group.contains("stale_other_name"));
    assert!(group.contains(r#"class="filter-rail__meta">Patient<"#));
    assert!(group.contains(r#"href="/ui/sql/view-definitions?vd=other""#));
}

/// "Tests esperados" #5 (Libraries): the same restore/record/prune behaviors
/// as View Definitions, and — because `rails.sqlQueries`/`rails.sqlViews` are
/// distinct keys — a selection recorded on one page never surfaces on the
/// other, even against the same stored document.
#[tokio::test]
async fn sql_libraries_restore_record_prune_and_never_cross_contaminate() {
    let system = "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";
    let libs = vec![
        json!({"resourceType": "Library", "id": "q1", "name": "patient_counts", "status": "active",
            "type": {"coding": [{"system": system, "code": "sql-query"}]}}),
        json!({"resourceType": "Library", "id": "q2", "name": "encounter_counts", "status": "draft",
            "type": {"coding": [{"system": system, "code": "sql-query"}]}}),
        json!({"resourceType": "Library", "id": "v1", "name": "flat_patients", "status": "draft",
            "type": {"coding": [{"system": system, "code": "sql-view"}]}}),
    ];

    // A stored `last` restores; a stale one falls back silently.
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "sqlQueries",
        json!({"last": "q2", "recent": [{"id": "q2", "name": "encounter_counts", "meta": "draft"}]}),
    )
    .await;
    let html = get_ok_html(lib_app_with(store, libs.clone()), "/ui/sql/queries").await;
    // #839: the title row's h2 also carries the type icon and the type/
    // status chips, so this checks the selected name inside that element
    // rather than an exact `<h2>...</h2>` match.
    assert!(html.contains(r#"<h2 class="page-head__title page-head__title--kind">"#));
    let title_row = html
        .split(r#"<h2 class="page-head__title page-head__title--kind">"#)
        .nth(1)
        .and_then(|s| s.split("</h2>").next())
        .expect("a title row h2");
    assert!(title_row.contains("encounter_counts"));
    assert!(title_row.contains("tag--draft"));

    // An explicit selection writes its snapshot; a stale explicit id
    // prunes; `?lib=new` never writes.
    let store = Arc::new(InMemorySettingsStore::new());
    get_ok_html(
        lib_app_with(store.clone(), libs.clone()),
        "/ui/sql/queries?lib=q1",
    )
    .await;
    let doc = store.peek("l2:").expect("settings stored");
    assert_eq!(
        stored_rail(&doc, "sqlQueries")["recent"][0],
        json!({"id": "q1", "name": "patient_counts", "meta": "active"})
    );
    get_ok_html(
        lib_app_with(store.clone(), libs.clone()),
        "/ui/sql/queries?lib=new",
    )
    .await;
    assert_eq!(
        stored_rail(&store.peek("l2:").unwrap(), "sqlQueries")["last"],
        "q1",
        "?lib=new must not touch the registry"
    );

    // A stale explicit id already sitting in the registry (e.g. a Library
    // deleted after being picked) is pruned when clicked; one that was never
    // recorded at all is simply a no-op write — RailState::prune has nothing
    // to clean either way, so this seeds "gone" first to exercise the actual
    // removal.
    seed_rail(
        &store,
        "sqlQueries",
        json!({
            "last": "gone",
            "recent": [
                {"id": "gone", "name": "gone", "meta": "active"},
                {"id": "q1", "name": "patient_counts", "meta": "active"}
            ]
        }),
    )
    .await;
    let html = get_ok_html(
        lib_app_with(store.clone(), libs.clone()),
        "/ui/sql/queries?lib=gone",
    )
    .await;
    assert!(!html.contains(r#"name="json""#));
    let doc = store.peek("l2:").unwrap();
    let rail = stored_rail(&doc, "sqlQueries");
    assert_eq!(
        rail["last"],
        Value::Null,
        "the stale explicit id is pruned from last"
    );
    assert_eq!(
        rail["recent"],
        json!([{"id": "q1", "name": "patient_counts", "meta": "active"}])
    );

    // No cross-contamination: SQL Views, under the very same stored
    // document, has never recorded anything of its own and does not see SQL
    // Queries' selections — it opens on its own fallback (its first entry).
    let html = get_ok_html(lib_app_with(store.clone(), libs), "/ui/sql/views").await;
    assert!(
        html.split(r#"<h2 class="page-head__title page-head__title--kind">"#)
            .nth(1)
            .and_then(|s| s.split("</h2>").next())
            .is_some_and(|title_row| title_row.contains("flat_patients"))
    );
    assert!(
        store.peek("l2:").unwrap()["byTenant"]["default"]["rails"]
            .get("sqlViews")
            .is_none(),
        "SQL Views must not have written anything just by opening on its own fallback"
    );
}

/// "Tests esperados" #6: all three SQL rails ship `resource-filter.js`, their
/// own "Recently used" group with the right id, a divider, and a localized
/// "All …" heading — and, with no settings store configured, the group
/// renders present-but-`hidden` exactly like the type rails.
#[tokio::test]
async fn sql_rails_ship_the_recent_group_script_and_localized_all_heading() {
    for (path, group_id, all_heading) in [
        (
            "/ui/sql/view-definitions",
            "vd-rail-recent",
            "All View Definitions",
        ),
        ("/ui/sql/queries", "lib-rail-recent", "All SQL Queries"),
        ("/ui/sql/views", "lib-rail-recent", "All SQL Views"),
    ] {
        let app = helios_ui::mount_with_conformance_source(
            Router::new(),
            "9.9.9",
            Some(std::path::PathBuf::from("../../data")),
            nl(),
            None,
            None,
            "default".to_string(),
            Arc::new(helios_ui::StaticConformanceSource::empty()),
            helios_fhir::FhirVersion::R4,
            None,
            "http://localhost:8080".to_string(),
            None,
        );
        let html = get_ok_html(app, path).await;
        assert!(
            html.contains("/ui/assets/resource-filter.js"),
            "{path} loads resource-filter.js"
        );
        assert!(html.contains(all_heading), "{path}'s localized All heading");
        assert!(
            html.contains("filter-rail__divider"),
            "{path}'s divider between the group and the list"
        );
        let group = recent_group_html(&html, group_id);
        assert!(
            group[..group.find('>').unwrap()].contains("hidden"),
            "{path}: no settings store renders the group hidden, like the type rails"
        );
    }
}

// ── Ticket 04: Compartments ────────────────────────────────────────────────

/// Whether the `<a ...>` tag whose `href` contains `needle` also carries
/// `aria-current="true"` — used to check which rail item the page marks
/// current without depending on the exact (HTML-escaped) query-string shape
/// `CmpQuery::href` produces. `needle` must be specific enough to name a
/// single anchor (a `def=<code>` pair does, since the rail is rendered before
/// the tabs/filters that repeat the *selected* def in their own hrefs).
fn anchor_with_href_is_current(html: &str, needle: &str) -> bool {
    let pos = html
        .find(needle)
        .unwrap_or_else(|| panic!("no anchor href contains {needle:?}"));
    let tag_start = html[..pos].rfind("<a ").expect("an enclosing <a> tag");
    let tag_end = html[tag_start..]
        .find('>')
        .map(|i| i + tag_start)
        .expect("the tag's closing '>'");
    html[tag_start..tag_end].contains(r#"aria-current="true""#)
}

/// "Tests esperados" #1: a stored `last` is restored when the request itself
/// carries no explicit `?def=` — the rail marks it current and the detail
/// panel shows its code, exactly as an explicit `?def=Encounter` would.
#[tokio::test]
async fn compartments_restores_the_stored_last_definition() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "compartments",
        json!({"last": "Encounter", "recent": [{"id": "Encounter"}]}),
    )
    .await;

    let html = get_ok_html(app_with(store), "/ui/compartments").await;
    assert!(
        anchor_with_href_is_current(&html, "def=Encounter"),
        "the Encounter rail item is marked current"
    );
    assert!(
        !anchor_with_href_is_current(&html, "def=Patient"),
        "Patient (the page's own fallback) must not also be current"
    );
    assert!(
        html.contains("<div>Encounter</div>"),
        "the code detail field"
    );
}

/// "Tests esperados" #2: an explicit `?def=` that resolves wins and is
/// recorded; repeating it writes nothing further (RailState::select is a
/// no-op once already current); and an explicit `?def=` that does not name a
/// real definition falls back to today's behavior (Patient → first) without
/// writing anything — the registry is left exactly as the previous, valid
/// selection left it.
#[tokio::test]
async fn compartments_explicit_def_wins_is_recorded_once_and_an_unknown_one_writes_nothing() {
    let store = Arc::new(InMemorySettingsStore::new());

    let html = get_ok_html(app_with(store.clone()), "/ui/compartments?def=Practitioner").await;
    assert!(anchor_with_href_is_current(&html, "def=Practitioner"));
    let doc = store.peek("l2:").expect("settings stored");
    let rail = stored_rail(&doc, "compartments");
    assert_eq!(rail["last"], "Practitioner");
    assert_eq!(rail["recent"], json!([{"id": "Practitioner"}]));

    // Repeating the same, already-current selection writes nothing further.
    let before = store.peek("l2:").expect("settings stored");
    get_ok_html(app_with(store.clone()), "/ui/compartments?def=Practitioner").await;
    let after = store.peek("l2:").expect("settings stored");
    assert_eq!(before, after);

    // An unknown explicit code falls back to today's behavior (Patient) and
    // leaves the still-valid stored selection untouched — it is neither
    // adopted nor pruned.
    let html = get_ok_html(app_with(store.clone()), "/ui/compartments?def=Nope").await;
    assert!(anchor_with_href_is_current(&html, "def=Patient"));
    assert!(html.contains("<div>Patient</div>"));
    let doc = store.peek("l2:").expect("settings stored");
    assert_eq!(
        stored_rail(&doc, "compartments")["last"],
        "Practitioner",
        "an unknown explicit def must not touch the registry"
    );
}

/// "Tests esperados" #3: a stored `last` that no longer names a real
/// definition (e.g. read against a version/tenant whose set has moved on)
/// falls back to the page's own default in silence — no error, and the stale
/// value is left untouched rather than overwritten by the fallback.
#[tokio::test]
async fn compartments_a_stale_stored_last_falls_back_silently_and_writes_nothing() {
    let store = Arc::new(InMemorySettingsStore::new());
    seed_rail(
        &store,
        "compartments",
        json!({"last": "NoSuchCompartment", "recent": [{"id": "NoSuchCompartment"}]}),
    )
    .await;

    let html = get_ok_html(app_with(store.clone()), "/ui/compartments").await;
    assert!(anchor_with_href_is_current(&html, "def=Patient"));

    let doc = store.peek("l2:").expect("settings stored");
    assert_eq!(
        stored_rail(&doc, "compartments")["last"],
        "NoSuchCompartment",
        "a silent fallback must not overwrite the stale value"
    );
}
