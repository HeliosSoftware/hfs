//! End-to-end locale negotiation through the mounted router: the same
//! requests a browser would make, exercising the middleware, the handlers,
//! and the templates together.

use axum::{
    Router,
    body::Body,
    http::{Request, header},
};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "support/html.rs"]
mod html;

fn app() -> Router {
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch {
            enabled: true,
            configured: true,
            model: "test-model".to_string(),
        },
        None,
        None,
        "default".to_string(),
        std::sync::Arc::new(helios_ui::StaticConformanceSource::empty()),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

fn resources_app() -> Router {
    let source = helios_ui::StaticConformanceSource::empty().with_metadata(serde_json::json!({
        "resourceType": "CapabilityStatement",
        "fhirVersion": "4.0.1",
        "rest": [{"mode": "server", "resource": [{
            "type": "Patient",
            "interaction": [{"code": "create"}]
        }]}]
    }));
    helios_ui::mount_with_conformance_source(
        Router::new(),
        "9.9.9",
        None,
        helios_ui::NlSearch {
            enabled: true,
            configured: true,
            model: "test-model".to_string(),
        },
        None,
        None,
        "default".to_string(),
        std::sync::Arc::new(source),
        helios_fhir::FhirVersion::R4,
        None,
        "http://localhost:8080".to_string(),
        None,
    )
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn accept_language_selects_the_ui_language() {
    let response = app()
        .oneshot(
            Request::get("/ui")
                .header(header::ACCEPT_LANGUAGE, "de-DE, de;q=0.9, en;q=0.7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.headers().get(header::SET_COOKIE).is_none());
    let vary: Vec<_> = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect();
    assert!(
        vary.iter().any(|v| v.contains("Accept-Language")),
        "language-dependent responses must vary on Accept-Language, got {vary:?}"
    );
    let html = body_text(response).await;
    assert!(html.contains(r#"<html lang="de">"#));
    assert!(html.contains("Startseite"));
}

#[tokio::test]
async fn lang_override_wins_and_persists_in_a_cookie() {
    let response = app()
        .oneshot(
            Request::get("/ui?lang=es")
                .header(header::ACCEPT_LANGUAGE, "de")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("explicit ?lang= must set the hfs_lang cookie")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.starts_with("hfs_lang=es"));

    let html = body_text(response).await;
    assert!(html.contains(r#"<html lang="es">"#));
    assert!(html.contains("Inicio"));
}

#[tokio::test]
async fn cookie_keeps_the_choice_on_later_requests() {
    let response = app()
        .oneshot(
            Request::get("/ui")
                .header(header::COOKIE, "hfs_lang=es")
                .header(header::ACCEPT_LANGUAGE, "de")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let html = body_text(response).await;
    assert!(html.contains(r#"<html lang="es">"#));
}

#[tokio::test]
async fn htmx_fragment_is_localized_too() {
    let response = app()
        .oneshot(
            Request::get("/ui/status")
                .header("HX-Request", "true")
                .header(header::ACCEPT_LANGUAGE, "es")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let html = body_text(response).await;
    assert!(html.contains("Última comprobación:"));
    assert!(!html.contains("<html"), "fragment, not a full page");
}

#[tokio::test]
async fn resources_create_label_names_the_selected_type_per_locale() {
    // #605: "Create new { $type }" carries the resource type name untranslated
    // inside the localized sentence, in every supported locale.
    for (lang, sentence) in [
        ("en", "Create new Patient"),
        ("es", "Crear Patient"),
        ("de", "Patient erstellen"),
    ] {
        let response = resources_app()
            .oneshot(
                Request::get(format!("/ui/resources?lang={lang}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let html = body_text(response).await;
        assert!(
            html.contains(sentence),
            "{lang} resources page must contain {sentence:?}, got: {html}"
        );
    }
}

#[tokio::test]
async fn pagination_failure_message_is_localized_with_the_public_origin_slot() {
    for (lang, sentence) in [
        (
            "en",
            "Could not load results from {origin}. Check HFS_BASE_URL and try again.",
        ),
        (
            "es",
            "No se pudieron cargar los resultados desde {origin}. Revise HFS_BASE_URL e inténtelo de nuevo.",
        ),
        (
            "de",
            "Ergebnisse von {origin} konnten nicht geladen werden. Prüfen Sie HFS_BASE_URL und versuchen Sie es erneut.",
        ),
    ] {
        let response = app()
            .oneshot(
                Request::get(format!("/ui/search?lang={lang}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let html = body_text(response).await;
        assert!(
            html.contains(sentence),
            "{lang} search page must expose the localized pagination error"
        );
    }
}

#[tokio::test]
async fn resources_create_block_reason_is_localized() {
    for (lang, sentence) in [
        ("en", "not available in the selected FHIR version"),
        ("es", "no está disponible en la versión FHIR seleccionada"),
        ("de", "ist in der ausgewählten FHIR-Version nicht verfügbar"),
    ] {
        let response = resources_app()
            .oneshot(
                Request::get(format!("/ui/resources?lang={lang}&type=patient"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let html = body_text(response).await;
        assert!(html.contains(sentence), "missing {lang} reason: {html}");
        assert!(html.contains(r#"data-create-eligible="false""#));
    }
}

#[tokio::test]
async fn default_is_english() {
    let response = app()
        .oneshot(Request::get("/ui").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let html = body_text(response).await;
    assert!(html.contains(r#"<html lang="en">"#));
    assert!(html.contains(">Home<"));
}

#[tokio::test]
async fn export_pages_pluralize_zero_one_and_two_exports_and_files_in_all_locales() {
    use helios_persistence::{backends::sqlite::SqliteBackend, core::SettingsStore};
    use std::sync::Arc;
    for (lang, exports, files, running) in [
        (
            "en",
            ["0 exports", "1 export", "2 exports"],
            ["0 files", "1 file", "2 files"],
            "running",
        ),
        (
            "es",
            ["0 exportaciones", "1 exportación", "2 exportaciones"],
            ["0 archivos", "1 archivo", "2 archivos"],
            "en curso",
        ),
        (
            "de",
            ["0 Exporte", "1 Export", "2 Exporte"],
            ["0 Dateien", "1 Datei", "2 Dateien"],
            "laufend",
        ),
    ] {
        for count in 0..=2usize {
            let backend = Arc::new(SqliteBackend::in_memory().unwrap());
            backend.init_schema().unwrap();
            let mut jobs = serde_json::Map::new();
            for id in 0..count {
                jobs.insert(format!("job-{id}"), serde_json::json!({
                    "name": "Localized export", "status": "complete",
                    "files": (0..count).map(|_| serde_json::json!({"type": "Patient", "url": "http://localhost:8080/file"})).collect::<Vec<_>>()
                }));
            }
            backend
                .put_settings(
                    "l2:",
                    serde_json::json!({"byTenant": {"default": {
                        "bulkExport": {"jobs": jobs.clone()}, "sqlExport": {"jobs": jobs}
                    }}}),
                    None,
                )
                .await
                .unwrap();
            let app = helios_ui::mount_with_conformance_source(
                Router::new(),
                "9.9.9",
                None,
                helios_ui::NlSearch::default(),
                None,
                Some(backend),
                "default".to_string(),
                Arc::new(helios_ui::StaticConformanceSource::empty()),
                helios_fhir::FhirVersion::R4,
                None,
                "http://localhost:8080".to_string(),
                None,
            );
            for path in ["/ui/bulk-export", "/ui/sql/export"] {
                let response = app
                    .clone()
                    .oneshot(
                        Request::get(format!("{path}?lang={lang}"))
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), axum::http::StatusCode::OK);
                let html = body_text(response).await;
                let dom = html::Dom::page(&html);
                let summary = dom.one(if path == "/ui/bulk-export" {
                    "#bulk-export-summary"
                } else {
                    "#sql-export-summary"
                });
                assert_eq!(summary.text(), format!("{} · 0 {running}", exports[count]));
                assert!(!summary.has_attr("hx-swap-oob"));
                if path == "/ui/bulk-export" && count > 0 {
                    assert_eq!(dom.all(".job-card__meta")[0].text(), files[count]);
                }
            }
        }
    }
}

#[tokio::test]
async fn unsaved_navigation_copy_is_localized_in_the_shared_layout() {
    for (lang, message, action) in [
        (
            "en",
            "You have unsaved changes. Discard them and leave this page?",
            "Discard and leave",
        ),
        (
            "es",
            "Hay cambios sin guardar. ¿Descartarlos y salir de esta página?",
            "Descartar y salir",
        ),
        (
            "de",
            "Es gibt ungespeicherte Änderungen. Verwerfen und diese Seite verlassen?",
            "Verwerfen und verlassen",
        ),
    ] {
        let response = app()
            .oneshot(
                Request::get(format!("/ui?lang={lang}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = body_text(response).await;
        let dom = html::Dom::page(&body);
        let body = dom.one("body");
        assert_eq!(body.attr("data-msg-unsaved-leave"), Some(message));
        assert_eq!(body.attr("data-msg-unsaved-leave-action"), Some(action));
    }
}
