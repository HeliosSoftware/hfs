//! The one CapabilityStatement read model, shared by HFS and HTS (#808).
//!
//! # Why this lives here
//!
//! `/metadata` returns the same document shape whichever Helios server
//! answers it, and both products render it as the same stacked cards. Until
//! #808 each crate carried its own parser, its own view model and its own
//! copy of the markup:
//!
//! | | HFS | HTS |
//! |---|---|---|
//! | Parser | `crates/ui/src/capability.rs::build_view` | `crates/hts-ui/src/upstream.rs::parse_capability_statement` |
//! | View model | `CapabilityView` + four row structs | `CapabilityView` + two row structs |
//! | Template | `crates/ui/templates/pages/capability-statement.html` | `crates/hts-ui/templates/pages/capability-statement.html` |
//!
//! Two structs with the same name, in two crates, projecting the same JSON.
//! The practical cost was that every improvement had to be made twice and
//! never was: #797 (version-correct documentation links) and the interaction
//! colour coding both landed on HFS only, and the HTS page rendered bare
//! `<span class="tag">` chips and no links at all.
//!
//! So the projection lives here once, and both products render it through the
//! card functions below. Same reasoning as [`crate::user_menu`]: a structure
//! that cannot diverge beats a test that notices divergence afterwards.
//!
//! # What this module deliberately does not do
//!
//! * **No fetching.** HFS reads its statement through a loopback self-call;
//!   HTS proxies an upstream and needs per-card isolation semantics. Those
//!   strategies are legitimately different, so both handlers keep their own —
//!   this shows up as [`CapabilityCards::raw`] taking already-fetched text
//!   and a fragment URL rather than a document to fetch itself. The fragment
//!   endpoint each product mounts *behind* that URL lives in
//!   [`crate::capability_json`], which HFS's #798 render budget and HTS's
//!   400 KB+ statement (one extension per loaded code system) both now share.
//! * **No FHIR schema dependency.** Deciding whether a resource type has an
//!   official core page in a given release needs a version-accurate resource
//!   catalog, which HFS has (the validator's core packs) and HTS does not
//!   want to carry. The caller supplies one through [`CoreResourceCatalog`],
//!   so this crate stays a `serde_json` + `url` leaf.

use crate::ChromeLabels;
use askama::Template;
use serde_json::Value;

// ── FHIR release ────────────────────────────────────────────────────────────

/// The FHIR release whose specification the page should link into.
///
/// Deliberately *not* `helios_fhir::FhirVersion`. That enum's variants are
/// `#[cfg]`-gated on the FHIR version features of the crate that built it, so
/// naming it here would drag this leaf crate into the R4/R4B/R5/R6 feature
/// matrix of both products for the sake of four documentation roots. Each
/// consumer maps its own version type onto this one — HFS from a total
/// `match` on `FhirVersion`, HTS from the `&'static str` release code its
/// binary was built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DocsVersion {
    #[default]
    R4,
    R4B,
    R5,
    R6,
}

impl DocsVersion {
    /// Parse the release code Helios uses everywhere (`R4`, `R4B`, `R5`,
    /// `R6`), matching [`helios_fhir::FhirVersion::as_str`] output.
    ///
    /// Case-insensitive, because a configured value may arrive as `r4`.
    /// Returns `None` for anything else rather than guessing a release and
    /// linking the operator at documentation for a spec their server does not
    /// implement.
    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_ascii_uppercase().as_str() {
            "R4" => Some(Self::R4),
            "R4B" => Some(Self::R4B),
            "R5" => Some(Self::R5),
            "R6" => Some(Self::R6),
            _ => None,
        }
    }

    /// The published documentation root for this release, trailing slash
    /// included. R6 is still balloting, so it is pinned to the ballot the
    /// generated models track rather than to a `6.0.0` path that 404s.
    pub fn docs_root(self) -> &'static str {
        match self {
            Self::R4 => "https://hl7.org/fhir/R4/",
            Self::R4B => "https://hl7.org/fhir/R4B/",
            Self::R5 => "https://hl7.org/fhir/R5/",
            Self::R6 => "https://hl7.org/fhir/6.0.0-ballot4/",
        }
    }
}

// ── Core resource catalog ───────────────────────────────────────────────────

/// Whether a resource type has an official core page in the release being
/// rendered.
///
/// This is the one question the projection cannot answer on its own, and
/// getting it wrong is user-visible: `SubscriptionTopic` exists in R4B but
/// not R4, so an R4 page that linked `subscriptiontopic.html` would send the
/// operator to a 404. HFS answers it from the validator's core schema packs;
/// HTS answers it from the fixed set of resource types a terminology server
/// can advertise. Neither answer belongs in this crate.
///
/// A type the catalog rejects is not linked to the specification at all — the
/// row falls back to the advertised profile, or to no link. Under-linking is
/// the safe direction.
pub trait CoreResourceCatalog {
    /// True when `resource_type` is a resource (not a datatype, not an
    /// unknown name) in the release this catalog was built for.
    fn is_core_resource(&self, resource_type: &str) -> bool;
}

/// A catalog that claims nothing, so no row is ever linked to the FHIR
/// specification. Useful for tests and for a caller that has no resource
/// catalog to hand; a real page should supply one.
pub struct NoCoreResources;

impl CoreResourceCatalog for NoCoreResources {
    fn is_core_resource(&self, _resource_type: &str) -> bool {
        false
    }
}

impl<F: Fn(&str) -> bool> CoreResourceCatalog for F {
    fn is_core_resource(&self, resource_type: &str) -> bool {
        self(resource_type)
    }
}

// ── View model ──────────────────────────────────────────────────────────────

/// The server-summary block: identity fields lifted off the statement root.
#[derive(Clone, Debug, Default)]
pub struct CapabilitySummary {
    /// `implementation.description`.
    pub description: String,
    /// `implementation.url` — the server's own base URL, which is what the
    /// `cap-summary-url` label ("Base URL") promises. Note this is *not* the
    /// statement's own `url` canonical, which identifies the document rather
    /// than the endpoint.
    pub implementation_url: String,
    pub fhir_version: String,
    pub status: String,
    pub kind: String,
    pub date: String,
    pub formats: Vec<String>,
}

/// One system-level interaction (`rest[].interaction[]`).
#[derive(Clone, Debug, Default)]
pub struct SystemInteraction {
    pub code: String,
    /// Deep link into the release's HTTP specification. Empty when the verb
    /// has no page of its own.
    pub href: String,
    pub tag_class: &'static str,
}

/// One server operation (`rest[].operation[]`).
#[derive(Clone, Debug, Default)]
pub struct OperationRow {
    pub name: String,
    /// An absolute HTTP(S) canonical, without its optional `|version`
    /// suffix. Empty when the advertised definition is not safe to navigate
    /// to.
    pub definition_href: String,
    pub definition: String,
}

/// An interaction code and the semantic tag style used across UI tables.
#[derive(Clone, Debug, Default)]
pub struct InteractionTag {
    pub code: String,
    pub tag_class: &'static str,
}

/// One per-resource row (`rest[].resource[]`).
#[derive(Clone, Debug, Default)]
pub struct ResourceRow {
    pub resource_type: String,
    /// A safe advertised profile, or the release's official core resource
    /// page. Empty only when neither is safe to construct.
    pub resource_href: String,
    pub interactions: Vec<InteractionTag>,
    pub search_param_count: usize,
    pub include_count: usize,
    pub revinclude_count: usize,
}

/// Everything the shared cards render.
#[derive(Clone, Debug, Default)]
pub struct CapabilityView {
    pub summary: CapabilitySummary,
    pub interactions: Vec<SystemInteraction>,
    pub operations: Vec<OperationRow>,
    pub resources: Vec<ResourceRow>,
    /// The explanatory backend-role note is rendered once, even if malformed
    /// metadata advertises the transaction interaction more than once.
    pub has_conditional_transaction: bool,
}

// ── Projection ──────────────────────────────────────────────────────────────

fn str_at<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    let mut cur = value;
    for p in path {
        match cur.get(p) {
            Some(v) => cur = v,
            None => return "",
        }
    }
    cur.as_str().unwrap_or("")
}

fn arr<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Every `rest[]` entry this page speaks for, flattened in document order.
///
/// The two crates disagreed here: HFS read `rest[0]` and HTS flattened every
/// entry. Flattening is the correct reading — `rest` is 0..* and a server may
/// legitimately publish more than one component — but it must not sweep in a
/// `mode: "client"` block, which describes what the server *calls*, not what
/// it *serves*. So: server components, plus components that state no mode at
/// all, which is what a hand-written statement or a test fixture usually
/// looks like.
fn server_rest(statement: &Value) -> Vec<&Value> {
    arr(statement, "rest")
        .iter()
        .filter(|rest| matches!(str_at(rest, &["mode"]), "server" | ""))
        .collect()
}

/// Projects the raw CapabilityStatement into the shared view.
///
/// Defensive over the JSON: absent fields render empty, never panic — the
/// statement shape varies with the server, its enabled features and its FHIR
/// release.
pub fn build_view(
    statement: &Value,
    version: DocsVersion,
    catalog: &dyn CoreResourceCatalog,
) -> CapabilityView {
    let rest = server_rest(statement);
    let flattened = |key: &'static str| -> Vec<&Value> {
        rest.iter().flat_map(|r| arr(r, key).iter()).collect()
    };

    let summary = CapabilitySummary {
        description: str_at(statement, &["implementation", "description"]).to_string(),
        implementation_url: str_at(statement, &["implementation", "url"]).to_string(),
        fhir_version: str_at(statement, &["fhirVersion"]).to_string(),
        status: str_at(statement, &["status"]).to_string(),
        kind: str_at(statement, &["kind"]).to_string(),
        date: str_at(statement, &["date"]).to_string(),
        formats: arr(statement, "format")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    };

    let interactions: Vec<SystemInteraction> = flattened("interaction")
        .iter()
        .map(|i| {
            let code = str_at(i, &["code"]).to_string();
            SystemInteraction {
                href: system_interaction_href(version, &code).unwrap_or_default(),
                tag_class: interaction_tag_class(&code),
                code,
            }
        })
        .collect();
    let has_conditional_transaction = interactions.iter().any(|i| i.code == "transaction");

    let operations = flattened("operation")
        .iter()
        .map(|o| {
            let definition = str_at(o, &["definition"]).to_string();
            OperationRow {
                name: str_at(o, &["name"]).to_string(),
                definition_href: safe_canonical_href(&definition).unwrap_or_default(),
                definition,
            }
        })
        .collect();

    let resources = flattened("resource")
        .iter()
        .map(|r| {
            let resource_type = str_at(r, &["type"]).to_string();
            let advertised_profile = str_at(r, &["profile"]);
            let is_core_resource = catalog.is_core_resource(&resource_type);
            let safe_profile = safe_canonical_href(advertised_profile);
            // The specification page wins when the type is a core resource
            // and the server advertises either no profile or the core
            // profile for that very type. A real constraint profile is the
            // more specific answer and is kept.
            let resource_href = if is_core_resource
                && (safe_profile.is_none()
                    || is_core_resource_canonical(advertised_profile, &resource_type))
            {
                resource_definition_href(version, &resource_type)
            } else {
                safe_profile.unwrap_or_default()
            };
            ResourceRow {
                resource_type,
                resource_href,
                interactions: arr(r, "interaction")
                    .iter()
                    .map(|i| {
                        let code = str_at(i, &["code"]).to_string();
                        InteractionTag {
                            tag_class: interaction_tag_class(&code),
                            code,
                        }
                    })
                    .collect(),
                search_param_count: arr(r, "searchParam").len(),
                include_count: arr(r, "searchInclude").len(),
                revinclude_count: arr(r, "searchRevInclude").len(),
            }
        })
        .collect();

    CapabilityView {
        summary,
        interactions,
        operations,
        resources,
        has_conditional_transaction,
    }
}

/// Deep link for a system-level interaction verb, or `None` when the release
/// has no page anchored on it.
pub fn system_interaction_href(version: DocsVersion, code: &str) -> Option<String> {
    let fragment = match code {
        "transaction" => "http.html#transaction",
        "batch" => "http.html#batch",
        "search-system" => "http.html#search",
        "history-system" => "http.html#history",
        _ => return None,
    };
    Some(format!("{}{fragment}", version.docs_root()))
}

/// The release's official page for a core resource type.
///
/// Callers validate the type through a [`CoreResourceCatalog`] before
/// deriving a path. Core resource pages use lowercase ASCII slugs.
pub fn resource_definition_href(version: DocsVersion, resource_type: &str) -> String {
    format!(
        "{}{resource_slug}.html",
        version.docs_root(),
        resource_slug = resource_type.to_ascii_lowercase()
    )
}

/// Whether an advertised profile is the exact HL7 core canonical for the
/// advertised resource type. A FHIR canonical may carry one `|version`
/// qualifier, which does not change the profile identity.
fn is_core_resource_canonical(canonical: &str, resource_type: &str) -> bool {
    let (base, version) = canonical
        .split_once('|')
        .map_or((canonical, None), |(base, version)| (base, Some(version)));
    if version.is_some_and(|version| {
        version.is_empty()
            || version.contains('|')
            || version.contains('?')
            || version.contains('#')
    }) {
        return false;
    }

    base == format!("http://hl7.org/fhir/StructureDefinition/{resource_type}")
        || base == format!("https://hl7.org/fhir/StructureDefinition/{resource_type}")
}

/// Returns an absolute HTTP(S) canonical stripped of its optional FHIR
/// `|version` qualifier, or `None` when it is not something safe to put in an
/// `href`. Everything else stays visible as plain text.
pub fn safe_canonical_href(canonical: &str) -> Option<String> {
    let fragment_start = canonical.find('#').unwrap_or(canonical.len());
    let before_fragment = &canonical[..fragment_start];
    let fragment = &canonical[fragment_start..];
    let base = before_fragment
        .split_once('|')
        .map_or(before_fragment, |(url, _)| url);
    let raw = format!("{base}{fragment}");
    let parsed = url::Url::parse(&raw).ok()?;
    matches!(parsed.scheme(), "http" | "https")
        .then(|| parsed.has_host())
        .filter(|has_host| *has_host)
        .map(|_| raw)
}

/// The semantic tag style for an interaction verb, shared by the system and
/// per-resource chips so the same word never reads two colours on two pages.
pub fn interaction_tag_class(code: &str) -> &'static str {
    match code {
        "read" | "vread" | "search-type" | "search-system" | "history-instance"
        | "history-type" | "history-system" => "tag--member",
        "create" | "update" | "patch" | "batch" | "transaction" => "tag--config",
        "delete" => "tag--excluded",
        _ => "tag--muted",
    }
}

// ── Cards ───────────────────────────────────────────────────────────────────

/// The progressively enhanced resource-type filter HFS renders in the
/// per-resource card head.
///
/// HTS advertises three resource types, so it passes `None` — a search box
/// over three rows is noise, not parity.
#[derive(Clone, Copy, Debug)]
pub struct ResourceFilter<'a> {
    /// Where the plain GET form submits, e.g. `/ui/capability-statement`.
    /// Also the htmx `hx-get` target, so the enhanced and unenhanced paths
    /// cannot drift.
    pub action: &'a str,
    /// FHIR release code carried through the form so narrowing the table
    /// does not silently reset the page's version.
    pub version: &'a str,
    /// The current filter, echoed back into the input.
    pub value: &'a str,
}

/// Renderer for the four cards HFS and HTS have in common.
///
/// Construct with [`CapabilityCards::new`] and adjust the handful of fields
/// where the two pages genuinely differ. Each `card` method returns one
/// complete `<section class="card">`, ready to splice into a page template
/// with `|safe` — every value reaching the output has been through Askama's
/// HTML escaper first.
///
/// The caller decides which cards to render and in what order, which is how
/// HTS slots its terminology card between the resource table and the raw
/// fold, and how it omits the system-interactions card entirely while its
/// server declares none.
pub struct CapabilityCards<'a> {
    i18n: &'a dyn ChromeLabels,
    view: &'a CapabilityView,
    notice: Option<&'a str>,
    transaction_note_href: Option<&'a str>,
    operations_empty_key: Option<&'a str>,
    resources_empty_key: &'a str,
    show_include_columns: bool,
    filter: Option<ResourceFilter<'a>>,
}

impl<'a> CapabilityCards<'a> {
    /// The plainest set of cards: no notice, no transaction note, no filter,
    /// no include columns, and the shared `cap-resources-empty` string for an
    /// empty resource table.
    pub fn new(i18n: &'a dyn ChromeLabels, view: &'a CapabilityView) -> Self {
        Self {
            i18n,
            view,
            notice: None,
            transaction_note_href: None,
            operations_empty_key: None,
            resources_empty_key: "cap-resources-empty",
            show_include_columns: false,
            filter: None,
        }
    }

    /// Replace every card *body* with this already-translated warning
    /// sentence, keeping the heading.
    ///
    /// This is what makes HTS's per-card degradation possible without a
    /// second copy of the card headings: one upstream fetch fails, its cards
    /// render their own `notice--warn`, and the cards fed by the other fetch
    /// are untouched.
    pub fn notice(mut self, notice: Option<&'a str>) -> Self {
        self.notice = notice;
        self
    }

    /// Render the backend-role footnote under the system interactions when
    /// the server advertises `transaction`, linking to `href` for the detail.
    ///
    /// HFS passes its persistence role matrix. A server whose transaction
    /// support is not backend-conditional passes `None` and gets no note.
    pub fn transaction_note_href(mut self, href: Option<&'a str>) -> Self {
        self.transaction_note_href = href;
        self
    }

    /// Fluent key for the empty-state row in the operations table. `None`
    /// renders no row at all, which is HFS's behaviour — its server always
    /// advertises operations.
    pub fn operations_empty_key(mut self, key: Option<&'a str>) -> Self {
        self.operations_empty_key = key;
        self
    }

    /// Fluent key for the empty-state row in the per-resource table.
    pub fn resources_empty_key(mut self, key: &'a str) -> Self {
        self.resources_empty_key = key;
        self
    }

    /// Show the `searchInclude` / `searchRevInclude` count columns.
    ///
    /// Off by default: a server that emits neither would render two columns
    /// of zeroes, which reads as a measurement rather than an absence.
    pub fn show_include_columns(mut self, show: bool) -> Self {
        self.show_include_columns = show;
        self
    }

    /// Attach the progressively enhanced resource-type filter.
    pub fn filter(mut self, filter: Option<ResourceFilter<'a>>) -> Self {
        self.filter = filter;
        self
    }

    /// Server Summary — identity fields lifted off the statement root.
    pub fn summary(&self) -> Result<String, askama::Error> {
        SummaryCardTemplate {
            i18n: self.i18n,
            notice: self.notice,
            summary: &self.view.summary,
        }
        .render()
    }

    /// System Interactions — the `rest[].interaction[]` verbs as linked,
    /// colour-coded chips.
    pub fn interactions(&self) -> Result<String, askama::Error> {
        InteractionsCardTemplate {
            i18n: self.i18n,
            notice: self.notice,
            interactions: &self.view.interactions,
            transaction_note_href: self
                .transaction_note_href
                .filter(|_| self.view.has_conditional_transaction),
        }
        .render()
    }

    /// Operations — the system-level `rest[].operation[]` table.
    pub fn operations(&self) -> Result<String, askama::Error> {
        OperationsCardTemplate {
            i18n: self.i18n,
            notice: self.notice,
            operations: &self.view.operations,
            empty_key: self.operations_empty_key,
        }
        .render()
    }

    /// Per-Resource Capabilities — the `rest[].resource[]` table.
    pub fn resources(&self) -> Result<String, askama::Error> {
        ResourcesCardTemplate {
            i18n: self.i18n,
            notice: self.notice,
            resources: &self.view.resources,
            empty_key: self.resources_empty_key,
            show_include_columns: self.show_include_columns,
            // `colspan` has to cover whichever column set is rendered, or the
            // empty row stops spanning the table.
            empty_colspan: if self.show_include_columns { 5 } else { 3 },
            // A degraded card has no table to narrow, so it gets no filter
            // either — an input that cannot affect anything is worse than
            // no input.
            filter: self.filter.filter(|_| self.notice.is_none()),
        }
        .render()
    }

    /// Raw CapabilityStatement — the foldable, htmx-lazy JSON block shared by
    /// both products (#808 follow-up to #798). Neither the fetch nor the
    /// fragment endpoint lives here — see [`crate::capability_json`] — this
    /// card is only the shell: a "Load JSON" link against `fragment_url` by
    /// default, or the fully resolved `raw` text when `raw_requested` (the
    /// no-JS fallback, driven by `raw_url`).
    ///
    /// Unlike the other four cards this one ignores `self.notice`: a failed
    /// fetch means the caller has no statement to fold at all, and skips
    /// calling this method entirely rather than rendering it degraded.
    pub fn raw(
        &self,
        raw_requested: bool,
        raw: &str,
        raw_url: &str,
        fragment_url: &str,
    ) -> Result<String, askama::Error> {
        RawCardTemplate {
            i18n: self.i18n,
            raw_requested,
            raw,
            raw_url,
            fragment_url,
        }
        .render()
    }
}

#[derive(Template)]
#[template(path = "partials/capability-raw-card.html")]
struct RawCardTemplate<'a> {
    i18n: &'a dyn ChromeLabels,
    raw_requested: bool,
    raw: &'a str,
    raw_url: &'a str,
    fragment_url: &'a str,
}

#[derive(Template)]
#[template(path = "partials/capability-summary-card.html")]
struct SummaryCardTemplate<'a> {
    i18n: &'a dyn ChromeLabels,
    notice: Option<&'a str>,
    summary: &'a CapabilitySummary,
}

#[derive(Template)]
#[template(path = "partials/capability-interactions-card.html")]
struct InteractionsCardTemplate<'a> {
    i18n: &'a dyn ChromeLabels,
    notice: Option<&'a str>,
    interactions: &'a [SystemInteraction],
    transaction_note_href: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "partials/capability-operations-card.html")]
struct OperationsCardTemplate<'a> {
    i18n: &'a dyn ChromeLabels,
    notice: Option<&'a str>,
    operations: &'a [OperationRow],
    empty_key: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "partials/capability-resources-card.html")]
struct ResourcesCardTemplate<'a> {
    i18n: &'a dyn ChromeLabels,
    notice: Option<&'a str>,
    resources: &'a [ResourceRow],
    empty_key: &'a str,
    show_include_columns: bool,
    empty_colspan: u8,
    filter: Option<ResourceFilter<'a>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Stands in for a release's resource catalog. HFS passes the validator's
    /// core packs and HTS the three types a terminology server serves; the
    /// tests here only need the question answered, not answered accurately.
    fn core(types: &'static [&'static str]) -> impl CoreResourceCatalog {
        move |resource_type: &str| types.contains(&resource_type)
    }

    /// Project a fixture against an R4 catalog holding the three types the
    /// fixtures below use.
    fn project(statement: &Value) -> CapabilityView {
        build_view(
            statement,
            DocsVersion::R4,
            &core(&["Patient", "Observation", "Encounter"]),
        )
    }

    #[test]
    fn projects_the_statement_defensively() {
        let statement = json!({
            "resourceType": "CapabilityStatement",
            "status": "active", "kind": "instance", "date": "2026-08-24",
            "fhirVersion": "4.0.1",
            "format": ["application/fhir+json"],
            "implementation": {"description": "Helios FHIR Server", "url": "http://x/"},
            "rest": [{
                "mode": "server",
                "interaction": [{"code": "batch"}, {"code": "transaction"}],
                "operation": [
                    {"name": "export", "definition": "http://x/OperationDefinition/export"},
                    {"name": "viewdef", "definition": "http://sql-on-fhir.org/OperationDefinition/run"}
                ],
                "resource": [{
                    "type": "Patient",
                    "interaction": [{"code": "read"}, {"code": "search-type"}],
                    "searchParam": [{"name": "name"}, {"name": "birthdate"}],
                    "searchInclude": ["Patient:organization"]
                }]
            }]
        });
        let view = project(&statement);
        assert_eq!(view.summary.fhir_version, "4.0.1");
        assert_eq!(view.summary.implementation_url, "http://x/");
        assert_eq!(view.summary.formats.len(), 1);
        assert!(
            view.interactions
                .iter()
                .any(|i| i.code == "transaction" && i.tag_class == "tag--config")
        );
        assert!(
            view.interactions
                .iter()
                .any(|i| i.code == "batch" && i.tag_class == "tag--config")
        );
        assert_eq!(
            view.operations[0].definition_href,
            "http://x/OperationDefinition/export"
        );
        assert_eq!(
            view.operations[1].definition_href,
            "http://sql-on-fhir.org/OperationDefinition/run"
        );
        assert!(view.has_conditional_transaction);
        assert_eq!(
            view.resources[0].resource_href,
            "https://hl7.org/fhir/R4/patient.html"
        );
        assert_eq!(view.resources[0].search_param_count, 2);
        assert_eq!(view.resources[0].include_count, 1);
        assert_eq!(view.resources[0].revinclude_count, 0);

        // An empty statement renders empty, never panics.
        let empty = project(&json!({}));
        assert!(empty.resources.is_empty());
        assert!(empty.summary.fhir_version.is_empty());
    }

    #[test]
    fn documentation_roots_are_release_aware() {
        assert_eq!(DocsVersion::R4.docs_root(), "https://hl7.org/fhir/R4/");
        assert_eq!(DocsVersion::R4B.docs_root(), "https://hl7.org/fhir/R4B/");
        assert_eq!(DocsVersion::R5.docs_root(), "https://hl7.org/fhir/R5/");
        assert_eq!(
            DocsVersion::R6.docs_root(),
            "https://hl7.org/fhir/6.0.0-ballot4/"
        );
    }

    #[test]
    fn release_codes_round_trip_and_reject_the_unknown() {
        for (code, expected) in [
            ("R4", DocsVersion::R4),
            ("r4b", DocsVersion::R4B),
            ("R5", DocsVersion::R5),
            ("R6", DocsVersion::R6),
        ] {
            assert_eq!(DocsVersion::from_code(code), Some(expected), "{code}");
        }
        for code in ["", "R7", "4.0.1", "STU3"] {
            assert_eq!(DocsVersion::from_code(code), None, "{code}");
        }
    }

    #[test]
    fn core_resource_pages_follow_each_release_root() {
        for (version, expected) in [
            (DocsVersion::R4, "https://hl7.org/fhir/R4/patient.html"),
            (DocsVersion::R4B, "https://hl7.org/fhir/R4B/patient.html"),
            (DocsVersion::R5, "https://hl7.org/fhir/R5/patient.html"),
            (
                DocsVersion::R6,
                "https://hl7.org/fhir/6.0.0-ballot4/patient.html",
            ),
        ] {
            let statement = json!({"rest": [{"resource": [{
                "type": "Patient",
                "profile": "http://hl7.org/fhir/StructureDefinition/Patient"
            }]}]});
            let view = build_view(&statement, version, &core(&["Patient"]));
            assert_eq!(view.resources[0].resource_href, expected);
        }
    }

    #[test]
    fn a_type_outside_the_catalog_is_never_linked_to_the_spec() {
        let statement = json!({"rest": [{"resource": [
            {"type": "SubscriptionTopic"},
            {"type": "Patient"}
        ]}]});
        let view = build_view(&statement, DocsVersion::R4, &core(&["Patient"]));
        assert!(
            view.resources[0].resource_href.is_empty(),
            "a type the release does not define must not be linked"
        );
        assert_eq!(
            view.resources[1].resource_href,
            "https://hl7.org/fhir/R4/patient.html"
        );
    }

    #[test]
    fn system_interactions_have_exact_links_and_semantic_classes() {
        let statement = json!({"rest": [{"interaction": [
            {"code": "transaction"}, {"code": "batch"},
            {"code": "search-system"}, {"code": "history-system"},
            {"code": "read"}, {"code": "delete"}, {"code": "future-code"}
        ]}]});
        let view = project(&statement);

        let link = |code: &str| {
            view.interactions
                .iter()
                .find(|interaction| interaction.code == code)
                .map(|interaction| interaction.href.as_str())
                .unwrap()
        };
        assert_eq!(
            link("transaction"),
            "https://hl7.org/fhir/R4/http.html#transaction"
        );
        assert_eq!(link("batch"), "https://hl7.org/fhir/R4/http.html#batch");
        assert_eq!(
            link("search-system"),
            "https://hl7.org/fhir/R4/http.html#search"
        );
        assert_eq!(
            link("history-system"),
            "https://hl7.org/fhir/R4/http.html#history"
        );
        assert_eq!(link("read"), "");
        assert_eq!(link("future-code"), "");

        let class = |code: &str| {
            view.interactions
                .iter()
                .find(|interaction| interaction.code == code)
                .map(|interaction| interaction.tag_class)
                .unwrap()
        };
        assert_eq!(class("transaction"), "tag--config");
        assert_eq!(class("read"), "tag--member");
        assert_eq!(class("delete"), "tag--excluded");
        assert_eq!(class("future-code"), "tag--muted");
    }

    #[test]
    fn system_interaction_links_follow_each_release_root() {
        for (version, root) in [
            (DocsVersion::R4, "https://hl7.org/fhir/R4/"),
            (DocsVersion::R4B, "https://hl7.org/fhir/R4B/"),
            (DocsVersion::R5, "https://hl7.org/fhir/R5/"),
            (DocsVersion::R6, "https://hl7.org/fhir/6.0.0-ballot4/"),
        ] {
            for (code, fragment) in [
                ("transaction", "http.html#transaction"),
                ("batch", "http.html#batch"),
                ("search-system", "http.html#search"),
                ("history-system", "http.html#history"),
            ] {
                assert_eq!(
                    system_interaction_href(version, code),
                    Some(format!("{root}{fragment}"))
                );
            }
            assert_eq!(system_interaction_href(version, "unknown"), None);
        }
    }

    #[test]
    fn interaction_codes_use_the_semantic_tag_palette() {
        for code in [
            "read",
            "vread",
            "search-type",
            "search-system",
            "history-instance",
            "history-type",
            "history-system",
        ] {
            assert_eq!(interaction_tag_class(code), "tag--member", "{code}");
        }
        for code in ["create", "update", "patch", "batch", "transaction"] {
            assert_eq!(interaction_tag_class(code), "tag--config", "{code}");
        }
        assert_eq!(interaction_tag_class("delete"), "tag--excluded");
        assert_eq!(interaction_tag_class("future-code"), "tag--muted");
    }

    #[test]
    fn canonicals_are_safe_and_strip_the_fhir_version_qualifier() {
        assert_eq!(
            safe_canonical_href("https://example.org/OperationDefinition/export|1.2.3"),
            Some("https://example.org/OperationDefinition/export".to_string())
        );
        assert_eq!(
            safe_canonical_href("http://example.org/StructureDefinition/Patient"),
            Some("http://example.org/StructureDefinition/Patient".to_string())
        );
        assert_eq!(
            safe_canonical_href("https://example.org/Profile|1.0#details"),
            Some("https://example.org/Profile#details".to_string())
        );
        assert_eq!(
            safe_canonical_href("https://example.org/Profile#details|part"),
            Some("https://example.org/Profile#details|part".to_string())
        );
        for unsafe_value in [
            "OperationDefinition/export",
            "/OperationDefinition/export",
            "javascript:alert(1)",
            "urn:oid:1.2.3",
            "https://",
            "not a URL",
        ] {
            assert_eq!(safe_canonical_href(unsafe_value), None, "{unsafe_value}");
        }
    }

    #[test]
    fn core_profiles_are_versioned_while_custom_profiles_are_preserved() {
        let statement = json!({"rest": [{"resource": [
            {"type": "Patient", "profile": "http://hl7.org/fhir/StructureDefinition/Patient"},
            {"type": "Patient", "profile": "https://hl7.org/fhir/StructureDefinition/Patient|4.0.1"},
            {"type": "Patient", "profile": "https://example.org/Patient|2.0"},
            {"type": "Observation", "profile": "javascript:alert(1)"},
            {"type": "Encounter"},
            {"type": "NotARealResource", "profile": "https://example.org/custom"},
            {"type": "UnknownUnsafe", "profile": "javascript:alert(2)"},
            {"type": "UnknownMissing"},
            {"type": "<script>"}
        ]}]});
        let view = project(&statement);

        assert_eq!(
            view.resources[0].resource_href,
            "https://hl7.org/fhir/R4/patient.html"
        );
        assert_eq!(
            view.resources[1].resource_href,
            "https://hl7.org/fhir/R4/patient.html"
        );
        assert_eq!(
            view.resources[2].resource_href,
            "https://example.org/Patient"
        );
        assert_eq!(
            view.resources[3].resource_href,
            "https://hl7.org/fhir/R4/observation.html"
        );
        assert_eq!(
            view.resources[4].resource_href,
            "https://hl7.org/fhir/R4/encounter.html"
        );
        assert_eq!(
            view.resources[5].resource_href,
            "https://example.org/custom"
        );
        assert!(view.resources[6].resource_href.is_empty());
        assert!(view.resources[7].resource_href.is_empty());
        assert!(view.resources[8].resource_href.is_empty());
    }

    #[test]
    fn core_profile_near_misses_are_not_rewritten() {
        let profiles = [
            "https://hl7.org.example.com/fhir/StructureDefinition/Patient",
            "https://hl7.org/fhir/StructureDefinition/Observation",
            "https://hl7.org/fhir/StructureDefinition/Patient?mode=custom",
            "https://hl7.org/fhir/StructureDefinition/Patient#custom",
            "https://hl7.org/fhir/StructureDefinition/Patient|4.0.1#custom",
            "https://hl7.org/fhir/StructureDefinition/Patient|4.0.1?mode=custom",
            "https://hl7.org/fhir/StructureDefinition/Patient|4.0.1|extra",
        ];
        let statement = json!({"rest": [{"resource": profiles.map(|profile| json!({
            "type": "Patient",
            "profile": profile
        }))}]});
        let view = project(&statement);

        for (row, profile) in view.resources.iter().zip(profiles) {
            assert_eq!(
                row.resource_href,
                safe_canonical_href(profile).unwrap(),
                "{profile}"
            );
        }
    }

    #[test]
    fn duplicate_transactions_produce_one_note_state() {
        let statement = json!({"rest": [{"interaction": [
            {"code": "transaction"}, {"code": "transaction"}
        ]}]});
        assert!(project(&statement).has_conditional_transaction);
    }

    #[test]
    fn every_server_rest_component_contributes_and_client_components_do_not() {
        let statement = json!({"rest": [
            {"mode": "client", "interaction": [{"code": "transaction"}],
             "operation": [{"name": "client-only"}],
             "resource": [{"type": "Observation"}]},
            {"mode": "server", "interaction": [{"code": "batch"}],
             "operation": [{"name": "export"}],
             "resource": [{"type": "Patient"}]},
            {"interaction": [{"code": "search-system"}],
             "operation": [{"name": "unmoded"}],
             "resource": [{"type": "Encounter"}]}
        ]});
        let view = project(&statement);

        let codes: Vec<&str> = view.interactions.iter().map(|i| i.code.as_str()).collect();
        assert_eq!(codes, ["batch", "search-system"]);
        assert!(!view.has_conditional_transaction);
        let names: Vec<&str> = view.operations.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["export", "unmoded"]);
        let types: Vec<&str> = view
            .resources
            .iter()
            .map(|r| r.resource_type.as_str())
            .collect();
        assert_eq!(types, ["Patient", "Encounter"]);
    }

    // ── Cards ───────────────────────────────────────────────────────────────

    struct Labels;

    impl ChromeLabels for Labels {
        fn lang(&self) -> String {
            "en".to_string()
        }
        fn t(&self, key: &str) -> String {
            key.to_string()
        }
    }

    fn sample_view() -> CapabilityView {
        let statement = json!({
            "implementation": {"description": "Helios", "url": "http://x/"},
            "fhirVersion": "4.0.1", "status": "active", "kind": "instance",
            "date": "2026-08-24", "format": ["application/fhir+json"],
            "rest": [{
                "mode": "server",
                "interaction": [{"code": "transaction"}, {"code": "delete"}],
                "operation": [{"name": "export", "definition": "http://x/OperationDefinition/export"}],
                "resource": [{
                    "type": "Patient",
                    "interaction": [{"code": "read"}],
                    "searchParam": [{"name": "name"}],
                    "searchInclude": ["Patient:organization"],
                    "searchRevInclude": ["Observation:subject"]
                }]
            }]
        });
        project(&statement)
    }

    #[test]
    fn the_summary_card_keeps_the_shared_metadata_grid_contract() {
        let view = sample_view();
        let html = CapabilityCards::new(&Labels, &view).summary().unwrap();
        // The design-system suite asserts a card body holding one `.kv-grid`
        // of exactly seven `.detail__field` children; the grid's responsive
        // column rules key off that shape.
        assert!(html.contains(r#"<div class="card__body">"#));
        assert!(html.contains(r#"<div class="kv-grid kv-grid--flush">"#));
        assert_eq!(html.matches(r#"class="detail__field"#).count(), 7);
        assert_eq!(html.matches("detail__field--wide").count(), 2);
        assert!(html.contains("Helios"));
        assert!(html.contains("http://x/"));
    }

    #[test]
    fn absent_summary_fields_render_an_em_dash_not_a_blank() {
        let view = CapabilityView::default();
        let html = CapabilityCards::new(&Labels, &view).summary().unwrap();
        assert_eq!(html.matches("&mdash;").count(), 7);
    }

    #[test]
    fn interaction_chips_are_linked_and_colour_coded() {
        let view = sample_view();
        let html = CapabilityCards::new(&Labels, &view).interactions().unwrap();
        assert!(html.contains(
            r#"<a class="tag tag--config" href="https://hl7.org/fhir/R4/http.html#transaction""#
        ));
        assert!(html.contains(r#"<span class="tag tag--excluded">delete</span>"#));
    }

    #[test]
    fn the_transaction_note_is_opt_in() {
        let view = sample_view();
        let plain = CapabilityCards::new(&Labels, &view).interactions().unwrap();
        assert!(!plain.contains("cap-transaction-note"));

        let noted = CapabilityCards::new(&Labels, &view)
            .transaction_note_href(Some("https://example.test/roles"))
            .interactions()
            .unwrap();
        assert!(noted.contains(r#"<p class="cap-transaction-note">"#));
        assert!(noted.contains(r#"href="https://example.test/roles""#));

        // No `transaction` verb, no note — even when a href is supplied.
        let without = build_view(
            &json!({"rest": [{"interaction": [{"code": "batch"}]}]}),
            DocsVersion::R4,
            &NoCoreResources,
        );
        let html = CapabilityCards::new(&Labels, &without)
            .transaction_note_href(Some("https://example.test/roles"))
            .interactions()
            .unwrap();
        assert!(!html.contains("cap-transaction-note"));
    }

    #[test]
    fn operation_definitions_link_when_the_canonical_is_safe() {
        let view = sample_view();
        let html = CapabilityCards::new(&Labels, &view).operations().unwrap();
        assert!(html.contains(r#"<td>$export</td>"#));
        assert!(html.contains(
            r#"<a class="url" href="http://x/OperationDefinition/export" target="_blank" rel="noopener">"#
        ));

        let unsafe_definition = build_view(
            &json!({"rest": [{"operation": [{"name": "x", "definition": "javascript:alert(1)"}]}]}),
            DocsVersion::R4,
            &NoCoreResources,
        );
        let html = CapabilityCards::new(&Labels, &unsafe_definition)
            .operations()
            .unwrap();
        assert!(!html.contains("javascript:alert(1)</a>"));
        assert!(html.contains(r#"<span class="url">javascript:alert(1)</span>"#));
    }

    #[test]
    fn the_operations_empty_row_is_opt_in() {
        let view = CapabilityView::default();
        assert!(
            !CapabilityCards::new(&Labels, &view)
                .operations()
                .unwrap()
                .contains("data-table__empty")
        );
        assert!(
            CapabilityCards::new(&Labels, &view)
                .operations_empty_key(Some("no-operations"))
                .operations()
                .unwrap()
                .contains(
                    r#"<tr class="data-table__empty"><td colspan="2">no-operations</td></tr>"#
                )
        );
    }

    #[test]
    fn the_resource_table_columns_and_empty_span_move_together() {
        let view = CapabilityView::default();
        let narrow = CapabilityCards::new(&Labels, &view).resources().unwrap();
        assert!(!narrow.contains("cap-col-includes"));
        assert!(narrow.contains(r#"<td colspan="3">"#));

        let wide = CapabilityCards::new(&Labels, &view)
            .show_include_columns(true)
            .resources()
            .unwrap();
        assert!(wide.contains("cap-col-includes") && wide.contains("cap-col-revincludes"));
        assert!(wide.contains(r#"<td colspan="5">"#));
    }

    #[test]
    fn resource_rows_link_the_type_and_colour_code_its_verbs() {
        let view = sample_view();
        let html = CapabilityCards::new(&Labels, &view)
            .show_include_columns(true)
            .resources()
            .unwrap();
        assert!(html.contains(
            r#"<a href="https://hl7.org/fhir/R4/patient.html" target="_blank" rel="noopener">Patient</a>"#
        ));
        assert!(html.contains(r#"<span class="tag tag--member">read</span>"#));
        assert!(html.contains(r#"<td class="col-num">1</td>"#));
    }

    #[test]
    fn the_filter_form_is_opt_in_and_carries_the_release() {
        let view = sample_view();
        let plain = CapabilityCards::new(&Labels, &view).resources().unwrap();
        assert!(!plain.contains("<form"));
        // The htmx swap target is a stable hook either way.
        assert!(plain.contains(r#"id="cap-resource-table""#));

        let filtered = CapabilityCards::new(&Labels, &view)
            .filter(Some(ResourceFilter {
                action: "/ui/capability-statement",
                version: "R4B",
                value: "Pat",
            }))
            .resources()
            .unwrap();
        assert!(filtered.contains(r#"action="/ui/capability-statement""#));
        assert!(filtered.contains(r#"<input type="hidden" name="version" value="R4B">"#));
        assert!(filtered.contains(r#"value="Pat""#));
        assert!(filtered.contains(r##"hx-target="#cap-resource-table""##));
    }

    #[test]
    fn a_notice_replaces_every_card_body_and_keeps_every_heading() {
        let view = sample_view();
        let cards = CapabilityCards::new(&Labels, &view)
            .notice(Some("upstream is unreachable"))
            .transaction_note_href(Some("https://example.test/roles"))
            .show_include_columns(true)
            .filter(Some(ResourceFilter {
                action: "/ui/capability-statement",
                version: "R4",
                value: "",
            }));
        for (html, heading) in [
            (cards.summary().unwrap(), "cap-summary-heading"),
            (cards.interactions().unwrap(), "cap-interactions-heading"),
            (cards.operations().unwrap(), "cap-operations-heading"),
            (cards.resources().unwrap(), "cap-resources-heading"),
        ] {
            assert!(html.contains(heading), "the heading survives: {heading}");
            assert!(
                html.contains(r#"<p class="notice notice--warn">upstream is unreachable</p>"#),
                "the body degrades to the notice: {heading}",
            );
            assert!(!html.contains("<table"), "no stale table: {heading}");
            assert!(!html.contains("<form"), "no stale form: {heading}");
            assert!(!html.contains("Patient"), "no stale data: {heading}");
        }
    }

    #[test]
    fn labels_are_html_escaped_before_the_safe_filter() {
        // Card output is spliced into a page with `|safe`, so anything that
        // reaches it must already be escaped. A statement is attacker-shaped
        // data whenever a Helios server proxies someone else's `/metadata`.
        let hostile = json!({
            "implementation": {"description": "<script>alert(1)</script>"},
            "rest": [{"resource": [{"type": "<img src=x onerror=alert(1)>"}]}]
        });
        let view = build_view(&hostile, DocsVersion::R4, &NoCoreResources);
        let cards = CapabilityCards::new(&Labels, &view);
        let summary = cards.summary().unwrap();
        assert!(!summary.contains("<script>"));
        assert!(summary.contains("&#60;script&#62;"));
        let resources = cards.resources().unwrap();
        assert!(!resources.contains("<img src=x"));
    }
}
