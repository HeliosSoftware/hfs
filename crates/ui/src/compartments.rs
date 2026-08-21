//! Read model for the Compartment viewer & tester (`/ui/compartments`, #237).
//!
//! Definition metadata comes from the server's own `GET /CompartmentDefinition`
//! endpoint (primary storage is the source of truth — the server seeds the spec
//! definitions there at startup), fetched over HTTP via a [`ConformanceSource`].
//! The membership chips and the tester, however, resolve through
//! [`helios_fhir::compartment_params`] — the codegen'd table the REST
//! compartment handler actually consults — so what this screen shows is what
//! the server does. The seeded bundle is kept in step with the codegen table by
//! a parity test in `helios_fhir::compartment::loader`.
//!
//! The spec's degenerate `questionnaire` compartment (no `code`, zero
//! `resource` entries) is excluded from the shipped bundle.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use helios_fhir::FhirVersion;
use serde::Deserialize;

use crate::conformance::ConformanceSource;

/// One CompartmentDefinition, trimmed to the fields the screen shows. Fetched
/// from the server's `GET /CompartmentDefinition` endpoint (storage is the
/// source of truth) and deserialized here.
#[derive(Deserialize, Clone)]
pub(crate) struct CompartmentDef {
    /// The stored resource id — what Edit deep-links and Delete address.
    /// Empty when the server returned the definition without one.
    #[serde(default)]
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub version: String,
    pub status: String,
    #[serde(default)]
    pub experimental: bool,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub description: String,
    pub code: String,
    pub search: bool,
    pub resource: Vec<CompartmentResource>,
}

#[derive(Deserialize, Clone)]
pub(crate) struct CompartmentResource {
    pub code: String,
    /// Only the parity test reads this; the live view resolves linking
    /// params through the runtime table (`runtime_params`) instead.
    #[cfg_attr(not(test), allow(dead_code))]
    #[serde(default)]
    pub param: Vec<String>,
}

/// Lazily-fetched, process-lifetime CompartmentDefinitions, one set per enabled
/// FHIR version. Fetched from the server's own endpoint once per version and
/// cached; sorted by compartment code.
/// Cache key: the tenant the definitions were fetched for, and the version.
type TenantVersion = (String, FhirVersion);

pub(crate) struct CompartmentCatalog {
    source: Arc<dyn ConformanceSource>,
    cache: Mutex<HashMap<TenantVersion, Arc<Vec<CompartmentDef>>>>,
}

impl CompartmentCatalog {
    pub fn new(source: Arc<dyn ConformanceSource>) -> Self {
        CompartmentCatalog {
            source,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The definitions for a version, fetching on first use. A failed fetch
    /// yields an empty set (the page degrades to a warning).
    pub async fn definitions(
        &self,
        tenant: &str,
        version: FhirVersion,
    ) -> Arc<Vec<CompartmentDef>> {
        let key = (tenant.to_string(), version);
        if let Some(cached) = self.cache.lock().expect("compartment lock").get(&key) {
            return cached.clone();
        }
        let fetched = self
            .source
            .fetch("CompartmentDefinition", version, tenant)
            .await;
        let fetch_ok = fetched.is_ok();
        let mut defs: Vec<CompartmentDef> = match fetched {
            Ok(resources) => resources
                .into_iter()
                .filter_map(|r| serde_json::from_value(r).ok())
                .collect(),
            Err(_) => Vec::new(),
        };
        defs.sort_by(|a, b| a.code.cmp(&b.code));
        let built = Arc::new(defs);
        // A failed fetch is served empty for this request only — caching it
        // would pin the page to the failure until restart. An *empty success*
        // is treated the same (#462): the server seeds the spec definitions
        // at startup, so emptiness means the search path hasn't caught up
        // (a composite backend's index still syncing), and caching it would
        // keep the page broken long after the sync lands.
        if !fetch_ok || built.is_empty() {
            return built;
        }
        self.cache
            .lock()
            .expect("compartment lock")
            .entry(key)
            .or_insert_with(|| built.clone())
            .clone()
    }

    /// Drops the cached definitions for a tenant + version so the next request
    /// re-fetches. The page calls this on `?refresh=1`, which the CRUD flows
    /// append after a write lands through the FHIR API (#237).
    pub fn invalidate(&self, tenant: &str, version: FhirVersion) {
        self.cache
            .lock()
            .expect("compartment lock")
            .remove(&(tenant.to_string(), version));
    }

    /// Every resource type of the version, from the first CompartmentDefinition
    /// (each enumerates the full set — 145 in R4, 157 in R5). Used by the
    /// resource pickers on the Search, Queries, Resources, and Bulk Export
    /// pages, which pass the sidebar's selected version (#562). For versions
    /// other than the server's seeded default the definitions come from the
    /// shipped spec bundles, not storage — see [`crate::conformance`].
    pub async fn resource_type_names(&self, tenant: &str, version: FhirVersion) -> Vec<String> {
        self.definitions(tenant, version)
            .await
            .first()
            .map(|def| def.resource.iter().map(|r| r.code.clone()).collect())
            .unwrap_or_default()
    }
}

impl CompartmentDef {
    /// Member = the runtime table returns at least one linking parameter.
    /// `{def}` (the compartment resource itself) counts as a member.
    pub fn member_count(&self, version: FhirVersion) -> usize {
        self.resource
            .iter()
            .filter(|r| !runtime_params(version, &self.code, &r.code).is_empty())
            .count()
    }
}

/// The codegen'd linking parameters the compartment handler consults.
pub(crate) fn runtime_params(
    version: FhirVersion,
    compartment: &str,
    resource_type: &str,
) -> &'static [&'static str] {
    helios_fhir::compartment_params(version, compartment, resource_type)
}

// ---------------------------------------------------------------------------
// View model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tab {
    Definition,
    Members,
    Tester,
}

impl Tab {
    pub fn from_str(value: Option<&str>) -> Tab {
        match value {
            Some("members") => Tab::Members,
            Some("tester") => Tab::Tester,
            _ => Tab::Definition,
        }
    }
    pub fn key(&self) -> &'static str {
        match self {
            Tab::Definition => "definition",
            Tab::Members => "members",
            Tab::Tester => "tester",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum MemberFilter {
    Members,
    All,
    Excluded,
}

impl MemberFilter {
    pub fn from_str(value: Option<&str>) -> MemberFilter {
        match value {
            Some("all") => MemberFilter::All,
            Some("excluded") => MemberFilter::Excluded,
            _ => MemberFilter::Members,
        }
    }
    pub fn key(&self) -> &'static str {
        match self {
            MemberFilter::Members => "members",
            MemberFilter::All => "all",
            MemberFilter::Excluded => "excluded",
        }
    }
}

/// Tester outcome, mirroring the REST handler's decisions
/// (`crates/rest/src/handlers/compartment.rs`).
pub(crate) enum TesterOutcome {
    /// Member via linking params; `flat_search` is the equivalent search the
    /// server runs. `self_def` marks the `{def}` case (the compartment
    /// resource itself).
    Member {
        params: Vec<String>,
        route: String,
        flat_search: String,
        self_def: bool,
    },
    /// Not a member: the handler returns 404 with an OperationOutcome.
    NotMember { route: String, preview: String },
    /// `*` fan-out across every member type; excluded types are skipped,
    /// not failed.
    FanOut {
        route: String,
        member_types: Vec<String>,
        total: usize,
    },
}

pub(crate) struct CmpRailItem {
    pub code: String,
    pub members: usize,
    pub total: usize,
    pub href: String,
    pub current: bool,
}

pub(crate) struct CmpTab {
    pub key: &'static str,
    pub href: String,
    pub current: bool,
}

pub(crate) struct CmpFilter {
    pub key: &'static str,
    pub count: usize,
    pub href: String,
    pub current: bool,
}

pub(crate) struct MemberRow {
    pub name: String,
    pub member: bool,
    pub params: Vec<String>,
}

/// Tester outcome flattened for the template: `kind` is one of
/// "member" | "self" | "notmember" | "fanout".
pub(crate) struct TesterView {
    pub id: String,
    pub target: String,
    pub kind: &'static str,
    pub route: String,
    pub params: String,
    pub body: String,
    pub member_types: String,
    pub total: usize,
}

impl TesterView {
    fn new(id: String, target: String, outcome: TesterOutcome) -> TesterView {
        match outcome {
            TesterOutcome::Member {
                params,
                route,
                flat_search,
                self_def,
            } => TesterView {
                id,
                target,
                kind: if self_def { "self" } else { "member" },
                route,
                params: params.join(" OR "),
                body: flat_search,
                member_types: String::new(),
                total: 0,
            },
            TesterOutcome::NotMember { route, preview } => TesterView {
                id,
                target,
                kind: "notmember",
                route,
                params: String::new(),
                body: preview,
                member_types: String::new(),
                total: 0,
            },
            TesterOutcome::FanOut {
                route,
                member_types,
                total,
            } => {
                let shown: Vec<&String> = member_types.iter().take(18).collect();
                let mut list = shown
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if member_types.len() > 18 {
                    list.push_str(", …");
                }
                TesterView {
                    id,
                    target,
                    kind: "fanout",
                    route,
                    params: String::new(),
                    body: String::new(),
                    member_types: list,
                    total,
                }
            }
        }
    }
}

pub(crate) struct CmpView {
    pub versions: Vec<crate::search_params::VersionLink>,
    pub rail: Vec<CmpRailItem>,
    pub def: CompartmentDef,
    pub tab: &'static str,
    pub tabs: Vec<CmpTab>,
    pub member_filters: Vec<CmpFilter>,
    pub members: Vec<MemberRow>,
    pub tester: TesterView,
    /// Hidden inputs so the tester form round-trips version/def/tab.
    pub hidden_fields: Vec<(String, String)>,
}

/// Parsed query state for the page.
#[derive(Clone, Default)]
pub(crate) struct CmpQuery {
    pub version: Option<String>,
    pub def: Option<String>,
    pub tab: Option<String>,
    pub filter: Option<String>,
    pub id: String,
    pub target: String,
}

impl CmpQuery {
    pub fn fhir_version(&self) -> FhirVersion {
        self.version
            .as_deref()
            .and_then(crate::search_params::version_from_str)
            .unwrap_or_default()
    }

    fn href(&self, def: &str, tab: Tab, filter: MemberFilter) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(v) = &self.version {
            parts.push(format!("version={v}"));
        }
        parts.push(format!("def={def}"));
        parts.push(format!("tab={}", tab.key()));
        if filter != MemberFilter::Members {
            parts.push(format!("filter={}", filter.key()));
        }
        format!("/ui/compartments?{}", parts.join("&"))
    }
}

/// Assembles the whole page state. Returns `None` when the build has no
/// FHIR version enabled with compartment data (not a supported target).
pub(crate) fn build_view(query: &CmpQuery, defs: &[CompartmentDef]) -> Option<CmpView> {
    let version = query.fhir_version();
    let def = query
        .def
        .as_deref()
        .and_then(|code| defs.iter().find(|d| d.code == code))
        .or_else(|| {
            defs.iter()
                .find(|d| d.code == "Patient")
                .or_else(|| defs.first())
        })?;
    let tab = Tab::from_str(query.tab.as_deref());
    let filter = MemberFilter::from_str(query.filter.as_deref());

    let rail = defs
        .iter()
        .map(|d| CmpRailItem {
            code: d.code.clone(),
            members: d.member_count(version),
            total: d.resource.len(),
            href: query.href(&d.code, tab, filter),
            current: d.code == def.code,
        })
        .collect();

    let tabs = [Tab::Definition, Tab::Members, Tab::Tester]
        .into_iter()
        .map(|t| CmpTab {
            key: t.key(),
            href: query.href(&def.code, t, filter),
            current: t == tab,
        })
        .collect();

    let member_count = def.member_count(version);
    let total_count = def.resource.len();
    let member_filters = [
        (MemberFilter::Members, member_count),
        (MemberFilter::All, total_count),
        (MemberFilter::Excluded, total_count - member_count),
    ]
    .into_iter()
    .map(|(f, count)| CmpFilter {
        key: f.key(),
        count,
        href: query.href(&def.code, tab, f),
        current: f == filter,
    })
    .collect();

    let members = def
        .resource
        .iter()
        .filter_map(|r| {
            let params = runtime_params(version, &def.code, &r.code);
            let member = !params.is_empty();
            match filter {
                MemberFilter::Members if !member => return None,
                MemberFilter::Excluded if member => return None,
                _ => {}
            }
            Some(MemberRow {
                name: r.code.clone(),
                member,
                params: params.iter().map(|p| p.to_string()).collect(),
            })
        })
        .collect();

    let id = if query.id.trim().is_empty() {
        "example".to_string()
    } else {
        query.id.trim().to_string()
    };
    let target = if query.target.trim().is_empty() {
        "Observation".to_string()
    } else {
        query.target.trim().to_string()
    };
    let outcome = run_tester(version, def, &id, &target);
    let tester = TesterView::new(id, target, outcome);

    let versions = crate::search_params::enabled_versions()
        .into_iter()
        .map(|v| {
            let mut q = query.clone();
            q.version = Some(v.as_str().to_string());
            crate::search_params::VersionLink {
                label: v.as_str(),
                href: q.href(&def.code, tab, filter),
                current: v == version,
            }
        })
        .collect();

    let mut hidden_fields: Vec<(String, String)> = Vec::new();
    if let Some(v) = &query.version {
        hidden_fields.push(("version".into(), v.clone()));
    }
    hidden_fields.push(("def".into(), def.code.clone()));
    hidden_fields.push(("tab".into(), "tester".into()));

    Some(CmpView {
        versions,
        rail,
        def: def.clone(),
        tab: tab.key(),
        tabs,
        member_filters,
        members,
        tester,
        hidden_fields,
    })
}

pub(crate) fn run_tester(
    version: FhirVersion,
    def: &CompartmentDef,
    id: &str,
    target: &str,
) -> TesterOutcome {
    let id = if id.is_empty() { "example" } else { id };
    if target == "*" {
        let member_types: Vec<String> = def
            .resource
            .iter()
            .filter(|r| !runtime_params(version, &def.code, &r.code).is_empty())
            .map(|r| r.code.clone())
            .collect();
        return TesterOutcome::FanOut {
            route: format!("GET /{}/{}/*", def.code, id),
            total: member_types.len(),
            member_types,
        };
    }

    let params = runtime_params(version, &def.code, target);
    let route = format!("GET /{}/{}/{}", def.code, id, target);
    if params.is_empty() {
        let preview = format!(
            "{route}\n\u{2192} 404 Not Found\n{{ \"resourceType\": \"OperationOutcome\",\n  \"issue\": [{{ \"severity\": \"error\", \"code\": \"not-found\",\n    \"diagnostics\": \"{target} is not a member of the {} compartment\" }}] }}",
            def.code
        );
        return TesterOutcome::NotMember { route, preview };
    }

    let self_def = params == ["{def}"];
    let flat_search = if self_def {
        format!("GET /{}/{}", def.code, id)
    } else {
        let clauses: Vec<String> = params
            .iter()
            .map(|p| format!("{p}={}/{id}", def.code))
            .collect();
        format!("GET /{target}?\n  {}", clauses.join("  OR  "))
    };
    TesterOutcome::Member {
        params: params.iter().map(|p| p.to_string()).collect(),
        route,
        flat_search,
        self_def,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "R4")]
    const R4: FhirVersion = FhirVersion::R4;

    /// A source whose first fetch is empty and later fetches carry data, the
    /// way a composite backend behaves while its search index still syncs.
    struct WarmingSource(std::sync::atomic::AtomicUsize);

    #[async_trait::async_trait]
    impl ConformanceSource for WarmingSource {
        async fn fetch(
            &self,
            _rt: &str,
            _v: FhirVersion,
            _t: &str,
        ) -> Result<Vec<serde_json::Value>, String> {
            let call = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                return Ok(Vec::new());
            }
            Ok(vec![serde_json::json!({
                "resourceType": "CompartmentDefinition",
                "url": "http://example.org/CompartmentDefinition/patient",
                "status": "active",
                "code": "Patient",
                "search": true,
                "resource": []
            })])
        }
    }

    /// #462: an empty success must not be cached — the next request retries
    /// and sees the definitions once the backend's index catches up.
    #[tokio::test]
    async fn empty_definitions_are_not_pinned() {
        let catalog = CompartmentCatalog::new(Arc::new(WarmingSource(0.into())));
        let cold = catalog.definitions("t", FhirVersion::default()).await;
        assert!(cold.is_empty(), "first fetch is empty");
        let warm = catalog.definitions("t", FhirVersion::default()).await;
        assert_eq!(warm.len(), 1, "second request re-fetched");
    }

    /// The R4 compartment definitions, parsed from the shipped `data/` bundle
    /// exactly as an HTTP fetch would deliver them — keeps the test offline.
    #[cfg(feature = "R4")]
    fn r4_defs() -> Vec<CompartmentDef> {
        let raw = std::fs::read_to_string("../../data/compartment-definitions-r4.json")
            .expect("r4 compartment bundle in ../../data");
        let bundle: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let mut defs: Vec<CompartmentDef> = bundle["entry"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| serde_json::from_value(e["resource"].clone()).unwrap())
            .collect();
        defs.sort_by(|a, b| a.code.cmp(&b.code));
        defs
    }

    /// The shipped `data/` bundle must say exactly what the codegen'd runtime
    /// table says — every (compartment, type) slot. (All-version parity is
    /// additionally guarded in `helios_fhir::compartment::loader`.)
    #[cfg(feature = "R4")]
    #[test]
    fn data_bundle_matches_codegen_table() {
        for def in &r4_defs() {
            for resource in &def.resource {
                let runtime = runtime_params(R4, &def.code, &resource.code);
                assert_eq!(
                    runtime,
                    resource.param.as_slice(),
                    "R4 compartment {} / {}",
                    def.code,
                    resource.code
                );
            }
        }
    }

    #[tokio::test]
    async fn invalidate_drops_the_cached_definitions() {
        let source = crate::conformance::StaticConformanceSource::empty().with(
            "CompartmentDefinition",
            FhirVersion::default(),
            vec![serde_json::json!({
                "resourceType": "CompartmentDefinition",
                "id": "cd-1",
                "url": "http://example.org/CompartmentDefinition/patient",
                "status": "active",
                "code": "Patient",
                "search": true,
                "resource": []
            })],
        );
        let catalog = CompartmentCatalog::new(std::sync::Arc::new(source));
        let first = catalog.definitions("t1", FhirVersion::default()).await;
        let cached = catalog.definitions("t1", FhirVersion::default()).await;
        assert!(
            std::sync::Arc::ptr_eq(&first, &cached),
            "second read serves the cache"
        );

        catalog.invalidate("t1", FhirVersion::default());
        let fresh = catalog.definitions("t1", FhirVersion::default()).await;
        assert!(
            !std::sync::Arc::ptr_eq(&first, &fresh),
            "invalidate forces a re-fetch"
        );
        assert_eq!(fresh[0].id, "cd-1");
    }

    #[cfg(feature = "R4")]
    #[test]
    fn defs_keep_the_resource_id_for_the_editor_links() {
        for def in &r4_defs() {
            assert!(!def.id.is_empty(), "id missing on {}", def.url);
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn r4_ships_the_five_spec_compartments() {
        let defs = r4_defs();
        let codes: Vec<&str> = defs.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "Device",
                "Encounter",
                "Patient",
                "Practitioner",
                "RelatedPerson"
            ]
        );
        let patient = defs.iter().find(|d| d.code == "Patient").unwrap();
        assert_eq!(patient.resource.len(), 145);
        assert_eq!(patient.member_count(R4), 66);
    }

    #[cfg(feature = "R4")]
    #[test]
    fn tester_resolves_membership_like_the_handler() {
        let defs = r4_defs();
        let patient = defs.iter().find(|d| d.code == "Patient").unwrap();

        match run_tester(R4, patient, "example", "Observation") {
            TesterOutcome::Member {
                params,
                flat_search,
                self_def,
                ..
            } => {
                assert_eq!(params, ["subject", "performer"]);
                assert!(flat_search.contains("subject=Patient/example"));
                assert!(flat_search.contains("OR  performer=Patient/example"));
                assert!(!self_def);
            }
            _ => panic!("Observation is a Patient-compartment member"),
        }

        match run_tester(R4, patient, "example", "Medication") {
            TesterOutcome::NotMember { preview, .. } => {
                assert!(preview.contains("404"));
                assert!(preview.contains("OperationOutcome"));
            }
            _ => panic!("Medication is not a Patient-compartment member"),
        }

        match run_tester(R4, patient, "example", "*") {
            TesterOutcome::FanOut { total, .. } => assert_eq!(total, 66),
            _ => panic!("* fans out"),
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn tester_reports_the_self_definition_case() {
        let defs = r4_defs();
        let encounter = defs.iter().find(|d| d.code == "Encounter").unwrap();
        match run_tester(R4, encounter, "e1", "Encounter") {
            TesterOutcome::Member {
                self_def,
                flat_search,
                ..
            } => {
                assert!(self_def, "{{def}} marks the compartment resource itself");
                assert_eq!(flat_search, "GET /Encounter/e1");
            }
            _ => panic!("Encounter is in its own compartment via {{def}}"),
        }
    }

    #[cfg(feature = "R4")]
    #[test]
    fn view_defaults_to_patient_and_builds_all_tabs() {
        let view = build_view(&CmpQuery::default(), &r4_defs()).expect("view builds");
        assert_eq!(view.def.code, "Patient");
        assert_eq!(view.rail.len(), 5);
        assert_eq!(view.tabs.len(), 3);
        assert_eq!(view.member_filters.len(), 3);
        // Default filter: members only.
        assert!(view.members.iter().all(|m| m.member));
    }
}
