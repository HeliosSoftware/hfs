//! #1065: while a search-index rebuild runs for the tenant, Home and Resources
//! say so — stored resources stay readable by id, but searches can miss them
//! until it finishes — and neither page shows the line otherwise.
//!
//! #1125: when the tenant's last rebuild left resources unindexed, the line
//! stays — naming how many and the `$reindex-status` job that lists them —
//! instead of vanishing the moment the job stops running, in every locale, and
//! a clean retry of the resources one of them named clears the line only when
//! nothing permanent is left behind it.
//!
//! #1082 stopped the rail showing placeholder zeros during a rebuild; this is
//! the other half it left: the explicit "search index rebuilding" line.
//!
//! Registers its own [`DashboardProvider`] (process-global state), so it lives
//! in its own test binary, and each case uses a tenant of its own so the
//! snapshot cache never mixes them.

use axum::{
    Router,
    body::Body,
    http::{Request, header},
};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

use helios_observability::dashboard::{
    DashboardProvider, DashboardSnapshot, DashboardWindow, Figures, ReindexActivity, TypeCount,
    set_provider,
};
use helios_persistence::search::{ReindexProgress, ReindexStatus, reindex::ReindexProgressError};

#[path = "support/html.rs"]
mod html;
use html::Dom;

/// A tenant whose rebuild has counted its resources: 8,000 of 19,000.
const REBUILDING_TENANT: &str = "rebuild-running";
/// A tenant whose rebuild is still counting what it will process.
const COUNTING_TENANT: &str = "rebuild-counting";
/// A tenant with no rebuild running.
const IDLE_TENANT: &str = "rebuild-idle";
/// A tenant whose last rebuild completed with 11,704 resources unindexed.
const FAILED_TENANT: &str = "rebuild-failed";
/// A tenant whose last rebuild failed as a whole, naming no resource.
const FAILED_JOB_TENANT: &str = "rebuild-failed-job";
/// The failed rebuilds' job id.
const FAILED_JOB_ID: &str = "job-1125";
/// A tenant whose last full rebuild completed with one retryable and one
/// permanent per-resource error, and whose resource-scoped retry of the
/// retryable one completed cleanly afterwards.
const PERMANENT_ERRORS_TENANT: &str = "rebuild-permanent";
/// The same history, except every error of that rebuild was retryable — so the
/// clean retry really did leave nothing unindexed.
const RETRIED_CLEAN_TENANT: &str = "rebuild-retried-clean";
/// The job id of the full rebuild both of those retries follow.
const GENERATION_JOB_ID: &str = "job-1125-generation";

const REBUILD_LINE: &str = "[data-rebuild-notice]";

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

async fn get_as(tenant: &str, path: &str) -> Dom {
    let response = app_as(tenant)
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Dom::page(std::str::from_utf8(&bytes).unwrap())
}

/// The line's text as read, without Fluent's bidi isolation marks around
/// interpolated values.
fn rebuild_line(dom: &Dom) -> String {
    dom.one(REBUILD_LINE)
        .text()
        .replace(['\u{2068}', '\u{2069}'], "")
}

/// The same page as [`get_as`], asked for in `lang` — the Accept-Language
/// negotiation `tests/i18n_http.rs` drives the UI language with.
async fn get_as_in(tenant: &str, path: &str, lang: &str) -> Dom {
    let response = app_as(tenant)
        .oneshot(
            Request::get(path)
                .header(header::ACCEPT_LANGUAGE, lang)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Dom::page(std::str::from_utf8(&bytes).unwrap())
}

/// One resource a rebuild left unindexed, transiently (`retryable`) or for
/// good — a document the search backend rejects however often it is resent.
fn unindexed(id: &str, retryable: bool) -> ReindexProgressError {
    ReindexProgressError {
        resource_type: "Provenance".to_string(),
        resource_id: id.to_string(),
        error: "backend unavailable: elasticsearch".to_string(),
        retryable,
    }
}

/// A tenant's finished rebuilds after a retry: the full rebuild
/// ([`GENERATION_JOB_ID`]), which completed with per-resource errors, and the
/// later `resource_scoped` retry of its retryable ones, which completed
/// cleanly. With `permanent`, one of the full rebuild's two errors is a
/// permanent rejection the retry could not have fixed.
fn generation_then_clean_retry(tenant: &str, permanent: bool) -> Vec<ReindexProgress> {
    let finished = |job: &mut ReindexProgress, minutes_ago: i64| {
        job.tenant_id = Some(tenant.to_string());
        job.status = ReindexStatus::Completed;
        job.completed_at =
            Some((chrono::Utc::now() - chrono::Duration::minutes(minutes_ago)).to_rfc3339());
    };
    let mut generation = ReindexProgress::new(GENERATION_JOB_ID);
    finished(&mut generation, 5);
    generation.errors = vec![unindexed("p1", true)];
    if permanent {
        generation.errors.push(unindexed("p2", false));
    }
    let mut retry = ReindexProgress::new("job-1125-retry");
    finished(&mut retry, 1);
    retry.resource_scoped = true;
    vec![generation, retry]
}

/// The banner the server shows for a tenant whose rebuild jobs are `jobs`,
/// once a `resource_scoped` retry among them has completed cleanly.
///
/// `helios_rest::dashboard::reindex_activity_of` makes that call on the live
/// server and its own unit tests pin it; this crate cannot call it (the UI
/// depends on the rest of the workspace, never the reverse), so the two facts
/// it turns on are read back off the same fixture here — after a clean
/// resource-scoped retry only the earlier rebuild's *permanent* errors are
/// still unindexed, and it is that earlier job the banner points at.
fn banner_after_clean_retry(jobs: &[ReindexProgress]) -> Option<ReindexActivity> {
    let generation = jobs
        .iter()
        .find(|job| !job.resource_scoped)
        .expect("the fixture's full rebuild");
    let permanent = generation
        .errors
        .iter()
        .filter(|error| !error.retryable)
        .count() as u64;
    (permanent > 0).then(|| ReindexActivity::Failed {
        job_id: generation.job_id.clone(),
        errors: permanent,
    })
}

struct Rebuilding;

#[async_trait::async_trait]
impl DashboardProvider for Rebuilding {
    async fn snapshot(
        &self,
        _window: DashboardWindow,
        tenant: &str,
        _types: &[String],
        _include_empty: bool,
    ) -> DashboardSnapshot {
        DashboardSnapshot {
            available: vec![TypeCount {
                resource_type: "Patient".into(),
                total: 42,
            }],
            figures: Figures::Exact {
                read_at: chrono::Utc::now(),
            },
            reindex_active: match tenant {
                REBUILDING_TENANT => Some(ReindexActivity::Running {
                    jobs: 1,
                    processed: 8_000,
                    total: 19_000,
                }),
                COUNTING_TENANT => Some(ReindexActivity::Running {
                    jobs: 1,
                    processed: 0,
                    total: 0,
                }),
                FAILED_TENANT => Some(ReindexActivity::Failed {
                    job_id: FAILED_JOB_ID.to_string(),
                    errors: 11_704,
                }),
                FAILED_JOB_TENANT => Some(ReindexActivity::Failed {
                    job_id: FAILED_JOB_ID.to_string(),
                    errors: 0,
                }),
                PERMANENT_ERRORS_TENANT => {
                    banner_after_clean_retry(&generation_then_clean_retry(tenant, true))
                }
                RETRIED_CLEAN_TENANT => {
                    banner_after_clean_retry(&generation_then_clean_retry(tenant, false))
                }
                _ => None,
            },
            ..Default::default()
        }
    }
}

#[tokio::test]
async fn home_and_resources_say_when_the_search_index_is_rebuilding() {
    set_provider(Arc::new(Rebuilding));

    for path in ["/ui", "/ui/resources"] {
        let dom = get_as(REBUILDING_TENANT, path).await;
        let line = rebuild_line(&dom);
        assert!(
            line.contains("Search index rebuilding — 42% (8,000 of 19,000 resources)"),
            "{path}: the line names the rebuild and its progress: {line:?}"
        );
        assert!(
            line.contains("Searches may miss stored resources until it finishes."),
            "{path}: and what it means for search: {line:?}"
        );
        assert!(
            dom.one(REBUILD_LINE)
                .attr("class")
                .is_some_and(|class| class.contains("notice--warn")),
            "{path}: it is a warning, not a label"
        );

        // Before the rebuild has counted its resources: no percentage at all,
        // never a fabricated "0%".
        let dom = get_as(COUNTING_TENANT, path).await;
        let line = rebuild_line(&dom);
        assert!(
            line.contains("Search index rebuilding.") && !line.contains('%'),
            "{path}: {line:?}"
        );

        let dom = get_as(IDLE_TENANT, path).await;
        assert_eq!(dom.count(REBUILD_LINE), 0, "{path}: no rebuild, no line");
    }

    // On Home the line sits in the live region, and a running rebuild keeps
    // that region refreshing at the fast cadence so its percentage moves.
    let running = get_as(REBUILDING_TENANT, "/ui").await;
    let live = running.one("#dash-live");
    assert_eq!(
        live.one(REBUILD_LINE).attr("data-dash-notice"),
        Some("rebuilding")
    );
    assert_eq!(live.attr("data-dash-moving"), Some("1"));
    let idle = get_as(IDLE_TENANT, "/ui").await;
    assert_eq!(idle.one("#dash-live").attr("data-dash-moving"), None);
}

#[tokio::test]
async fn home_and_resources_keep_saying_when_the_last_rebuild_left_resources_unindexed() {
    set_provider(Arc::new(Rebuilding));

    for path in ["/ui", "/ui/resources"] {
        let dom = get_as(FAILED_TENANT, path).await;
        let line = rebuild_line(&dom);
        assert!(
            line.contains("The last search index rebuild left 11,704 resources unindexed."),
            "{path}: the line says the rebuild ended incomplete, and by how much: {line:?}"
        );
        assert!(
            line.contains("$reindex-status/job-1125"),
            "{path}: and where to find which resources: {line:?}"
        );
        assert!(
            !line.contains('%'),
            "{path}: a failed rebuild has no progress"
        );
        let notice = dom.one(REBUILD_LINE);
        assert!(
            notice
                .attr("class")
                .is_some_and(|class| class.contains("notice--warn")),
            "{path}: it is a warning"
        );
        assert_eq!(notice.attr("data-rebuild-state"), Some("failed"), "{path}");

        // A job that failed as a whole has no resource count to give.
        let dom = get_as(FAILED_JOB_TENANT, path).await;
        let line = rebuild_line(&dom);
        assert!(
            line.contains("The last search index rebuild failed before it finished.")
                && line.contains("$reindex-status/job-1125")
                && !line.contains("0 resources"),
            "{path}: {line:?}"
        );

        let dom = get_as(REBUILDING_TENANT, path).await;
        assert_eq!(
            dom.one(REBUILD_LINE).attr("data-rebuild-state"),
            Some("running"),
            "{path}"
        );
    }

    // On Home the failed line has its own notice slug, so the switch from
    // "rebuilding" is announced, and it does not keep the region on the fast
    // cadence: nothing is moving any more.
    let failed = get_as(FAILED_TENANT, "/ui").await;
    let live = failed.one("#dash-live");
    let line = live.one(REBUILD_LINE);
    assert_eq!(line.attr("data-dash-notice"), Some("rebuild-failed"));
    assert_eq!(line.attr("aria-live"), Some("polite"));
    assert_eq!(live.attr("data-dash-moving"), None);
}

#[tokio::test]
async fn the_unindexed_line_is_localized() {
    set_provider(Arc::new(Rebuilding));

    // The `search-index-rebuild-failed` / `-failed-job` messages of
    // locales/de/main.ftl and locales/es/main.ftl, with the count and the job
    // id filled in. Whole sentences, so a translation that dropped a slot or
    // an interpolation fails here rather than reading as a partial match.
    //
    // The count is grouped for the locale ("11.704" in German and Spanish,
    // "11,704" in English), and the catalog selects the plural form, so a
    // single unindexed resource reads "one resource" rather than
    // "1 resources" (#1125). See
    // `a_clean_resource_scoped_retry_does_not_hide_the_permanent_failures`.
    for (lang, left_unindexed, failed_outright) in [
        (
            "de",
            "Der letzte Neuaufbau des Suchindex hat 11.704 Ressourcen nicht indiziert. \
             Suchen finden sie erst nach einem erfolgreichen Neuaufbau; \
             GET $reindex-status/job-1125 nennt die betroffenen Ressourcen.",
            "Der letzte Neuaufbau des Suchindex ist vor dem Abschluss fehlgeschlagen. \
             Suchen können gespeicherte Ressourcen übersehen, bis ein Neuaufbau erfolgreich ist; \
             GET $reindex-status/job-1125 nennt den Grund.",
        ),
        (
            "es",
            "La última reconstrucción del índice de búsqueda dejó 11.704 recursos sin indexar. \
             Las búsquedas no los encuentran hasta que una reconstrucción termine bien; \
             GET $reindex-status/job-1125 indica cuáles.",
            "La última reconstrucción del índice de búsqueda falló antes de terminar. \
             Las búsquedas pueden omitir recursos almacenados hasta que una reconstrucción \
             termine bien; GET $reindex-status/job-1125 indica el motivo.",
        ),
    ] {
        for path in ["/ui", "/ui/resources"] {
            let dom = get_as_in(FAILED_TENANT, path, lang).await;
            assert_eq!(
                dom.one("html").attr("lang"),
                Some(lang),
                "{path}: the {lang} request must be answered in {lang}"
            );
            assert_eq!(
                rebuild_line(&dom),
                left_unindexed,
                "{path}: the {lang} line names the count and the job"
            );
            assert!(
                dom.one(REBUILD_LINE)
                    .attr("class")
                    .is_some_and(|class| class.contains("notice--warn")),
                "{path}: it is a warning in {lang} too"
            );

            let dom = get_as_in(FAILED_JOB_TENANT, path, lang).await;
            assert_eq!(
                rebuild_line(&dom),
                failed_outright,
                "{path}: a job that failed as a whole has no count to give in {lang} either"
            );
        }
    }
}

#[tokio::test]
async fn a_clean_resource_scoped_retry_does_not_hide_the_permanent_failures() {
    set_provider(Arc::new(Rebuilding));

    for path in ["/ui", "/ui/resources"] {
        // The retry re-indexed p1 and stopped there; p2 was rejected outright
        // and is still missing from the index, so the line stays — counting
        // only p2, and still pointing at the rebuild that hit it.
        let dom = get_as_in(PERMANENT_ERRORS_TENANT, path, "en").await;
        let line = rebuild_line(&dom);
        assert_eq!(
            line,
            "The last search index rebuild left one resource unindexed. \
             Searches miss it until a rebuild succeeds; \
             GET $reindex-status/job-1125-generation says which one.",
            "{path}: the clean retry must not hide the permanent failure"
        );
        assert_eq!(
            dom.one(REBUILD_LINE).attr("data-rebuild-state"),
            Some("failed"),
            "{path}"
        );

        // With nothing permanent left, the same clean retry clears the line.
        let dom = get_as_in(RETRIED_CLEAN_TENANT, path, "en").await;
        assert_eq!(
            dom.count(REBUILD_LINE),
            0,
            "{path}: a retry that fixed every failure leaves nothing to say"
        );
    }
}
