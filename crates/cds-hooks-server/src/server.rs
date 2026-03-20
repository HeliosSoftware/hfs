use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use helios_cds_hooks::{
    CdsHooksError, CdsHooksService, CdsRequest, CdsResponse, DiscoveryResponse, FeedbackRequest,
    OrderSelectContext,
};
use http::{HeaderValue, Method};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::galleri_service::GalleriCoverageService;

/// Shared application state holding all registered CDS services.
pub struct AppState {
    pub galleri_service: GalleriCoverageService,
}

/// Server configuration via environment variables.
pub struct ServerConfig {
    pub port: u16,
    pub host: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: std::env::var("CDS_HOOKS_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8787),
            host: std::env::var("CDS_HOOKS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
        }
    }
}

/// Build the Axum router with all CDS Hooks endpoints.
pub fn build_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin("*".parse::<HeaderValue>().unwrap())
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::ACCEPT,
            http::header::AUTHORIZATION,
        ]);

    Router::new()
        .route("/cds-services", get(discovery))
        .route(
            "/cds-services/bcbsm-galleri-crd",
            post(invoke_galleri_service),
        )
        .route(
            "/cds-services/bcbsm-galleri-crd/feedback",
            post(handle_feedback),
        )
        .route("/health", get(health_check))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the CDS Hooks server.
pub async fn run_server(config: ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AppState {
        galleri_service: GalleriCoverageService,
    });

    let app = build_router(state);

    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    info!("CDS Hooks server listening on {}", addr);
    info!("Discovery endpoint: http://{}/cds-services", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// --- Handlers ---

/// GET /cds-services — Discovery endpoint
async fn discovery(State(state): State<Arc<AppState>>) -> Json<DiscoveryResponse> {
    let services = vec![state.galleri_service.definition()];
    Json(DiscoveryResponse { services })
}

/// POST /cds-services/bcbsm-galleri-crd — Service invocation
async fn invoke_galleri_service(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CdsRequest>,
) -> Result<Json<CdsResponse>, CdsHooksErrorResponse> {
    let context: OrderSelectContext = state
        .galleri_service
        .extract_context(&request)
        .map_err(CdsHooksErrorResponse)?;

    let response = state
        .galleri_service
        .call(&request, &context)
        .await
        .map_err(CdsHooksErrorResponse)?;

    Ok(Json(response))
}

/// POST /cds-services/bcbsm-galleri-crd/feedback — Feedback endpoint
async fn handle_feedback(
    State(state): State<Arc<AppState>>,
    Json(feedback): Json<FeedbackRequest>,
) -> Result<StatusCode, CdsHooksErrorResponse> {
    state
        .galleri_service
        .on_feedback(&feedback)
        .await
        .map_err(CdsHooksErrorResponse)?;

    Ok(StatusCode::OK)
}

/// GET /health — Health check
async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "cds-hooks-server",
    }))
}

// --- Error handling ---

struct CdsHooksErrorResponse(CdsHooksError);

impl IntoResponse for CdsHooksErrorResponse {
    fn into_response(self) -> axum::response::Response {
        let status =
            StatusCode::from_u16(self.0.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::json!({
            "error": self.0.to_string(),
        });
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use helios_cds_hooks::Indicator;
    use http::Request;
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState {
            galleri_service: GalleriCoverageService,
        })
    }

    #[tokio::test]
    async fn test_discovery_endpoint() {
        let app = build_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/cds-services")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let discovery: DiscoveryResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(discovery.services.len(), 1);
        assert_eq!(discovery.services[0].id, "bcbsm-galleri-crd");
        assert_eq!(discovery.services[0].hook, "order-select");
    }

    #[tokio::test]
    async fn test_galleri_service_invocation() {
        let app = build_router(test_state());

        let request_body = serde_json::json!({
            "hook": "order-select",
            "hookInstance": "d1577c69-dfbe-44ad-bd63-8c585e40b1a2",
            "context": {
                "userId": "Practitioner/pract-123",
                "patientId": "pat-456",
                "selections": ["ServiceRequest/sr-galleri-001"],
                "draftOrders": {
                    "resourceType": "Bundle",
                    "type": "collection",
                    "entry": [{
                        "resource": {
                            "resourceType": "ServiceRequest",
                            "id": "sr-galleri-001",
                            "status": "draft",
                            "intent": "proposal",
                            "subject": { "reference": "Patient/pat-456" },
                            "requester": { "reference": "Practitioner/pract-123" },
                            "code": {
                                "coding": [
                                    {
                                        "system": "http://www.ama-assn.org/go/cpt",
                                        "code": "81479",
                                        "display": "Unlisted molecular pathology procedure"
                                    },
                                    {
                                        "system": "https://www.grail.com/tests",
                                        "code": "GALLERI",
                                        "display": "Galleri Multi-Cancer Early Detection Test"
                                    }
                                ],
                                "text": "Galleri (Multicancer Early Detection Test)"
                            },
                            "occurrenceDateTime": "2026-03-13"
                        }
                    }]
                }
            }
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/cds-services/bcbsm-galleri-crd")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let cds_response: CdsResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(cds_response.cards.len(), 1);
        assert_eq!(cds_response.cards[0].indicator, Indicator::Critical);
        assert!(cds_response.cards[0].summary.contains("Not Covered"));
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let app = build_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
