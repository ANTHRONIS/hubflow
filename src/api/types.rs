//! Request / response types for the HubFlow HTTP + WebSocket API.
//!
//! These types are intentionally kept separate from the core domain model so
//! that the serialisation contract can evolve independently.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

// ── REST request types ────────────────────────────────────────────────────────

/// Request body for `POST /api/packet` — injects a packet into the system.
#[derive(Debug, Deserialize)]
pub struct InjectPacketRequest {
    /// UUID of the destination Knot.
    pub target_id: Uuid,

    /// UUID of the originating sender. Defaults to the nil UUID when absent.
    pub sender_id: Option<Uuid>,

    /// Delivery priority (0 = lowest, 255 = highest). Defaults to `128`.
    pub priority: Option<u8>,

    /// Packet time-to-live in milliseconds. Defaults to `5000` (5 seconds).
    pub ttl_ms: Option<u64>,

    /// Arbitrary JSON payload to carry inside the packet.
    pub payload: Value,
}

// ── JSON-RPC 2.0 envelope ─────────────────────────────────────────────────────

/// A JSON-RPC 2.0 request sent by the client over the WebSocket.
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// Must be `"2.0"`.
    pub jsonrpc: String,

    /// The method name (e.g. `"send_packet"`, `"get_graph"`).
    pub method: String,

    /// Optional parameters; method-specific shape.
    pub params: Option<Value>,

    /// Correlation identifier echoed back in the response.
    pub id: Option<Value>,
}

/// A JSON-RPC 2.0 response sent by the server over the WebSocket.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: String,

    /// Present on success; absent on error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    /// Present on error; absent on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,

    /// Echoed from the corresponding request `id` field.
    pub id: Option<Value>,
}

/// Error object embedded in a failed [`JsonRpcResponse`].
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    /// Numeric error code following the JSON-RPC 2.0 convention.
    /// Common values: `-32700` parse error, `-32601` method not found,
    /// `-32602` invalid params.
    pub code: i32,

    /// Human-readable error message.
    pub message: String,
}

impl JsonRpcResponse {
    /// Constructs a successful response carrying `result`.
    pub fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Constructs an error response.
    pub fn err(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
            id,
        }
    }
}

// ── WebSocket push envelope ───────────────────────────────────────────────────

/// A serialisable event envelope pushed to every connected WebSocket client.
///
/// All [`KnotEvent`](crate::core::event::KnotEvent) variants are mapped to this
/// structure by [`crate::api::server::event_to_envelope`] so the frontend never
/// needs to handle raw Rust enum layout.
#[derive(Debug, Serialize)]
pub struct EventEnvelope {
    /// Discriminant string matching the `KnotEvent` variant name
    /// (e.g. `"PacketReceived"`, `"DeadLetter"`).
    pub event_type: String,

    /// Variant-specific data serialised as a flat JSON object.
    pub payload: Value,
}
