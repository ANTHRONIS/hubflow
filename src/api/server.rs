//! HTTP + WebSocket API server for the HubFlow frontend.
//!
//! Exposes three surfaces:
//!
//! | Surface      | Path              | Purpose                             |
//! |--------------|-------------------|-------------------------------------|
//! | REST GET     | `/api/graph`      | Full topology snapshot for editor   |
//! | REST GET     | `/api/knot/:id`   | Single Knot by UUID                 |
//! | REST GET     | `/api/knot/:id/state` | Object Knot property map        |
//! | REST POST    | `/api/packet`     | Inject a packet from the UI         |
//! | WebSocket    | `/api/ws`         | Bidirectional event stream          |
//!
//! # WebSocket protocol
//!
//! The server pushes [`EventEnvelope`] JSON objects for every [`KnotEvent`].
//! The client can send [`JsonRpcRequest`] messages; currently supported methods:
//!
//! | Method        | Params                   | Description                    |
//! |---------------|--------------------------|--------------------------------|
//! | `send_packet` | Same shape as REST body  | Inject a packet via WebSocket  |
//! | `get_graph`   | *(none)*                 | Request a topology snapshot    |

use crate::api::types::{EventEnvelope, InjectPacketRequest, JsonRpcRequest, JsonRpcResponse};
use crate::core::event::KnotEvent;
use crate::core::inbox::Inbox;
use crate::core::packet::Packet;
use crate::core::registry::KnotRegistry;
use axum::{
    Json, Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;

// ── AppState ──────────────────────────────────────────────────────────────────

/// Shared application state threaded through all axum handlers via [`State`].
///
/// Every field is cheaply cloneable (`Arc`-backed) so axum can clone the state
/// for each request without copying the underlying data.
#[derive(Clone)]
pub struct AppState {
    /// The live topology directory — used for graph and knot queries.
    pub registry: Arc<KnotRegistry>,

    /// Root inbox used for packet injection from the REST / WebSocket API.
    ///
    /// Packets pushed here enter the Hub's normal routing pipeline, so they
    /// will be forwarded down to child Knots just like any other packet.
    pub root_inbox: Arc<Inbox>,

    /// Clone of the system-wide event broadcast sender.
    ///
    /// Each WebSocket handler calls `.subscribe()` to obtain its own
    /// [`broadcast::Receiver`], allowing independent lag tracking per client.
    pub event_tx: broadcast::Sender<KnotEvent>,
}

// ── ApiServer ─────────────────────────────────────────────────────────────────

/// Async HTTP + WebSocket server exposing HubFlow internals to the Vue frontend.
///
/// # Example
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use std::net::SocketAddr;
/// use tokio::sync::broadcast;
///
/// # async fn example(
/// #     registry: Arc<hubflow::core::registry::KnotRegistry>,
/// #     root_inbox: Arc<hubflow::core::inbox::Inbox>,
/// # ) {
/// let (event_tx, _) = broadcast::channel(256);
/// let server = hubflow::api::ApiServer::new(registry, root_inbox, event_tx);
/// server.serve("127.0.0.1:3000".parse().unwrap()).await.unwrap();
/// # }
/// ```
pub struct ApiServer {
    state: AppState,
}

impl ApiServer {
    /// Creates a new [`ApiServer`].
    ///
    /// # Arguments
    ///
    /// * `registry`   — Shared topology directory.
    /// * `root_inbox` — Entry point for packet injection.
    /// * `event_tx`   — Broadcast sender for the system event bus; each
    ///                  WebSocket client subscribes independently.
    pub fn new(
        registry: Arc<KnotRegistry>,
        root_inbox: Arc<Inbox>,
        event_tx: broadcast::Sender<KnotEvent>,
    ) -> Self {
        Self {
            state: AppState {
                registry,
                root_inbox,
                event_tx,
            },
        }
    }

    /// Builds the axum [`Router`] without binding to a port.
    ///
    /// Useful in tests via `axum::test` / Tower's `ServiceExt`, or when you
    /// want to compose the router into a larger application.
    pub fn router(&self) -> Router {
        let cors = tower_http::cors::CorsLayer::permissive();

        Router::new()
            .route("/api/graph", get(handle_graph))
            .route("/api/knot/:id", get(handle_knot))
            .route("/api/knot/:id/state", get(handle_knot_state))
            .route("/api/packet", post(handle_inject_packet))
            .route("/api/ws", get(handle_ws_upgrade))
            .with_state(self.state.clone())
            .layer(cors)
    }

    /// Binds to `addr` and starts serving.
    ///
    /// This future runs until the process exits or an OS-level error occurs.
    /// For graceful shutdown, wrap the returned future with
    /// [`tokio::select!`] against a shutdown signal.
    pub async fn serve(self, addr: SocketAddr) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!(addr = %addr, "HubFlow API server listening");
        axum::serve(listener, self.router()).await
    }
}

// ── REST handlers ─────────────────────────────────────────────────────────────

/// `GET /api/graph`
///
/// Returns the complete topology as a JSON array of [`RegistryEntry`] objects.
/// The frontend uses this to render the initial graph on load.
async fn handle_graph(State(state): State<AppState>) -> impl IntoResponse {
    let entries = state.registry.all().await;
    Json(entries)
}

/// `GET /api/knot/:id`
///
/// Returns a single [`RegistryEntry`] by UUID.
/// Responds with `400 Bad Request` for malformed UUIDs and `404 Not Found`
/// if the UUID is not present in the registry.
async fn handle_knot(State(state): State<AppState>, Path(id_str): Path<String>) -> Response {
    let id = match Uuid::parse_str(&id_str) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid UUID" })),
            )
                .into_response();
        }
    };

    match state.registry.get(&id).await {
        Some(entry) => Json(entry).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "knot not found" })),
        )
            .into_response(),
    }
}

/// `GET /api/knot/:id/state`
///
/// Returns an Object Knot's current property-state map as a JSON object.
/// Responds with `404 Not Found` if the Knot does not exist or has no tracked
/// state (e.g. it is a Hub or Action Knot).
async fn handle_knot_state(State(state): State<AppState>, Path(id_str): Path<String>) -> Response {
    let id = match Uuid::parse_str(&id_str) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid UUID" })),
            )
                .into_response();
        }
    };

    match state.registry.get_state_snapshot(&id).await {
        Some(snapshot) => Json(snapshot).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "knot not found or has no state" })),
        )
            .into_response(),
    }
}

/// `POST /api/packet`
///
/// Deserialises an [`InjectPacketRequest`] from the request body and pushes the
/// resulting [`Packet`] into the root inbox. Responds with `202 Accepted`
/// immediately — delivery is asynchronous.
///
/// Missing optional fields fall back to sensible defaults:
///
/// | Field       | Default            |
/// |-------------|--------------------|
/// | `sender_id` | nil UUID           |
/// | `priority`  | `128` (mid-range)  |
/// | `ttl_ms`    | `5000` (5 seconds) |
async fn handle_inject_packet(
    State(state): State<AppState>,
    Json(req): Json<InjectPacketRequest>,
) -> impl IntoResponse {
    let sender_id = req.sender_id.unwrap_or(Uuid::nil());
    let priority = req.priority.unwrap_or(128);
    let ttl = Duration::from_millis(req.ttl_ms.unwrap_or(5_000));

    let packet = Packet::new(sender_id, req.target_id, priority, ttl, req.payload);
    state.root_inbox.push(packet).await;

    (StatusCode::ACCEPTED, Json(json!({ "status": "queued" })))
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

/// `GET /api/ws`
///
/// Performs the WebSocket upgrade handshake and hands off to [`handle_ws`].
async fn handle_ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Drives a single WebSocket connection for its entire lifetime.
///
/// Two concurrent arms run inside a `select!` loop:
///
/// 1. **Server → client** — receives [`KnotEvent`]s from the broadcast bus and
///    serialises them as [`EventEnvelope`] JSON text frames.
/// 2. **Client → server** — parses incoming text frames as [`JsonRpcRequest`]
///    and dispatches the method to the appropriate handler.
///
/// The loop exits (and the connection closes) when:
/// - The client sends a `Close` frame or drops the connection.
/// - The broadcast channel is closed (`RecvError::Closed`).
/// - A send on the socket fails (client already gone).
///
/// Lag (missed events due to a slow client) is logged as a warning and does
/// **not** terminate the connection — the client simply misses those events.
async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let mut event_rx = state.event_tx.subscribe();

    loop {
        tokio::select! {
            // ── Server → client: forward broadcast events ─────────────────────
            result = event_rx.recv() => {
                match result {
                    Ok(event) => {
                        let envelope = event_to_envelope(&event);
                        let text = match serde_json::to_string(&envelope) {
                            Ok(t)  => t,
                            Err(e) => {
                                tracing::warn!("failed to serialise event: {e}");
                                continue;
                            }
                        };
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break; // client disconnected
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("WebSocket client lagged — missed {n} events");
                        // Continue serving; don't punish a slow client with a disconnect.
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::debug!("event bus closed — terminating WebSocket");
                        break;
                    }
                }
            }

            // ── Client → server: receive JSON-RPC commands ────────────────────
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_ws_message(text.as_str(), &state, &mut socket).await;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        // axum handles Pong automatically for most clients, but
                        // explicit handling ensures compatibility with all stacks.
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::debug!("WebSocket client sent Close or disconnected");
                        break;
                    }
                    Some(Ok(_)) => {
                        // Binary frames and Pong frames are silently ignored.
                    }
                    Some(Err(e)) => {
                        tracing::warn!("WebSocket receive error: {e}");
                        break;
                    }
                }
            }
        }
    }

    tracing::debug!("WebSocket connection closed");
}

/// Parses a raw text frame from the client as a [`JsonRpcRequest`] and routes
/// it to the appropriate handler.
///
/// Sends a JSON-RPC error response for parse failures or unknown methods.
async fn handle_ws_message(text: &str, state: &AppState, socket: &mut WebSocket) {
    // ── Parse ─────────────────────────────────────────────────────────────────
    let req: JsonRpcRequest = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(e) => {
            let resp = JsonRpcResponse::err(None, -32700, format!("parse error: {e}"));
            send_rpc(socket, &resp).await;
            return;
        }
    };

    let id = req.id.clone();

    // ── Dispatch ──────────────────────────────────────────────────────────────
    let response = match req.method.as_str() {
        "send_packet" => rpc_send_packet(id.clone(), req.params, state).await,
        "get_graph" => {
            let entries = state.registry.all().await;
            match serde_json::to_value(entries) {
                Ok(v) => JsonRpcResponse::ok(id, v),
                Err(e) => JsonRpcResponse::err(id, -32603, format!("internal error: {e}")),
            }
        }
        other => JsonRpcResponse::err(id, -32601, format!("method not found: {other}")),
    };

    send_rpc(socket, &response).await;
}

/// Handles the `send_packet` JSON-RPC method.
async fn rpc_send_packet(
    id: Option<Value>,
    params: Option<Value>,
    state: &AppState,
) -> JsonRpcResponse {
    let params = match params {
        Some(p) => p,
        None => return JsonRpcResponse::err(id, -32602, "params required for send_packet"),
    };

    let req: InjectPacketRequest = match serde_json::from_value(params) {
        Ok(r) => r,
        Err(e) => return JsonRpcResponse::err(id, -32602, format!("invalid params: {e}")),
    };

    let sender_id = req.sender_id.unwrap_or(Uuid::nil());
    let priority = req.priority.unwrap_or(128);
    let ttl = Duration::from_millis(req.ttl_ms.unwrap_or(5_000));

    let packet = Packet::new(sender_id, req.target_id, priority, ttl, req.payload);
    state.root_inbox.push(packet).await;

    JsonRpcResponse::ok(id, json!({ "status": "queued" }))
}

/// Serialises a [`JsonRpcResponse`] and sends it as a text frame.
///
/// Serialisation failures are logged but do not close the connection.
async fn send_rpc(socket: &mut WebSocket, resp: &JsonRpcResponse) {
    match serde_json::to_string(resp) {
        Ok(text) => {
            let _ = socket.send(Message::Text(text.into())).await;
        }
        Err(e) => {
            tracing::error!("failed to serialise JSON-RPC response: {e}");
        }
    }
}

// ── Event serialisation ───────────────────────────────────────────────────────

/// Converts a [`KnotEvent`] into a serialisable [`EventEnvelope`].
///
/// Each variant is mapped to a named `event_type` string so the frontend can
/// use a simple `switch` / `match` without depending on Rust enum layout.
/// The `payload` field always contains a flat JSON object.
pub fn event_to_envelope(event: &KnotEvent) -> EventEnvelope {
    match event {
        KnotEvent::Started { knot_id } => EventEnvelope {
            event_type: "Started".into(),
            payload: json!({ "knot_id": knot_id }),
        },

        KnotEvent::Stopped { knot_id } => EventEnvelope {
            event_type: "Stopped".into(),
            payload: json!({ "knot_id": knot_id }),
        },

        KnotEvent::PacketReceived { packet } => EventEnvelope {
            event_type: "PacketReceived".into(),
            payload: serde_json::to_value(packet).unwrap_or(Value::Null),
        },

        KnotEvent::PacketForwardedDown { target_id, packet } => EventEnvelope {
            event_type: "PacketForwardedDown".into(),
            payload: json!({
                "target_id": target_id,
                "packet":    packet,
            }),
        },

        KnotEvent::PacketForwardedUp { packet } => EventEnvelope {
            event_type: "PacketForwardedUp".into(),
            payload: serde_json::to_value(packet).unwrap_or(Value::Null),
        },

        KnotEvent::DeadLetter { packet, reason } => EventEnvelope {
            event_type: "DeadLetter".into(),
            payload: json!({
                "reason": reason.to_string(),
                "packet": packet,
            }),
        },

        KnotEvent::ObjectStateChanged {
            knot_id,
            property,
            value,
        } => EventEnvelope {
            event_type: "ObjectStateChanged".into(),
            payload: json!({
                "knot_id":  knot_id,
                "property": property,
                "value":    value,
            }),
        },

        KnotEvent::ActionExecuted {
            knot_id,
            engine_name,
            logs,
        } => EventEnvelope {
            event_type: "ActionExecuted".into(),
            payload: json!({
                "knot_id":     knot_id,
                "engine_name": engine_name,
                "logs":        logs,
            }),
        },
    }
}
