use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use helios_cds_hooks::{
    CdsHooksError, CdsRequest, CdsResponse, DiscoveryResponse, FeedbackRequest,
};
use http::{HeaderValue, Method};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use crate::auth::{Authenticator, ClientContext, Refusal};
use crate::catalog::ServiceCatalog;
use crate::clients::ClientRegistry;
use crate::galleri_service::GalleriCoverageService;

/// Everything a request needs: the services on offer and who may call them.
pub struct AppState {
    pub catalog: ServiceCatalog,
    pub auth: Authenticator,
}

impl AppState {
    pub fn new(catalog: ServiceCatalog, registry: ClientRegistry) -> Self {
        for (client, service) in registry.unknown_services(&catalog.ids()) {
            warn!(
                client,
                service, "client enables a service this server does not offer"
            );
        }
        if let Some(anon) = &registry.anonymous_client {
            warn!(client = %anon, "calls without a token are accepted as this client; development only");
        }
        Self {
            catalog,
            auth: Authenticator::new(Arc::new(registry)),
        }
    }
}

/// Server configuration via environment variables.
pub struct ServerConfig {
    pub port: u16,
    pub host: String,
    /// The client registry file (`CDS_HOOKS_CLIENTS`); none means the development registry.
    pub clients: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: std::env::var("CDS_HOOKS_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8787),
            host: std::env::var("CDS_HOOKS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            clients: std::env::var_os("CDS_HOOKS_CLIENTS").map(PathBuf::from),
        }
    }
}

/// The services this binary offers.
pub fn catalog() -> ServiceCatalog {
    ServiceCatalog::new().with(GalleriCoverageService)
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
        .route("/cds-services/{id}", post(invoke))
        .route("/cds-services/{id}/feedback", post(feedback))
        .route("/health", get(health_check))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the CDS Hooks server.
pub async fn run_server(config: ServerConfig) -> Result<(), Box<dyn std::error::Error>> {
    let catalog = catalog();
    let registry = match &config.clients {
        Some(path) => {
            info!(path = %path.display(), "loading the client registry");
            ClientRegistry::load(path)?
        }
        None => {
            warn!("CDS_HOOKS_CLIENTS is not set: running with the development registry");
            ClientRegistry::development(&catalog.ids())
        }
    };
    let state = Arc::new(AppState::new(catalog, registry));

    let app = build_router(state);

    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    info!("CDS Hooks server listening on {}", addr);
    info!("Discovery endpoint: http://{}/cds-services", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// --- Handlers ---

/// GET /cds-services — the caller's own catalog: its enabled services, its prefetch.
async fn discovery(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<DiscoveryResponse>, Reply> {
    let ctx = state.auth.authenticate(&headers).await?;
    let services = state.catalog.definitions_for(&ctx.client);
    info!(client = %ctx.client.id, anonymous = ctx.anonymous, services = services.len(), "discovery");
    Ok(Json(DiscoveryResponse { services }))
}

/// POST /cds-services/{id} — a hook fired; only the caller's enabled services answer.
async fn invoke(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CdsRequest>,
) -> Result<Json<CdsResponse>, Reply> {
    let ctx = state.auth.authenticate(&headers).await?;
    Authenticator::check_fhir_server(&ctx, request.fhir_server.as_deref())?;
    let service = state
        .catalog
        .for_client(&ctx.client, &id)
        .ok_or_else(|| Reply::not_found(&ctx, &id))?;
    let definition = service.definition();
    if definition.hook != request.hook {
        return Err(Reply::bad_request(format!(
            "service {} answers the {} hook, not {}",
            id, definition.hook, request.hook
        )));
    }
    let response = service.invoke(&request).await.map_err(Reply::from)?;
    info!(
        client = %ctx.client.id,
        service = %id,
        hook_instance = %request.hook_instance,
        cards = response.cards.len(),
        "hook answered"
    );
    Ok(Json(response))
}

/// POST /cds-services/{id}/feedback — what the clinician did with the cards.
async fn feedback(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(feedback): Json<FeedbackRequest>,
) -> Result<StatusCode, Reply> {
    let ctx = state.auth.authenticate(&headers).await?;
    let service = state
        .catalog
        .for_client(&ctx.client, &id)
        .ok_or_else(|| Reply::not_found(&ctx, &id))?;
    service.feedback(&feedback).await.map_err(Reply::from)?;
    info!(client = %ctx.client.id, service = %id, cards = feedback.feedback.len(), "feedback");
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

/// A refusal as the wire sees it: a status, one sentence, nothing from the request.
struct Reply {
    status: StatusCode,
    message: String,
}

impl Reply {
    fn not_found(ctx: &ClientContext, id: &str) -> Self {
        info!(client = %ctx.client.id, service = %id, "service not available to this client");
        Self {
            status: StatusCode::NOT_FOUND,
            message: format!("no service {id}"),
        }
    }

    fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }
}

impl From<Refusal> for Reply {
    fn from(refusal: Refusal) -> Self {
        let status = match refusal {
            Refusal::ForeignFhirServer => StatusCode::FORBIDDEN,
            _ => StatusCode::UNAUTHORIZED,
        };
        Self {
            status,
            message: refusal.to_string(),
        }
    }
}

impl From<CdsHooksError> for Reply {
    fn from(error: CdsHooksError) -> Self {
        Self {
            status: StatusCode::from_u16(error.status_code())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            message: error.to_string(),
        }
    }
}

impl IntoResponse for Reply {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.message });
        let mut response = (self.status, Json(body)).into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                http::header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer"),
            );
        }
        response
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
        let catalog = catalog();
        let registry = ClientRegistry::development(&catalog.ids());
        Arc::new(AppState::new(catalog, registry))
    }

    pub(crate) fn galleri_request() -> serde_json::Value {
        serde_json::json!({
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

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/cds-services/bcbsm-galleri-crd")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_vec(&galleri_request()).unwrap()))
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
    async fn an_unknown_service_and_a_wrong_hook_are_refused() {
        let app = build_router(test_state());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/cds-services/nope")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_vec(&galleri_request()).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let mut wrong = galleri_request();
        wrong["hook"] = serde_json::json!("patient-view");
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/cds-services/bcbsm-galleri-crd")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_vec(&wrong).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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
