//! `/ws/subscriptions/bind` reads one short `bind-with-token <token>` message.
//! A message larger than the route's limit is refused by the WebSocket layer
//! before the server reads it as a bind message, rather than buffered up to
//! the library's 64 MiB default, and the peer is told why with close code
//! 1009.
#![cfg(feature = "subscriptions")]

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::{SinkExt, StreamExt};
use helios_fhir::FhirVersion;
use helios_persistence::backends::sqlite::SqliteBackend;
use helios_persistence::tenant::TenantId;
use helios_rest::{AppState, ServerConfig};
use helios_subscriptions::{
    ResourceEvent, ResourceEventType, SubscriptionConfig, SubscriptionEngine,
};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::Frame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::{Data, OpCode};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The tenant `ServerConfig::for_testing()` resolves requests to.
const TENANT_ID: &str = "test-tenant";
const TOPIC_URL: &str = "http://example.org/topic/encounter-create";
const SUBSCRIPTION_ID: &str = "sub-ws-1";

/// Close code and reason of the frame that ended a connection.
type Closed = Option<(u16, String)>;

/// Serves the FHIR routes with a subscription engine on a loopback port.
async fn serve() -> (std::net::SocketAddr, Arc<SubscriptionEngine>) {
    let backend = Arc::new(SqliteBackend::in_memory().expect("in-memory sqlite"));
    backend.init_schema().expect("init schema");
    let config = SubscriptionConfig {
        supported_channel_types: vec!["websocket".to_string()],
        ..Default::default()
    };
    let engine = Arc::new(SubscriptionEngine::new(
        config,
        "http://localhost".to_string(),
    ));
    let state = AppState::new(backend, ServerConfig::for_testing())
        .with_subscription_engine(Arc::clone(&engine));
    let app = helios_rest::routing::fhir_routes::create_routes(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (addr, engine)
}

async fn connect(addr: std::net::SocketAddr) -> Client {
    let (socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/ws/subscriptions/bind"))
            .await
            .expect("upgrade");
    socket
}

/// Reads until the server closes the connection and returns the code and
/// reason of its close frame, or `None` if it ended without one. Frames that
/// are not a close (the handshake notification) are skipped.
async fn closed(socket: &mut Client) -> Closed {
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = socket.next().await {
            match frame {
                Ok(Message::Close(Some(close))) => {
                    return Some((close.code.into(), close.reason.to_string()));
                }
                Ok(Message::Close(None)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
        None
    })
    .await
    .expect("the server ends the connection")
}

fn too_big() -> Closed {
    Some((1009, "message-too-big".to_string()))
}

fn event(resource_type: &str, id: &str, resource: Value) -> ResourceEvent {
    ResourceEvent {
        tenant_id: TenantId::new(TENANT_ID),
        fhir_version: FhirVersion::default(),
        resource_type: resource_type.to_string(),
        resource_id: id.to_string(),
        version_id: "1".to_string(),
        event_type: ResourceEventType::Create,
        resource: Some(resource),
        previous_resource: None,
        timestamp: Utc::now(),
    }
}

/// Registers a topic and a websocket Subscription on it (R4 backport shapes,
/// the default FHIR version) and returns a binding token for the Subscription.
async fn binding_token(engine: &SubscriptionEngine) -> String {
    engine
        .on_resource_event(event(
            "Basic",
            "topic-1",
            json!({
                "resourceType": "Basic",
                "id": "topic-1",
                "code": {
                    "coding": [{
                        "system": "http://hl7.org/fhir/fhir-types",
                        "code": "SubscriptionTopic"
                    }]
                },
                "extension": [{
                    "url": "http://hl7.org/fhir/5.0/StructureDefinition/extension-SubscriptionTopic.url",
                    "valueUri": TOPIC_URL
                }, {
                    "url": "http://hl7.org/fhir/4.3/StructureDefinition/extension-SubscriptionTopic.resourceTrigger",
                    "extension": [{
                        "url": "resource",
                        "valueUri": "http://hl7.org/fhir/StructureDefinition/Encounter"
                    }, {
                        "url": "supportedInteraction",
                        "valueCode": "create"
                    }]
                }]
            }),
        ))
        .await;
    engine
        .on_resource_event(event(
            "Subscription",
            SUBSCRIPTION_ID,
            json!({
                "resourceType": "Subscription",
                "id": SUBSCRIPTION_ID,
                "status": "requested",
                "criteria": TOPIC_URL,
                "channel": {
                    "type": "websocket",
                    "payload": "application/fhir+json"
                }
            }),
        ))
        .await;
    assert!(
        engine
            .manager()
            .get_subscription(TENANT_ID, SUBSCRIPTION_ID)
            .is_some(),
        "the websocket Subscription is registered"
    );
    engine
        .ws_token_manager()
        .generate_token(TENANT_ID, SUBSCRIPTION_ID)
        .0
}

#[tokio::test]
async fn an_oversized_bind_message_is_not_read_as_a_bind_message() {
    let (addr, _engine) = serve().await;

    let mut socket = connect(addr).await;
    let _ = socket
        .send(Message::Text("bind-with-token not-a-token".into()))
        .await;
    assert_eq!(
        closed(&mut socket).await,
        Some((1008, "invalid-or-expired-token".to_string())),
        "a bind message within the limit is read and its token judged"
    );

    let mut socket = connect(addr).await;
    let oversized = format!("bind-with-token {}", "a".repeat(8 * 1024));
    let _ = socket.send(Message::Text(oversized.into())).await;
    assert_eq!(
        closed(&mut socket).await,
        too_big(),
        "an 8 KiB message is refused before it is read as a bind message"
    );
}

#[tokio::test]
async fn a_message_of_small_fragments_is_held_to_the_same_limit() {
    let (addr, _engine) = serve().await;
    let mut socket = connect(addr).await;

    // Four 1.5 KiB frames: each is under the frame limit, the message is not.
    for index in 0..4 {
        let opcode = if index == 0 {
            OpCode::Data(Data::Text)
        } else {
            OpCode::Data(Data::Continue)
        };
        let fragment = Frame::message(vec![b'a'; 1500], opcode, index == 3);
        let _ = socket.send(Message::Frame(fragment)).await;
    }

    assert_eq!(
        closed(&mut socket).await,
        too_big(),
        "a 6 KiB message of 1.5 KiB fragments is refused, not read as a bind message"
    );
}

#[tokio::test]
async fn a_bound_client_is_held_to_the_limit_too() {
    let (addr, engine) = serve().await;
    let token = binding_token(&engine).await;
    let mut socket = connect(addr).await;

    socket
        .send(Message::Text(format!("bind-with-token {token}").into()))
        .await
        .expect("send bind message");
    let handshake = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("handshake arrives");
    assert!(
        matches!(handshake, Some(Ok(Message::Text(_)))),
        "the bind is accepted and answered with the handshake notification, got {handshake:?}"
    );
    assert_eq!(
        engine.ws_manager().client_count(TENANT_ID, SUBSCRIPTION_ID),
        1
    );

    // Messages within the limit are ignored after the bind, larger ones end it.
    socket
        .send(Message::Text("still here".into()))
        .await
        .expect("send small message");
    let _ = socket
        .send(Message::Text("a".repeat(8 * 1024).into()))
        .await;

    assert_eq!(closed(&mut socket).await, too_big());
}
