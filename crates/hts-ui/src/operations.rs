//! Slice E — standalone Operations workbench (design doc §7.6).
//!
//! Ships:
//!   - `GET /hts/operations` — full-page shell (default `?op=lookup&resource=CodeSystem`).
//!   - `GET /hts/operations/input` — input-swap fragment dispatched by
//!     `?op=&resource=`.
//!   - Seven real runners (`lookup`, CS + VS `validate-code`, `subsumes`,
//!     `expand`, `translate`, `closure`) that proxy the
//!     [`UpstreamClient`] methods.
//!   - `batch-validate` UI-fabricated fan-out (§7.6.1 F1=D): seed handler +
//!     per-row polling target + progress-counter target, backed by an
//!     in-process job store bounded by [`HTS_UI_BATCH_FANOUT_CONCURRENCY`].
//!
//! Every operation proxies to HTS as POST regardless of the source
//! form verb (§7.6 verb rule). Op segments are literals — no `/{op}`
//! capture — so `tests/route_enum.rs` can enumerate them explicitly
//! (§7.6.1 invariant #6).

use askama::Template;
use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use axum_htmx::HxRequest;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{RwLock, Semaphore};

use crate::i18n::{I18n, RequestLocale};
use crate::upstream::{
    ClosureConcept, ClosureParams, ClosureResult, ExpandParams, ExpansionResult, LookupParams,
    LookupResult, OutcomeView, SubsumesParams, SubsumesResult, TranslateDirection, TranslateParams,
    TranslateResult, UpstreamClient, UpstreamError, ValidateCodeParams, ValidateCodeResult,
    ValidateInputMode, VsValidateMode, VsValidateParams, VsValidateResult, VsValidateSource,
    HTS_UI_BATCH_FANOUT_CONCURRENCY, HTS_UI_MAX_EXPANSION_SIZE_HINT,
};
use crate::{Chrome, HtsUiState};

/// Batch input row cap used by `hts-vs-batch-input.html` and the seed
/// handler pre-flight. Anything beyond this collapses to a page-level
/// `invalid` OperationOutcome without seeding a batch (§7.6.1 F1 bullet).
pub const HTS_UI_BATCH_MAX_ROWS: usize = 50;

// ── Routing ─────────────────────────────────────────────────────────────

pub(crate) fn routes() -> Router<Arc<HtsUiState>> {
    Router::new()
        .route("/hts/operations", get(operations_shell))
        .route("/hts/operations/input", get(operations_input))
        .route("/hts/operations/lookup", post(run_lookup))
        .route("/hts/operations/validate-code", post(run_validate_code))
        .route("/hts/operations/subsumes", post(run_subsumes))
        .route("/hts/operations/expand", post(run_expand))
        .route("/hts/operations/translate", post(run_translate))
        .route("/hts/operations/closure", post(run_closure))
        .route(
            "/hts/operations/batch-validate",
            post(run_batch_validate_seed),
        )
        .route(
            "/hts/operations/batch-validate/row/{i}",
            get(run_batch_validate_row),
        )
        .route(
            "/hts/operations/batch-validate/progress",
            get(batch_validate_progress),
        )
}

// ── Ops + resource families ─────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Lookup,
    ValidateCode,
    Subsumes,
    Expand,
    Translate,
    Closure,
    Batch,
}

impl OperationKind {
    pub fn from_query(value: &str) -> Self {
        match value {
            "validate-code" => Self::ValidateCode,
            "subsumes" => Self::Subsumes,
            "expand" => Self::Expand,
            "translate" => Self::Translate,
            "closure" => Self::Closure,
            "batch-validate" => Self::Batch,
            _ => Self::Lookup,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Lookup => "lookup",
            Self::ValidateCode => "validate-code",
            Self::Subsumes => "subsumes",
            Self::Expand => "expand",
            Self::Translate => "translate",
            Self::Closure => "closure",
            Self::Batch => "batch-validate",
        }
    }

    pub fn label_key(self) -> String {
        format!("hts-operations-op-{}", self.slug())
    }
}

/// Which FHIR resource family the current op is scoped to. `None` is
/// reserved for `$closure`, which is not resource-scoped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeResource {
    CodeSystem,
    ValueSet,
    ConceptMap,
    None,
}

impl ScopeResource {
    pub fn from_query(value: &str) -> Self {
        match value {
            "CodeSystem" => Self::CodeSystem,
            "ValueSet" => Self::ValueSet,
            "ConceptMap" => Self::ConceptMap,
            _ => Self::None,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::CodeSystem => "CodeSystem",
            Self::ValueSet => "ValueSet",
            Self::ConceptMap => "ConceptMap",
            Self::None => "",
        }
    }
}

fn default_resource(op: OperationKind) -> ScopeResource {
    match op {
        OperationKind::Lookup | OperationKind::Subsumes => ScopeResource::CodeSystem,
        OperationKind::ValidateCode | OperationKind::Batch => ScopeResource::CodeSystem,
        OperationKind::Expand => ScopeResource::ValueSet,
        OperationKind::Translate => ScopeResource::ConceptMap,
        OperationKind::Closure => ScopeResource::None,
    }
}

// ── Batch state (in-process job store) ──────────────────────────────────
//
// Batch fan-out state lives in a process-global store rather than on
// `HtsUiState`, so the mount site (`crates/hts/src/server.rs`) does not
// need to grow another constructor argument. The store is cheap enough
// (a `HashMap<String, Arc<RwLock<BatchJob>>>` behind an `RwLock`) that
// per-tenant isolation is not necessary for the operator surface. A
// job entry is keyed by a monotonic + wall-clock id so client-side
// polling can address a specific batch across requests. Jobs are not
// evicted in-process; the store grows with the number of submissions
// since the last server restart.

/// In-process batch fan-out state.
///
/// Public so `lib.rs` can re-export the type without exposing plumbing.
/// The seed handler inserts a job and spawns bounded workers; the
/// per-row polling handler reads the same job by id.
#[derive(Clone, Debug)]
pub struct BatchJobs {
    inner: Arc<RwLock<HashMap<String, Arc<RwLock<BatchJob>>>>>,
    semaphore: Arc<Semaphore>,
}

impl BatchJobs {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            semaphore: Arc::new(Semaphore::new(HTS_UI_BATCH_FANOUT_CONCURRENCY)),
        }
    }
}

impl Default for BatchJobs {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-global job store — see [`BatchJobs`] docs.
static BATCH_JOBS: OnceLock<BatchJobs> = OnceLock::new();

fn batch_jobs() -> &'static BatchJobs {
    BATCH_JOBS.get_or_init(BatchJobs::new)
}

#[derive(Clone, Debug)]
struct BatchJob {
    /// Canonical URL of the target ValueSet. Stored so a future
    /// resume-after-restart surface can rebuild the seed page from
    /// the job store without asking the operator to re-enter it.
    #[allow(dead_code)]
    target: String,
    rows: Vec<BatchRowState>,
    completed: usize,
    total: usize,
}

#[derive(Clone, Debug)]
struct BatchRowState {
    /// Row index inside the job. Stored so future rehydration surfaces
    /// don't need to lean on `Vec` position alone.
    #[allow(dead_code)]
    index: usize,
    input: BatchRowInput,
    result: Option<BatchRowResult>,
}

#[derive(Clone, Debug, Default)]
struct BatchRowInput {
    code: String,
    system: String,
    display: String,
}

/// Rendered payload for one completed batch row (`hts-vs-batch-row.html`).
#[derive(Clone, Debug, Default)]
struct BatchRowResult {
    /// `true` when the row terminated via a per-row timeout (§7.10 row
    /// 7.6 batch state — `severity=warning code=timeout`).
    is_warning: bool,
    /// Optional operator-facing message for the badge tooltip.
    message: String,
    /// Membership result (`result=true` / `result=false`). Ignored when
    /// `outcome.is_some()` or `is_warning`.
    result: bool,
    /// Display text HTS echoed back on `result=true`, if any.
    display: String,
    /// Row-scoped `OperationOutcome` (`severity=error code=exception`
    /// per §7.10). `Some` for any HTS 5xx / decode failure.
    outcome: Option<OutcomeView>,
}

fn new_batch_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    // A pseudo-random suffix based on the address of a fresh box, so
    // parallel tests do not collide even when the monotonic clock has
    // millisecond resolution.
    let boxed = Box::new(0u8);
    let addr = &*boxed as *const u8 as usize;
    drop(boxed);
    format!("b{nanos:x}-{addr:x}")
}

// ── Template flags (page + input dispatcher share the same shape) ───────

/// Precomputed flags the operations shell / input partials branch on.
///
/// Keeping the fan-out here (rather than as inline helper methods on the
/// page template) means the input-swap fragment and the shell page use
/// the exact same struct — no template needs to know the difference.
#[derive(Clone, Debug)]
pub struct OpsFlags {
    // Per-op booleans (matched by `hts-op-input.html`).
    pub is_lookup: bool,
    pub is_validate_code: bool,
    pub is_subsumes: bool,
    pub is_expand: bool,
    pub is_translate: bool,
    pub is_closure: bool,
    pub is_batch: bool,
    // Per-resource booleans (matched by `hts-resource-family-tabs.html`).
    pub is_resource_cs: bool,
    pub is_resource_vs: bool,
    // Shell-conditional regions.
    pub supports_resource_tabs: bool,
    pub shows_closure_banner: bool,
    pub shows_batch_progress: bool,
    pub op_slug: &'static str,
    pub op_selector_entries: Vec<OpSelectorEntry>,
}

impl OpsFlags {
    fn new(op: OperationKind, resource: ScopeResource) -> Self {
        Self {
            is_lookup: matches!(op, OperationKind::Lookup),
            is_validate_code: matches!(op, OperationKind::ValidateCode),
            is_subsumes: matches!(op, OperationKind::Subsumes),
            is_expand: matches!(op, OperationKind::Expand),
            is_translate: matches!(op, OperationKind::Translate),
            is_closure: matches!(op, OperationKind::Closure),
            is_batch: matches!(op, OperationKind::Batch),
            is_resource_cs: matches!(resource, ScopeResource::CodeSystem),
            is_resource_vs: matches!(resource, ScopeResource::ValueSet),
            supports_resource_tabs: matches!(
                op,
                OperationKind::ValidateCode | OperationKind::Batch
            ),
            shows_closure_banner: matches!(op, OperationKind::Closure),
            shows_batch_progress: matches!(op, OperationKind::Batch),
            op_slug: op.slug(),
            op_selector_entries: build_op_selector_entries(op),
        }
    }
}

#[derive(Clone, Debug)]
pub struct OpSelectorEntry {
    pub slug: &'static str,
    pub resource: &'static str,
    pub label_key: String,
    pub active: bool,
}

fn build_op_selector_entries(active: OperationKind) -> Vec<OpSelectorEntry> {
    const OPS: &[OperationKind] = &[
        OperationKind::Lookup,
        OperationKind::ValidateCode,
        OperationKind::Subsumes,
        OperationKind::Expand,
        OperationKind::Translate,
        OperationKind::Closure,
        OperationKind::Batch,
    ];
    OPS.iter()
        .map(|op| OpSelectorEntry {
            slug: op.slug(),
            resource: default_resource(*op).slug(),
            label_key: op.label_key(),
            active: *op == active,
        })
        .collect()
}

// ── Shell / input-swap ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default, Clone)]
struct ShellQuery {
    op: Option<String>,
    resource: Option<String>,
    #[allow(dead_code)]
    lang: Option<String>,
}

impl ShellQuery {
    fn op(&self) -> OperationKind {
        OperationKind::from_query(self.op.as_deref().unwrap_or(""))
    }

    fn resource(&self) -> ScopeResource {
        match self.resource.as_deref() {
            Some(s) if !s.is_empty() => ScopeResource::from_query(s),
            _ => default_resource(self.op()),
        }
    }
}

#[derive(Template)]
#[template(path = "pages/operations.html")]
struct OperationsPageTemplate<'a> {
    chrome: Chrome<'a>,
    flags: OpsFlags,
    resource: ScopeResource,
    ceiling: u64,
    // Read by the E2 batch table/progress templates once they land; keep
    // the field so the plumbing is in place. Silence the unused warning
    // until then.
    #[allow(dead_code)]
    batch_max_rows: usize,
}

#[derive(Template)]
#[template(path = "partials/hts-op-input.html")]
struct OperationsInputTemplate<'a> {
    chrome: Chrome<'a>,
    flags: OpsFlags,
    resource: ScopeResource,
    ceiling: u64,
    #[allow(dead_code)]
    batch_max_rows: usize,
}

async fn operations_shell(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
    Query(query): Query<ShellQuery>,
) -> Response {
    let op = query.op();
    let resource = query.resource();
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "operations",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    if is_htmx {
        // htmx `hx-target="body"` still delivers a full-shell swap; but
        // when a downstream partial asks for a fragment (resource-tab
        // strip) via the same URL, we still ship the shell. The Slice E1
        // decision is: `/operations` = shell in both modes, no
        // fragment split — that keeps the URL contract in sync with
        // F14 (a plain `<a href>` load renders the same shell).
    }
    render(
        OperationsPageTemplate {
            chrome,
            flags: OpsFlags::new(op, resource),
            resource,
            ceiling: HTS_UI_MAX_EXPANSION_SIZE_HINT,
            batch_max_rows: HTS_UI_BATCH_MAX_ROWS,
        }
        .render(),
    )
}

async fn operations_input(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    Query(query): Query<ShellQuery>,
) -> Response {
    let op = query.op();
    let resource = query.resource();
    let chrome = Chrome {
        i18n: I18n::new(locale),
        active_page: "operations",
        fhir_version: state.fhir_version,
        version: state.version,
    };
    render(
        OperationsInputTemplate {
            chrome,
            flags: OpsFlags::new(op, resource),
            resource,
            ceiling: HTS_UI_MAX_EXPANSION_SIZE_HINT,
            batch_max_rows: HTS_UI_BATCH_MAX_ROWS,
        }
        .render(),
    )
}

// ── Shared result view (all runners use this shape) ─────────────────────

/// Result dispatcher template. Slice E2 branches on the source op so
/// the closure and VS `$validate-code` result partials can own their
/// bespoke layouts (edge-list; neutral `result=false` badge) while the
/// remaining ops keep sharing `hts-op-generic-result.html`.
#[derive(Template)]
#[template(path = "partials/hts-op-result.html")]
struct OpResultTemplate<'a> {
    chrome: Chrome<'a>,
    view: OpResultView,
    is_closure: bool,
    is_validate_code: bool,
}

/// Payload for the shared operations result partial
/// (`partials/hts-op-generic-result.html`). Each op populates only its
/// own slot; the template branches on `Some(_)` per slot without needing
/// to know which op was run.
#[derive(Clone, Debug, Default)]
pub struct OpResultView {
    pub request_url: String,
    pub raw_body: String,
    pub lookup: Option<LookupResult>,
    pub validate_code: Option<ValidateCodeResult>,
    pub vs_validate: Option<VsValidateResult>,
    pub subsumes: Option<SubsumesResult>,
    pub expand: Option<ExpansionResult>,
    pub translate: Option<TranslateResult>,
    pub closure: Option<ClosureResult>,
    pub outcome: Option<OutcomeView>,
    pub degraded_reason: Option<&'static str>,
}

impl OpResultView {
    fn invalid_input(msg: impl Into<String>) -> Self {
        Self {
            outcome: Some(OutcomeView::invalid_input(msg.into())),
            ..Self::default()
        }
    }

    fn from_error(request_url: String, err: &UpstreamError) -> Self {
        let mut view = Self::default();
        view.request_url = request_url;
        match err {
            UpstreamError::Outcome { outcome, .. } => view.outcome = Some(outcome.clone()),
            UpstreamError::NotFound { .. } => {
                view.outcome = Some(OutcomeView {
                    severity: "error".to_owned(),
                    code: "not-found".to_owned(),
                    ..OutcomeView::default()
                });
            }
            UpstreamError::HttpStatus { status, .. } => {
                view.outcome = Some(OutcomeView {
                    severity: "error".to_owned(),
                    code: match *status {
                        400 => "invalid".to_owned(),
                        404 => "not-found".to_owned(),
                        422 => "too-costly".to_owned(),
                        _ => "unknown".to_owned(),
                    },
                    ..OutcomeView::default()
                });
            }
            UpstreamError::Connect { .. }
            | UpstreamError::Timeout { .. }
            | UpstreamError::ClientBuild { .. } => {
                view.degraded_reason = Some(err.degraded_reason());
            }
            UpstreamError::Decode { message, .. } => {
                view.outcome = Some(OutcomeView::invalid_input(message.clone()));
            }
        }
        view
    }
}

fn render_result(chrome: Chrome<'_>, view: OpResultView, op: OperationKind) -> Response {
    let is_closure = matches!(op, OperationKind::Closure);
    let is_validate_code = matches!(op, OperationKind::ValidateCode);
    render(
        OpResultTemplate {
            chrome,
            view,
            is_closure,
            is_validate_code,
        }
        .render(),
    )
}

// ── Runners (5 real ops) ────────────────────────────────────────────────

async fn run_lookup(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let chrome = chrome_of(&state, locale);
    let form = parse_form(&body);
    let system = single(&form, "system");
    let params = LookupParams {
        code: single(&form, "code"),
        version: opt(&form, "version"),
        display_language: opt(&form, "displayLanguage"),
        properties: multi(&form, "property"),
        date: opt(&form, "date"),
    };
    if system.trim().is_empty() || params.code.trim().is_empty() {
        return render_result(
            chrome,
            OpResultView::invalid_input("`system` and `code` are required"),
            OperationKind::Lookup,
        );
    }
    let request_url = format!("{}/CodeSystem/$lookup", state.upstream.base_url());
    let view = match state.upstream.cs_lookup_type_level(&system, params).await {
        Ok(r) => OpResultView {
            request_url: r.request_url.clone(),
            raw_body: r.raw_body.clone(),
            lookup: Some(r),
            ..OpResultView::default()
        },
        Err(err) => OpResultView::from_error(request_url, &err),
    };
    render_result(chrome, view, OperationKind::Lookup)
}

async fn run_validate_code(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let chrome = chrome_of(&state, locale);
    let form = parse_form(&body);
    let resource = ScopeResource::from_query(&single(&form, "resource"));
    if resource == ScopeResource::ValueSet {
        return run_vs_validate_code(chrome, &state, &form).await;
    }
    let system = single(&form, "system");
    let params = ValidateCodeParams {
        mode: ValidateInputMode::from_form(
            form.get("mode").and_then(|v| v.first()).map(String::as_str),
        ),
        code: single(&form, "code"),
        display: opt(&form, "display"),
        coding_system: single(&form, "coding.system"),
        coding_code: single(&form, "coding.code"),
        coding_display: opt(&form, "coding.display"),
        display_language: opt(&form, "displayLanguage"),
    };
    if system.trim().is_empty() {
        return render_result(
            chrome,
            OpResultView::invalid_input("`system` (CodeSystem canonical URL) is required"),
            OperationKind::ValidateCode,
        );
    }
    if matches!(params.mode, ValidateInputMode::Code) && params.code.trim().is_empty() {
        return render_result(
            chrome,
            OpResultView::invalid_input("`code` is required in code mode"),
            OperationKind::ValidateCode,
        );
    }
    if matches!(params.mode, ValidateInputMode::Coding)
        && (params.coding_code.trim().is_empty() || params.coding_system.trim().is_empty())
    {
        return render_result(
            chrome,
            OpResultView::invalid_input("`coding.system` and `coding.code` are required"),
            OperationKind::ValidateCode,
        );
    }
    let request_url = format!("{}/CodeSystem/$validate-code", state.upstream.base_url());
    let view = match state.upstream.cs_validate_code(&system, params).await {
        Ok(r) => OpResultView {
            request_url: r.request_url.clone(),
            raw_body: r.raw_body.clone(),
            validate_code: Some(r),
            ..OpResultView::default()
        },
        Err(err) => OpResultView::from_error(request_url, &err),
    };
    render_result(chrome, view, OperationKind::ValidateCode)
}

/// VS `$validate-code` branch (design doc §7.6 F4 + §7.4.1 F9). Called
/// from [`run_validate_code`] when `resource=ValueSet`. Ships the full
/// input matrix (canonical / instance / inline) × three input-shape
/// modes (code / Coding / CodeableConcept) as documented in the skill
/// §6 parameter table. `result=false` is neutral (§7.6 F11), not an
/// error surface.
async fn run_vs_validate_code(
    chrome: Chrome<'_>,
    state: &HtsUiState,
    form: &HashMap<String, Vec<String>>,
) -> Response {
    let source_mode = single(form, "sourceMode");
    let source = match source_mode.as_str() {
        "instance" => {
            let id = single(form, "sourceInstance");
            if id.trim().is_empty() {
                return render_result(
                    chrome,
                    OpResultView::invalid_input(
                        "`sourceInstance` is required in instance mode",
                    ),
                    OperationKind::ValidateCode,
                );
            }
            VsValidateSource::Instance(id)
        }
        "inline" => {
            let json = single(form, "sourceInline");
            if json.trim().is_empty() {
                return render_result(
                    chrome,
                    OpResultView::invalid_input(
                        "`sourceInline` must contain a ValueSet JSON body",
                    ),
                    OperationKind::ValidateCode,
                );
            }
            VsValidateSource::Inline(json)
        }
        _ => {
            let canonical = single(form, "sourceCanonical");
            if canonical.trim().is_empty() {
                return render_result(
                    chrome,
                    OpResultView::invalid_input(
                        "`sourceCanonical` (canonical URL) is required in canonical mode",
                    ),
                    OperationKind::ValidateCode,
                );
            }
            VsValidateSource::Canonical(canonical)
        }
    };
    let mode = VsValidateMode::from_form_value(&single(form, "mode"));
    let params = VsValidateParams {
        mode,
        code: single(form, "code"),
        system: single(form, "system"),
        system_version: opt(form, "systemVersion"),
        coding_system: single(form, "coding.system"),
        coding_code: single(form, "coding.code"),
        coding_display: single(form, "coding.display"),
        coding_rows: collect_concept_rows(form, "coding[].system", "coding[].code"),
        codeable_concept_text: opt(form, "codeableConcept.text"),
        display: opt(form, "display"),
        display_language: opt(form, "displayLanguage"),
        valueset_version: opt(form, "valueSetVersion"),
        date: opt(form, "date"),
        active_only: bool_opt(form, "activeOnly"),
        abstract_ok: bool_opt(form, "abstract"),
        lenient_display_validation: bool_opt(form, "lenient-display-validation"),
        use_supplement: multi(form, "useSupplement"),
        tx_resource: multi(form, "tx-resource"),
        system_version_pins: multi(form, "system-version"),
        check_system_version: multi(form, "check-system-version"),
        force_system_version: multi(form, "force-system-version"),
        default_valueset_version: opt(form, "default-valueset-version"),
    };
    // Pre-flight: `code` mode without a code and no fallback Coding is a
    // deterministic invalid submit — do not burn an HTS round-trip.
    if matches!(mode, VsValidateMode::Code) && params.code.trim().is_empty() {
        return render_result(
            chrome,
            OpResultView::invalid_input("`code` is required in code mode"),
            OperationKind::ValidateCode,
        );
    }
    if matches!(mode, VsValidateMode::Coding)
        && params.coding_code.trim().is_empty()
    {
        return render_result(
            chrome,
            OpResultView::invalid_input("`coding.code` is required in Coding mode"),
            OperationKind::ValidateCode,
        );
    }
    let request_url = match &source {
        VsValidateSource::Instance(id) => {
            format!("{}/ValueSet/{}/$validate-code", state.upstream.base_url(), id)
        }
        _ => format!("{}/ValueSet/$validate-code", state.upstream.base_url()),
    };
    let view = match state.upstream.vs_validate_code(&source, &params).await {
        Ok(r) => OpResultView {
            request_url: r.request_url.clone(),
            raw_body: r.raw_body.clone(),
            vs_validate: Some(r),
            ..OpResultView::default()
        },
        Err(err) => OpResultView::from_error(request_url, &err),
    };
    render_result(chrome, view, OperationKind::ValidateCode)
}

async fn run_subsumes(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let chrome = chrome_of(&state, locale);
    let form = parse_form(&body);
    let system = single(&form, "system");
    let params = SubsumesParams {
        code_a: single(&form, "codeA"),
        code_b: single(&form, "codeB"),
        version: opt(&form, "version"),
    };
    if system.trim().is_empty()
        || params.code_a.trim().is_empty()
        || params.code_b.trim().is_empty()
    {
        return render_result(
            chrome,
            OpResultView::invalid_input("`system`, `codeA`, and `codeB` are required"),
            OperationKind::Subsumes,
        );
    }
    let request_url = format!("{}/CodeSystem/$subsumes", state.upstream.base_url());
    let view = match state.upstream.cs_subsumes(&system, params).await {
        Ok(r) => OpResultView {
            request_url: r.request_url.clone(),
            raw_body: r.raw_body.clone(),
            subsumes: Some(r),
            ..OpResultView::default()
        },
        Err(err) => OpResultView::from_error(request_url, &err),
    };
    render_result(chrome, view, OperationKind::Subsumes)
}

async fn run_expand(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let chrome = chrome_of(&state, locale);
    let form = parse_form(&body);
    // Instance-id slot: E1 baseline. Canonical + inline JSON slots are
    // documented in §7.6.1 F8 as a follow-up; the surface still parses
    // both keys (`sourceCanonical` / `sourceInline`) so the E2 template
    // update can add the radio without a handler change.
    let instance = single(&form, "sourceInstance");
    if instance.trim().is_empty() {
        return render_result(
            chrome,
            OpResultView::invalid_input(
                "the standalone expand workbench requires a ValueSet instance id",
            ),
            OperationKind::Expand,
        );
    }
    let mode = single(&form, "mode");
    let (hierarchical, exclude_nested) = match mode.as_str() {
        "tree" => (Some(true), None),
        _ => (None, Some(true)),
    };
    let params = ExpandParams {
        filter: opt(&form, "filter"),
        count: parse_u32(&form, "count"),
        offset: parse_u32(&form, "offset"),
        display_language: opt(&form, "displayLanguage"),
        active_only: Some(checkbox(&form, "activeOnly")),
        include_designations: Some(checkbox(&form, "includeDesignations")),
        use_supplement: multi(&form, "useSupplement"),
        date: opt(&form, "date"),
        property: multi(&form, "property"),
        tx_resource: multi(&form, "tx-resource"),
        system_version: multi(&form, "system-version"),
        check_system_version: multi(&form, "check-system-version"),
        force_system_version: multi(&form, "force-system-version"),
        default_valueset_version: opt(&form, "default-valueset-version"),
        hierarchical,
        exclude_nested,
        threshold: parse_u64(&form, "threshold"),
    };
    let request_url = format!(
        "{}/ValueSet/{}/$expand",
        state.upstream.base_url(),
        instance
    );
    let view = match state.upstream.vs_expand_instance(&instance, &params).await {
        Ok(r) => OpResultView {
            request_url: r.request_url.clone(),
            raw_body: r.raw_body.clone(),
            expand: Some(r),
            ..OpResultView::default()
        },
        Err(err) => OpResultView::from_error(request_url, &err),
    };
    render_result(chrome, view, OperationKind::Expand)
}

async fn run_translate(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let chrome = chrome_of(&state, locale);
    let form = parse_form(&body);
    let instance = single(&form, "sourceInstance");
    if instance.trim().is_empty() {
        return render_result(
            chrome,
            OpResultView::invalid_input(
                "the standalone translate workbench requires a ConceptMap instance id",
            ),
            OperationKind::Translate,
        );
    }
    let direction = TranslateDirection::from_form(opt(&form, "direction").as_deref());
    let params = TranslateParams {
        direction,
        code: opt(&form, "code"),
        system: opt(&form, "system"),
        display: opt(&form, "display"),
        target_code: opt(&form, "targetCode"),
        target_system: opt(&form, "targetSystem"),
        source_url: opt(&form, "source"),
        target_url: opt(&form, "target"),
        date: opt(&form, "date"),
    };
    match direction {
        TranslateDirection::Forward => {
            let code = params.code.as_deref().unwrap_or_default().trim();
            let system = params.system.as_deref().unwrap_or_default().trim();
            if code.is_empty() || system.is_empty() {
                return render_result(
                    chrome,
                    OpResultView::invalid_input(
                        "Forward translation requires both `code` and `system`.",
                    ),
                    OperationKind::Translate,
                );
            }
        }
        TranslateDirection::Reverse => {
            let target = params.target_code.as_deref().unwrap_or_default().trim();
            if target.is_empty() {
                return render_result(
                    chrome,
                    OpResultView::invalid_input("Reverse translation requires `targetCode`."),
                    OperationKind::Translate,
                );
            }
        }
    }
    let request_url = format!(
        "{}/ConceptMap/{}/$translate",
        state.upstream.base_url(),
        instance
    );
    let view = match state
        .upstream
        .cm_translate_instance(&instance, &params)
        .await
    {
        Ok(r) => OpResultView {
            request_url: r.request_url.clone(),
            raw_body: r.raw_body.clone(),
            translate: Some(r),
            ..OpResultView::default()
        },
        Err(err) => OpResultView::from_error(request_url, &err),
    };
    render_result(chrome, view, OperationKind::Translate)
}

// ── Slice E2: real closure + batch handlers ────────────────────────────

async fn run_closure(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let chrome = chrome_of(&state, locale);
    let form = parse_form(&body);
    let name = single(&form, "name");
    if name.trim().is_empty() {
        return render_result(
            chrome,
            OpResultView::invalid_input(
                "`name` (closure table identifier) is required",
            ),
            OperationKind::Closure,
        );
    }
    let concepts = collect_concept_rows(&form, "concept.system", "concept.code");
    let params = ClosureParams { name, concepts };
    let request_url = format!("{}/ConceptMap/$closure", state.upstream.base_url());
    let view = match state.upstream.cm_closure(&params).await {
        Ok(r) => OpResultView {
            request_url: r.request_url.clone(),
            raw_body: r.raw_body.clone(),
            closure: Some(r),
            ..OpResultView::default()
        },
        Err(err) => OpResultView::from_error(request_url, &err),
    };
    render_result(chrome, view, OperationKind::Closure)
}

// ── Batch-validate fan-out (§7.6.1 F1 = D) ─────────────────────────────

/// Rendered payload for the batch seed skeleton table
/// (`hts-vs-batch-table.html`).
#[derive(Template)]
#[template(path = "partials/hts-vs-batch-table.html")]
struct BatchTableTemplate<'a> {
    chrome: Chrome<'a>,
    target: String,
    batch_id: String,
    rows: Vec<BatchTableRow>,
}

#[derive(Clone, Debug)]
struct BatchTableRow {
    index: usize,
    code: String,
    system: String,
    display: String,
}

/// Rendered payload for a single completed batch row
/// (`hts-vs-batch-row.html`).
#[derive(Template)]
#[template(path = "partials/hts-vs-batch-row.html")]
struct BatchRowTemplate<'a> {
    chrome: Chrome<'a>,
    index: usize,
    input: BatchRowInput,
    result: Option<BatchRowResult>,
}

/// Rendered payload for the batch progress region
/// (`hts-vs-batch-progress.html`).
#[derive(Template)]
#[template(path = "partials/hts-vs-batch-progress.html")]
struct BatchProgressTemplate<'a> {
    chrome: Chrome<'a>,
    done: bool,
    completed: usize,
    total: usize,
}

async fn run_batch_validate_seed(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(is_htmx): HxRequest,
    locale: RequestLocale,
    body: Bytes,
) -> Response {
    let chrome = chrome_of(&state, locale);
    let form = parse_form(&body);
    let target = single(&form, "target");
    if target.trim().is_empty() {
        return render_result(
            chrome,
            OpResultView::invalid_input(
                "`target` (canonical URL of the target ValueSet) is required",
            ),
            OperationKind::Batch,
        );
    }
    let rows = collect_batch_rows(&form);
    if rows.is_empty() {
        return render_result(
            chrome,
            OpResultView::invalid_input(
                "at least one non-empty row is required to seed a batch",
            ),
            OperationKind::Batch,
        );
    }
    if rows.len() > HTS_UI_BATCH_MAX_ROWS {
        return render_result(
            chrome,
            OpResultView::invalid_input(format!(
                "batch input exceeds the `{HTS_UI_BATCH_MAX_ROWS}` row cap; \
                 split the submission before retrying"
            )),
            OperationKind::Batch,
        );
    }

    if is_htmx {
        run_batch_seed_htmx(chrome, state.upstream.clone(), target, rows).await
    } else {
        // nojs path: fan out synchronously, still bounded by the shared
        // semaphore, and pre-render the completed table (§7.6 F14).
        run_batch_seed_synchronous(chrome, state.upstream.clone(), target, rows).await
    }
}

/// htmx path — seed the job store, spawn bounded workers, and return
/// the skeleton table so per-row polling can begin.
async fn run_batch_seed_htmx(
    chrome: Chrome<'_>,
    upstream: UpstreamClient,
    target: String,
    inputs: Vec<BatchRowInput>,
) -> Response {
    let batch_id = new_batch_id();
    let total = inputs.len();
    let job = BatchJob {
        target: target.clone(),
        rows: inputs
            .iter()
            .enumerate()
            .map(|(index, input)| BatchRowState {
                index,
                input: input.clone(),
                result: None,
            })
            .collect(),
        completed: 0,
        total,
    };
    let store = batch_jobs();
    let handle = Arc::new(RwLock::new(job));
    store
        .inner
        .write()
        .await
        .insert(batch_id.clone(), handle.clone());

    // Spawn one task per row; each task acquires a permit from the
    // shared semaphore before making the outbound HTS call. Total task
    // count is bounded by `total` (≤ HTS_UI_BATCH_MAX_ROWS).
    let semaphore = store.semaphore.clone();
    for (i, input) in inputs.into_iter().enumerate() {
        let upstream = upstream.clone();
        let handle = handle.clone();
        let target_url = target.clone();
        let semaphore = semaphore.clone();
        tokio::spawn(async move {
            let _permit = semaphore.acquire().await;
            let result = run_batch_row_upstream(&upstream, &target_url, &input).await;
            let mut guard = handle.write().await;
            if let Some(row) = guard.rows.get_mut(i) {
                row.result = Some(result);
                guard.completed += 1;
            }
        });
    }

    let table_rows: Vec<BatchTableRow> = (0..total)
        .zip(handle.read().await.rows.iter())
        .map(|(index, row)| BatchTableRow {
            index,
            code: row.input.code.clone(),
            system: row.input.system.clone(),
            display: row.input.display.clone(),
        })
        .collect();
    render(
        BatchTableTemplate {
            chrome,
            target,
            batch_id,
            rows: table_rows,
        }
        .render(),
    )
}

/// nojs path — fan out synchronously and pre-render the completed
/// table so a plain form POST still returns results (§7.6 F14).
async fn run_batch_seed_synchronous(
    chrome: Chrome<'_>,
    upstream: UpstreamClient,
    target: String,
    inputs: Vec<BatchRowInput>,
) -> Response {
    let store = batch_jobs();
    let batch_id = new_batch_id();
    let semaphore = store.semaphore.clone();
    let mut results: Vec<Option<BatchRowResult>> = vec![None; inputs.len()];
    // Serialize per-permit acquisitions so upstream fan-out is bounded
    // even in the sync arm.
    for (i, input) in inputs.iter().enumerate() {
        let _permit = semaphore.acquire().await;
        let r = run_batch_row_upstream(&upstream, &target, input).await;
        results[i] = Some(r);
    }
    let job = BatchJob {
        target: target.clone(),
        rows: inputs
            .iter()
            .enumerate()
            .zip(results.iter())
            .map(|((index, input), result)| BatchRowState {
                index,
                input: input.clone(),
                result: result.clone(),
            })
            .collect(),
        completed: inputs.len(),
        total: inputs.len(),
    };
    store
        .inner
        .write()
        .await
        .insert(batch_id.clone(), Arc::new(RwLock::new(job)));

    // Compose a nojs-friendly page-level table: pre-fill each row with
    // its completed result so the operator does not need JS to see the
    // outcomes.
    let mut html = String::new();
    html.push_str(&format!(
        r#"<div id="hts-workbench-result" class="hts-op-workbench__result">"#,
    ));
    html.push_str(&format!(
        r#"<h3 class="hts-op-workbench__result-heading">{}</h3>"#,
        html_escape(&chrome.i18n.t("hts-vs-batch-result-heading")),
    ));
    html.push_str(&format!(
        r#"<p class="hts-op-workbench__hint">{}</p>"#,
        html_escape(
            &chrome
                .i18n
                .t_arg("hts-vs-batch-target-hint", "target", target.clone()),
        ),
    ));
    html.push_str(r#"<table class="hts-op-workbench__table"><thead><tr>"#);
    for key in [
        "hts-vs-batch-column-code",
        "hts-vs-batch-column-system",
        "hts-vs-batch-column-display",
        "hts-vs-batch-column-result",
    ] {
        html.push_str(&format!(
            r#"<th scope="col">{}</th>"#,
            html_escape(&chrome.i18n.t(key))
        ));
    }
    html.push_str(r#"</tr></thead><tbody>"#);
    for (i, input) in inputs.iter().enumerate() {
        let row_html = BatchRowTemplate {
            chrome,
            index: i,
            input: input.clone(),
            result: results[i].clone(),
        }
        .render()
        .unwrap_or_default();
        html.push_str(&row_html);
    }
    html.push_str(r#"</tbody></table>"#);
    let progress = BatchProgressTemplate {
        chrome,
        done: true,
        completed: inputs.len(),
        total: inputs.len(),
    }
    .render()
    .unwrap_or_default();
    html.push_str(&progress);
    html.push_str(r#"</div>"#);
    Html(html).into_response()
}

/// Per-row upstream execution. Returns a rendered payload the caller
/// stores on the job (htmx path) or emits inline (nojs path).
async fn run_batch_row_upstream(
    upstream: &UpstreamClient,
    target: &str,
    input: &BatchRowInput,
) -> BatchRowResult {
    let source = VsValidateSource::Canonical(target.to_string());
    let params = VsValidateParams {
        mode: if input.code.trim().is_empty() && !input.system.trim().is_empty() {
            VsValidateMode::Coding
        } else {
            VsValidateMode::Code
        },
        code: input.code.clone(),
        system: input.system.clone(),
        coding_system: input.system.clone(),
        coding_code: input.code.clone(),
        coding_display: input.display.clone(),
        display: if input.display.trim().is_empty() {
            None
        } else {
            Some(input.display.clone())
        },
        ..VsValidateParams::default()
    };
    match upstream.vs_validate_code(&source, &params).await {
        Ok(r) => BatchRowResult {
            is_warning: false,
            message: r.message.clone(),
            result: r.result,
            display: r.display,
            outcome: None,
        },
        Err(UpstreamError::Timeout { message, .. }) => BatchRowResult {
            is_warning: true,
            message,
            result: false,
            display: String::new(),
            outcome: None,
        },
        Err(err) => {
            let outcome = match &err {
                UpstreamError::Outcome { outcome, .. } => outcome.clone(),
                _ => OutcomeView {
                    severity: "error".to_owned(),
                    code: "exception".to_owned(),
                    diagnostics: err.to_string(),
                    ..OutcomeView::default()
                },
            };
            BatchRowResult {
                is_warning: false,
                message: err.to_string(),
                result: false,
                display: String::new(),
                outcome: Some(outcome),
            }
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct BatchRowQuery {
    batch_id: Option<String>,
}

async fn run_batch_validate_row(
    State(state): State<Arc<HtsUiState>>,
    Path(index): Path<usize>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    Query(query): Query<BatchRowQuery>,
) -> Response {
    let chrome = chrome_of(&state, locale);
    let batch_id = query.batch_id.unwrap_or_default();
    let handle = match batch_jobs().inner.read().await.get(&batch_id).cloned() {
        Some(h) => h,
        None => {
            return render_missing_batch_row(chrome, index);
        }
    };
    // Wait — with a per-row deadline — until the background task
    // populates the result slot. This lets the client poll once per
    // row (`hx-trigger="load"`) and still see the completed row even
    // when the upstream is slow. Deadline mirrors the per-row upstream
    // timeout to avoid indefinite hangs.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    loop {
        {
            let job = handle.read().await;
            if let Some(row) = job.rows.get(index) {
                if row.result.is_some() {
                    let input = row.input.clone();
                    let result = row.result.clone();
                    drop(job);
                    return render(
                        BatchRowTemplate {
                            chrome,
                            index,
                            input,
                            result,
                        }
                        .render(),
                    );
                }
            } else {
                drop(job);
                return render_missing_batch_row(chrome, index);
            }
        }
        if std::time::Instant::now() >= deadline {
            let job = handle.read().await;
            let input = job
                .rows
                .get(index)
                .map(|r| r.input.clone())
                .unwrap_or_default();
            drop(job);
            let result = BatchRowResult {
                is_warning: true,
                message: "batch row polling deadline exceeded".to_owned(),
                ..BatchRowResult::default()
            };
            return render(
                BatchRowTemplate {
                    chrome,
                    index,
                    input,
                    result: Some(result),
                }
                .render(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn render_missing_batch_row(chrome: Chrome<'_>, index: usize) -> Response {
    // Missing job id — probably a client that fetched a stale batch
    // page after a server restart. Render a row-scoped invalid-input
    // outcome so the counters still make sense.
    let result = BatchRowResult {
        outcome: Some(OutcomeView {
            severity: "error".to_owned(),
            code: "not-found".to_owned(),
            diagnostics: "batch job not found".to_owned(),
            ..OutcomeView::default()
        }),
        ..BatchRowResult::default()
    };
    render(
        BatchRowTemplate {
            chrome,
            index,
            input: BatchRowInput::default(),
            result: Some(result),
        }
        .render(),
    )
}

async fn batch_validate_progress(
    State(state): State<Arc<HtsUiState>>,
    HxRequest(_is_htmx): HxRequest,
    locale: RequestLocale,
    Query(query): Query<BatchRowQuery>,
) -> Response {
    let chrome = chrome_of(&state, locale);
    let batch_id = query.batch_id.unwrap_or_default();
    let handle = batch_jobs().inner.read().await.get(&batch_id).cloned();
    let (completed, total) = match handle {
        Some(h) => {
            let job = h.read().await;
            (job.completed, job.total)
        }
        None => (0, 0),
    };
    let done = total > 0 && completed >= total;
    render(
        BatchProgressTemplate {
            chrome,
            done,
            completed,
            total,
        }
        .render(),
    )
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn chrome_of<'a>(state: &'a HtsUiState, locale: RequestLocale) -> Chrome<'a> {
    Chrome {
        i18n: I18n::new(locale),
        active_page: "operations",
        fhir_version: state.fhir_version,
        version: state.version,
    }
}

fn render(rendered: Result<String, askama::Error>) -> Response {
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(?err, "hts-ui operations template render failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Html(String::from("<pre>hts-ui: operations render error</pre>")),
            )
                .into_response()
        }
    }
}

fn parse_form(body: &[u8]) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in form_urlencoded::parse(body) {
        map.entry(k.into_owned()).or_default().push(v.into_owned());
    }
    map
}

fn single(form: &HashMap<String, Vec<String>>, key: &str) -> String {
    form.get(key)
        .and_then(|v| v.first())
        .cloned()
        .unwrap_or_default()
}

fn opt(form: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
    form.get(key)
        .and_then(|v| v.first())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn multi(form: &HashMap<String, Vec<String>>, key: &str) -> Vec<String> {
    form.get(key)
        .map(|v| v.iter().filter(|s| !s.trim().is_empty()).cloned().collect())
        .unwrap_or_default()
}

fn checkbox(form: &HashMap<String, Vec<String>>, key: &str) -> bool {
    form.get(key)
        .and_then(|v| v.first())
        .map(|s| {
            let t = s.trim();
            !t.is_empty() && !t.eq_ignore_ascii_case("false") && !t.eq_ignore_ascii_case("off")
        })
        .unwrap_or(false)
}

fn parse_u32(form: &HashMap<String, Vec<String>>, key: &str) -> Option<u32> {
    opt(form, key).and_then(|s| s.parse::<u32>().ok())
}

fn parse_u64(form: &HashMap<String, Vec<String>>, key: &str) -> Option<u64> {
    opt(form, key).and_then(|s| s.parse::<u64>().ok())
}

/// Read a possibly-repeatable field without dropping empty entries.
/// Used by the closure Coding rows and batch row grid so parallel
/// indexing across three input columns stays aligned.
fn raw_multi(form: &HashMap<String, Vec<String>>, key: &str) -> Vec<String> {
    form.get(key).cloned().unwrap_or_default()
}

/// Best-effort tri-state read of a checkbox: `Some(true)` when the box
/// is checked (form carries any non-empty non-"off"/"false" value),
/// `None` when the field is absent. Distinct from [`checkbox`] because
/// the vs-validate parameter shape uses `Option<bool>` (missing =
/// server default), not a plain `bool`.
fn bool_opt(form: &HashMap<String, Vec<String>>, key: &str) -> Option<bool> {
    if form.contains_key(key) {
        Some(checkbox(form, key))
    } else {
        None
    }
}

/// Collect a `Vec<ClosureConcept>` from two parallel repeatable form
/// fields — used by both closure Coding rows and VS-validate
/// CodeableConcept mode. Empty pairs are dropped.
fn collect_concept_rows(
    form: &HashMap<String, Vec<String>>,
    system_key: &str,
    code_key: &str,
) -> Vec<ClosureConcept> {
    let systems = raw_multi(form, system_key);
    let codes = raw_multi(form, code_key);
    let n = systems.len().max(codes.len());
    let mut out = Vec::new();
    for i in 0..n {
        let system = systems.get(i).cloned().unwrap_or_default();
        let code = codes.get(i).cloned().unwrap_or_default();
        if system.trim().is_empty() && code.trim().is_empty() {
            continue;
        }
        out.push(ClosureConcept { system, code });
    }
    out
}

/// Collect batch input rows from parallel `row.code`, `row.system`,
/// `row.display` fields. Empty rows are dropped (§7.6 F1 bullet).
fn collect_batch_rows(form: &HashMap<String, Vec<String>>) -> Vec<BatchRowInput> {
    let codes = raw_multi(form, "row.code");
    let systems = raw_multi(form, "row.system");
    let displays = raw_multi(form, "row.display");
    let n = codes.len().max(systems.len()).max(displays.len());
    let mut out = Vec::new();
    for i in 0..n {
        let code = codes.get(i).cloned().unwrap_or_default();
        let system = systems.get(i).cloned().unwrap_or_default();
        let display = displays.get(i).cloned().unwrap_or_default();
        if code.trim().is_empty()
            && system.trim().is_empty()
            && display.trim().is_empty()
        {
            continue;
        }
        out.push(BatchRowInput {
            code,
            system,
            display,
        });
    }
    out
}

/// Minimal HTML escape used only by the nojs batch table composer,
/// where the enclosing template can't be reused because the batch
/// rows partial re-emits its own `<tr>` wrapper.
fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
