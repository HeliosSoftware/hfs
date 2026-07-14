//! Read model for the SearchParameter viewer (`/ui/search-parameters`, #238).
//!
//! The screen is a read-only view of the registry the server actually
//! resolves searches against. The snapshot is seeded exactly the way the
//! storage backends seed their runtime registries: the minimal embedded
//! fallback first, then the full spec bundle from the data directory
//! (`search-parameters-<version>.json`), first canonical URL wins — the same
//! precedence `SearchParameterRegistry::register` applies. Tenant-scoped
//! (`Stored`) parameters join this view once #235 makes storage the source
//! of truth; until then the write half of the screen stays off.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use helios_fhir::FhirVersion;
use helios_fhir::search::{
    SearchParameterDefinition, SearchParameterLoader, SearchParameterSource,
};

/// Rows per page. The issue calls for pagination rather than a render cap.
pub(crate) const PAGE_SIZE: usize = 50;

/// Lazily-loaded, process-lifetime snapshot of the search-parameter read
/// model, one per enabled FHIR version.
pub(crate) struct SpCatalog {
    data_dir: Option<PathBuf>,
    versions: Mutex<HashMap<FhirVersion, Arc<VersionSnapshot>>>,
}

pub(crate) struct VersionSnapshot {
    /// Every registered definition, sorted by (code, url) for stable paging.
    pub params: Vec<Arc<SearchParameterDefinition>>,
    /// False when the spec bundle was not found in the data directory —
    /// the snapshot is then only the minimal embedded fallback, and the
    /// page says so instead of silently under-reporting.
    pub spec_loaded: bool,
}

impl SpCatalog {
    pub fn new(data_dir: Option<PathBuf>) -> Self {
        SpCatalog {
            data_dir,
            versions: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the snapshot for a version, loading it on first use.
    pub fn snapshot(&self, version: FhirVersion) -> Arc<VersionSnapshot> {
        let mut versions = self.versions.lock().expect("catalog lock");
        versions
            .entry(version)
            .or_insert_with(|| Arc::new(load_snapshot(version, self.data_dir.as_deref())))
            .clone()
    }
}

fn load_snapshot(version: FhirVersion, data_dir: Option<&Path>) -> VersionSnapshot {
    let loader = SearchParameterLoader::new(version);
    let mut params: Vec<Arc<SearchParameterDefinition>> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |defs: Vec<SearchParameterDefinition>| {
        for def in defs {
            if seen.insert(def.url.clone()) {
                params.push(Arc::new(def));
            }
        }
    };

    if let Ok(embedded) = loader.load_embedded() {
        push(embedded);
    }
    let dir = data_dir.unwrap_or_else(|| Path::new("./data"));
    // A missing or unreadable spec bundle is surfaced in the page itself
    // (`spec_loaded`), not just swallowed.
    let spec_loaded = match loader.load_from_spec_file(dir) {
        Ok(spec) => {
            push(spec);
            true
        }
        Err(_) => false,
    };

    params.sort_by(|a, b| a.code.cmp(&b.code).then_with(|| a.url.cmp(&b.url)));
    VersionSnapshot {
        params,
        spec_loaded,
    }
}

/// Parsed query state for the page. Every filter is also a link, so the
/// whole screen works without JavaScript.
#[derive(Clone, Default)]
pub(crate) struct SpQuery {
    pub version: Option<String>,
    pub base: Option<String>,
    pub ptype: Option<String>,
    pub source: Option<String>,
    pub q: String,
    pub page: usize,
    pub sel: Option<String>,
}

impl SpQuery {
    pub fn fhir_version(&self) -> FhirVersion {
        self.version
            .as_deref()
            .and_then(version_from_str)
            .unwrap_or_default()
    }

    /// Serializes the state to an href, with `page` and `sel` reset unless
    /// explicitly carried over.
    fn href_with(
        &self,
        base: Option<&str>,
        ptype: Option<&str>,
        source: Option<&str>,
        page: usize,
        sel: Option<&str>,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(v) = &self.version {
            parts.push(format!("version={}", urlencode(v)));
        }
        if let Some(b) = base {
            parts.push(format!("base={}", urlencode(b)));
        }
        if let Some(t) = ptype {
            parts.push(format!("type={}", urlencode(t)));
        }
        if let Some(s) = source {
            parts.push(format!("source={}", urlencode(s)));
        }
        if !self.q.is_empty() {
            parts.push(format!("q={}", urlencode(&self.q)));
        }
        if page > 1 {
            parts.push(format!("page={page}"));
        }
        if let Some(s) = sel {
            parts.push(format!("sel={}", urlencode(s)));
        }
        if parts.is_empty() {
            "/ui/search-parameters".to_string()
        } else {
            format!("/ui/search-parameters?{}", parts.join("&"))
        }
    }

    fn href_current(&self, page: usize, sel: Option<&str>) -> String {
        self.href_with(
            self.base.as_deref(),
            self.ptype.as_deref(),
            self.source.as_deref(),
            page,
            sel,
        )
    }
}

pub(crate) fn version_from_str(value: &str) -> Option<FhirVersion> {
    enabled_versions()
        .into_iter()
        .find(|v| v.as_str().eq_ignore_ascii_case(value))
}

/// FHIR versions compiled into this build, in spec order.
#[allow(clippy::vec_init_then_push)] // each push is cfg-gated on a version feature
pub(crate) fn enabled_versions() -> Vec<FhirVersion> {
    let mut versions = Vec::new();
    #[cfg(feature = "R4")]
    versions.push(FhirVersion::R4);
    #[cfg(feature = "R4B")]
    versions.push(FhirVersion::R4B);
    #[cfg(feature = "R5")]
    versions.push(FhirVersion::R5);
    #[cfg(feature = "R6")]
    versions.push(FhirVersion::R6);
    versions
}

fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn source_key(source: SearchParameterSource) -> &'static str {
    match source {
        SearchParameterSource::Embedded => "embedded",
        SearchParameterSource::Stored => "stored",
        SearchParameterSource::Config => "config",
    }
}

fn source_fluent_key(source: &str) -> &'static str {
    match source {
        "stored" => "sp-source-stored",
        "config" => "sp-source-config",
        _ => "sp-source-embedded",
    }
}

fn chip_fluent_key(kind: &str) -> &'static str {
    match kind {
        "conflict" => "sp-chip-conflict",
        "overrides" => "sp-chip-overrides",
        _ => "sp-chip-shadowed",
    }
}

// ---------------------------------------------------------------------------
// View model
// ---------------------------------------------------------------------------

pub(crate) struct RailItem {
    pub name: String,
    pub count: usize,
    pub href: String,
    pub current: bool,
}

pub(crate) struct Facet {
    pub label: String,
    /// Fluent key for localized labels; empty when `label` is technical
    /// vocabulary shown as-is (parameter types).
    pub key: String,
    pub count: usize,
    pub href: String,
    pub pressed: bool,
}

/// One `(base, code)` slot-mate relationship, mirroring how
/// `params_by_type` indexes definitions. Same-source mates are the
/// `DuplicateCode` conflict #242 rejects; cross-source mates are a
/// supported override resolved by source precedence.
pub(crate) struct SlotChip {
    /// "conflict" | "overrides" | "shadowed"
    pub kind: &'static str,
    /// Fluent key for the chip label.
    pub key: &'static str,
}

pub(crate) struct SpRow {
    pub code: String,
    pub url: String,
    pub type_label: String,
    pub bases: String,
    pub expression: String,
    pub source: &'static str,
    /// Fluent key for the localized source label.
    pub source_key: &'static str,
    pub chips: Vec<SlotChip>,
    pub href: String,
    pub selected: bool,
}

pub(crate) struct SpNote {
    /// "error" | "warn" | "info"
    pub severity: &'static str,
    /// Maps to an `sp-note-*` Fluent key in the template.
    pub kind: &'static str,
    /// Single message argument (canonical URL or slot, depending on kind).
    pub arg: String,
}

pub(crate) struct SpDetail {
    pub code: String,
    pub name: String,
    pub url: String,
    pub type_label: String,
    pub bases: Vec<String>,
    pub expression: String,
    pub description: String,
    pub targets: Vec<String>,
    pub components: Vec<String>,
    pub status: String,
    pub source: &'static str,
    /// Fluent key for the localized source label.
    pub source_key: &'static str,
    pub notes: Vec<SpNote>,
}

pub(crate) struct VersionLink {
    pub label: &'static str,
    pub href: String,
    pub current: bool,
}

pub(crate) struct PageLink {
    pub label: String,
    pub href: String,
    pub current: bool,
}

pub(crate) struct SpView {
    pub versions: Vec<VersionLink>,
    pub rail_all: RailItem,
    pub rail: Vec<RailItem>,
    pub q: String,
    pub facets_type: Vec<Facet>,
    pub facets_source: Vec<Facet>,
    pub rows: Vec<SpRow>,
    pub total: usize,
    pub page_count: usize,
    pub prev_href: Option<String>,
    pub next_href: Option<String>,
    pub pages: Vec<PageLink>,
    pub detail: Option<SpDetail>,
    pub spec_loaded: bool,
    /// Hidden inputs so the rail's search form round-trips the other filters.
    pub hidden_fields: Vec<(String, String)>,
}

pub(crate) fn build_view(snapshot: &VersionSnapshot, query: &SpQuery) -> SpView {
    let params = &snapshot.params;
    let version = query.fhir_version();

    // (base, code) slot map over the whole snapshot, the way the registry
    // indexes `params_by_type`, to surface shadows and conflicts.
    let mut slots: HashMap<(&str, &str), Vec<&Arc<SearchParameterDefinition>>> = HashMap::new();
    for param in params {
        for base in &param.base {
            slots
                .entry((base.as_str(), param.code.as_str()))
                .or_default()
                .push(param);
        }
    }
    let chips_for = |param: &Arc<SearchParameterDefinition>| -> Vec<SlotChip> {
        let mut kinds: Vec<&'static str> = Vec::new();
        for base in &param.base {
            for other in &slots[&(base.as_str(), param.code.as_str())] {
                if Arc::ptr_eq(other, param) {
                    continue;
                }
                let kind = if other.source == param.source {
                    "conflict"
                } else if source_rank(param.source) > source_rank(other.source) {
                    "overrides"
                } else {
                    "shadowed"
                };
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
        }
        kinds
            .into_iter()
            .map(|kind| SlotChip {
                kind,
                key: chip_fluent_key(kind),
            })
            .collect()
    };

    // Rail: base resource types with live counts, filtered by the search box.
    let mut base_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for param in params {
        for base in &param.base {
            *base_counts.entry(base.as_str()).or_default() += 1;
        }
    }
    let needle = query.q.to_lowercase();
    let rail: Vec<RailItem> = base_counts
        .iter()
        .filter(|(name, _)| needle.is_empty() || name.to_lowercase().contains(&needle))
        .map(|(name, count)| RailItem {
            name: (*name).to_string(),
            count: *count,
            href: query.href_with(
                Some(name),
                query.ptype.as_deref(),
                query.source.as_deref(),
                1,
                None,
            ),
            current: query.base.as_deref() == Some(*name),
        })
        .collect();
    let rail_all = RailItem {
        name: String::new(),
        count: params.len(),
        href: query.href_with(None, query.ptype.as_deref(), query.source.as_deref(), 1, None),
        current: query.base.is_none(),
    };

    // Facets are scoped to the current base type and show live counts.
    let in_base: Vec<&Arc<SearchParameterDefinition>> = params
        .iter()
        .filter(|p| match &query.base {
            Some(base) => p.base.iter().any(|b| b == base),
            None => true,
        })
        .collect();

    let mut type_counts: HashMap<String, usize> = HashMap::new();
    let mut source_counts: HashMap<&'static str, usize> = HashMap::new();
    for param in &in_base {
        *type_counts
            .entry(param.param_type.to_string())
            .or_default() += 1;
        *source_counts.entry(source_key(param.source)).or_default() += 1;
    }
    let mut facets_type: Vec<Facet> = type_counts
        .into_iter()
        .map(|(label, count)| {
            let pressed = query.ptype.as_deref() == Some(label.as_str());
            Facet {
                href: query.href_with(
                    query.base.as_deref(),
                    if pressed { None } else { Some(label.as_str()) },
                    query.source.as_deref(),
                    1,
                    None,
                ),
                pressed,
                key: String::new(),
                label,
                count,
            }
        })
        .collect();
    facets_type.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));
    let mut facets_source: Vec<Facet> = source_counts
        .into_iter()
        .map(|(label, count)| {
            let pressed = query.source.as_deref() == Some(label);
            Facet {
                href: query.href_with(
                    query.base.as_deref(),
                    query.ptype.as_deref(),
                    if pressed { None } else { Some(label) },
                    1,
                    None,
                ),
                pressed,
                key: source_fluent_key(label).to_string(),
                label: label.to_string(),
                count,
            }
        })
        .collect();
    facets_source.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.label.cmp(&b.label)));

    // Table: filter, then paginate.
    let visible: Vec<&Arc<SearchParameterDefinition>> = in_base
        .iter()
        .filter(|p| match &query.ptype {
            Some(t) => p.param_type.to_string() == *t,
            None => true,
        })
        .filter(|p| match &query.source {
            Some(s) => source_key(p.source) == *s,
            None => true,
        })
        .copied()
        .collect();

    let total = visible.len();
    let page_count = total.div_ceil(PAGE_SIZE).max(1);
    let page = query.page.clamp(1, page_count);
    let rows: Vec<SpRow> = visible
        .iter()
        .skip((page - 1) * PAGE_SIZE)
        .take(PAGE_SIZE)
        .map(|param| {
            let selected = query.sel.as_deref() == Some(param.url.as_str());
            SpRow {
                code: param.code.clone(),
                url: param.url.clone(),
                type_label: param.param_type.to_string(),
                bases: summarize_bases(&param.base),
                expression: snippet(&param.expression, 90),
                source: source_key(param.source),
                source_key: source_fluent_key(source_key(param.source)),
                chips: chips_for(param),
                href: query.href_current(page, Some(param.url.as_str())),
                selected,
            }
        })
        .collect();

    let prev_href = (page > 1).then(|| query.href_current(page - 1, query.sel.as_deref()));
    let next_href =
        (page < page_count).then(|| query.href_current(page + 1, query.sel.as_deref()));
    let pages: Vec<PageLink> = page_links(page, page_count)
        .into_iter()
        .map(|n| PageLink {
            label: n.to_string(),
            href: query.href_current(n, query.sel.as_deref()),
            current: n == page,
        })
        .collect();

    // Detail panel for the selected parameter.
    let detail = query.sel.as_deref().and_then(|sel| {
        params.iter().find(|p| p.url == sel).map(|param| {
            let mut notes: Vec<SpNote> = Vec::new();
            for chip in chips_for(param) {
                let mates: Vec<String> = param
                    .base
                    .iter()
                    .flat_map(|base| slots[&(base.as_str(), param.code.as_str())].iter())
                    .filter(|other| !Arc::ptr_eq(other, param))
                    .map(|other| other.url.clone())
                    .collect();
                notes.push(SpNote {
                    severity: if chip.kind == "conflict" { "error" } else { "info" },
                    kind: match chip.kind {
                        "conflict" => "conflict",
                        "overrides" => "overrides",
                        _ => "shadowed",
                    },
                    arg: mates.join(", "),
                });
            }
            if param.expression.is_empty() {
                notes.push(SpNote {
                    severity: "warn",
                    kind: "empty-expression",
                    arg: String::new(),
                });
            }
            if param.param_type == helios_fhir::search::SearchParamType::Reference
                && param.target.as_ref().is_none_or(|t| t.is_empty())
            {
                notes.push(SpNote {
                    severity: "warn",
                    kind: "no-target",
                    arg: String::new(),
                });
            }
            if param.expression.contains("ofType(")
                || param
                    .expression
                    .split(" as ")
                    .skip(1)
                    .any(|rest| rest.starts_with(|c: char| c.is_ascii_uppercase()))
            {
                notes.push(SpNote {
                    severity: "info",
                    kind: "choice-type",
                    arg: String::new(),
                });
            }
            SpDetail {
                code: param.code.clone(),
                name: param.name.clone().unwrap_or_else(|| param.code.clone()),
                url: param.url.clone(),
                type_label: param.param_type.to_string(),
                bases: param.base.clone(),
                expression: param.expression.clone(),
                description: param.description.clone().unwrap_or_default(),
                targets: param.target.clone().unwrap_or_default(),
                components: param
                    .component
                    .as_ref()
                    .map(|comps| comps.iter().map(|c| c.expression.clone()).collect())
                    .unwrap_or_default(),
                status: param.status.to_fhir_status().to_string(),
                source: source_key(param.source),
                source_key: source_fluent_key(source_key(param.source)),
                notes,
            }
        })
    });

    let versions = enabled_versions()
        .into_iter()
        .map(|v| {
            let mut q = query.clone();
            q.version = Some(v.as_str().to_string());
            VersionLink {
                label: v.as_str(),
                href: q.href_with(
                    query.base.as_deref(),
                    query.ptype.as_deref(),
                    query.source.as_deref(),
                    1,
                    None,
                ),
                current: v == version,
            }
        })
        .collect();

    let mut hidden_fields: Vec<(String, String)> = Vec::new();
    if let Some(v) = &query.version {
        hidden_fields.push(("version".into(), v.clone()));
    }
    if let Some(b) = &query.base {
        hidden_fields.push(("base".into(), b.clone()));
    }
    if let Some(t) = &query.ptype {
        hidden_fields.push(("type".into(), t.clone()));
    }
    if let Some(s) = &query.source {
        hidden_fields.push(("source".into(), s.clone()));
    }

    SpView {
        versions,
        rail_all,
        rail,
        q: query.q.clone(),
        facets_type,
        facets_source,
        rows,
        total,
        page_count,
        prev_href,
        next_href,
        pages,
        detail,
        spec_loaded: snapshot.spec_loaded,
        hidden_fields,
    }
}

/// Source precedence from #242: Embedded < Config < Stored.
fn source_rank(source: SearchParameterSource) -> u8 {
    match source {
        SearchParameterSource::Embedded => 0,
        SearchParameterSource::Config => 1,
        SearchParameterSource::Stored => 2,
    }
}

fn summarize_bases(bases: &[String]) -> String {
    if bases.len() > 3 {
        format!("{} +{}", bases[..3].join(", "), bases.len() - 3)
    } else {
        bases.join(", ")
    }
}

fn snippet(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        let cut: String = text.chars().take(max).collect();
        format!("{cut}…")
    } else {
        text.to_string()
    }
}

/// Compact pagination: first, last, and a window around the current page.
fn page_links(page: usize, page_count: usize) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for n in 1..=page_count {
        if n == 1 || n == page_count || n.abs_diff(page) <= 2 {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> VersionSnapshot {
        // Tests run with the crate as CWD; the workspace spec bundles live
        // two levels up.
        load_snapshot(FhirVersion::default(), Some(Path::new("../../data")))
    }

    fn def(
        url: &str,
        code: &str,
        base: &[&str],
        source: SearchParameterSource,
    ) -> SearchParameterDefinition {
        SearchParameterDefinition::new(
            url,
            code,
            helios_fhir::search::SearchParamType::Token,
            "Resource.id",
        )
        .with_base(base.iter().copied())
        .with_source(source)
    }

    #[test]
    fn snapshot_loads_the_full_spec_bundle() {
        let snapshot = snapshot();
        assert!(snapshot.spec_loaded, "spec bundle found in ../../data");
        assert!(
            snapshot.params.len() > 1000,
            "expected the full registry, got {}",
            snapshot.params.len()
        );
    }

    #[test]
    fn base_filter_narrows_and_facets_count_within_base() {
        let snapshot = snapshot();
        let query = SpQuery {
            base: Some("Patient".into()),
            ..SpQuery::default()
        };
        let view = build_view(&snapshot, &query);

        assert!(view.total < snapshot.params.len());
        assert!(view.total > 10, "Patient supports dozens of parameters");
        // Facet counts are scoped to the base type, so they sum to the total.
        let facet_sum: usize = view.facets_type.iter().map(|f| f.count).sum();
        assert_eq!(facet_sum, view.total);
        // The rail keeps every base type listed with the filter applied.
        assert!(view.rail.iter().any(|item| item.name == "Observation"));
    }

    #[test]
    fn pagination_is_stable_and_complete() {
        let snapshot = snapshot();
        let mut seen = std::collections::HashSet::new();
        let mut page = 1;
        loop {
            let view = build_view(
                &snapshot,
                &SpQuery {
                    page,
                    ..SpQuery::default()
                },
            );
            for row in &view.rows {
                assert!(seen.insert(row.url.clone()), "row repeated: {}", row.url);
            }
            if page >= view.page_count {
                assert_eq!(seen.len(), view.total, "every row appears exactly once");
                break;
            }
            page += 1;
        }
    }

    #[test]
    fn selection_builds_the_detail_panel() {
        let snapshot = snapshot();
        let first = snapshot.params[0].url.clone();
        let view = build_view(
            &snapshot,
            &SpQuery {
                sel: Some(first.clone()),
                ..SpQuery::default()
            },
        );
        let detail = view.detail.expect("selected parameter resolves");
        assert_eq!(detail.url, first);
    }

    #[test]
    fn cross_source_slot_mates_render_override_and_shadow_chips() {
        let spec = def(
            "http://hl7.org/fhir/SearchParameter/Observation-code",
            "code",
            &["Observation"],
            SearchParameterSource::Embedded,
        );
        let stored = def(
            "http://acme.health/fhir/SearchParameter/observation-code-local",
            "code",
            &["Observation"],
            SearchParameterSource::Stored,
        );
        let snapshot = VersionSnapshot {
            params: vec![Arc::new(spec), Arc::new(stored)],
            spec_loaded: true,
        };
        let view = build_view(&snapshot, &SpQuery::default());

        let spec_row = view.rows.iter().find(|r| r.url.contains("hl7.org")).unwrap();
        assert_eq!(spec_row.chips[0].kind, "shadowed");
        let stored_row = view.rows.iter().find(|r| r.url.contains("acme")).unwrap();
        assert_eq!(stored_row.chips[0].kind, "overrides");
    }

    #[test]
    fn same_source_slot_mates_are_a_conflict() {
        let one = def(
            "http://example.org/a",
            "code",
            &["Observation"],
            SearchParameterSource::Embedded,
        );
        let two = def(
            "http://example.org/b",
            "code",
            &["Observation"],
            SearchParameterSource::Embedded,
        );
        let snapshot = VersionSnapshot {
            params: vec![Arc::new(one), Arc::new(two)],
            spec_loaded: true,
        };
        let view = build_view(&snapshot, &SpQuery::default());
        assert!(view.rows.iter().all(|r| r.chips[0].kind == "conflict"));
    }

    #[test]
    fn missing_spec_dir_still_serves_the_fallback() {
        let snapshot =
            load_snapshot(FhirVersion::default(), Some(Path::new("./does-not-exist")));
        assert!(!snapshot.spec_loaded);
        assert!(!snapshot.params.is_empty(), "embedded fallback still loads");
    }
}
