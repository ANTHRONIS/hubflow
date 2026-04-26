//! Events emitted by Knots during operation.
//!
//! Consumers subscribe to a Knot's event bus (a [`tokio::sync::broadcast`]
//! channel) to observe routing decisions, delivered packets, and lifecycle
//! state changes without coupling to the Knot's internals.

use crate::core::packet::Packet;
use uuid::Uuid;

// ── Dead-letter reason ────────────────────────────────────────────────────────

/// Reason why a [`Packet`] was classified as undeliverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeadLetterReason {
    /// The packet's TTL elapsed before it could be delivered.
    TtlExpired,

    /// The target Knot is unknown and there is no parent to escalate to.
    NoRoute,

    /// The packet exceeded the maximum allowed hop count (future use).
    MaxHopsExceeded,
}

impl std::fmt::Display for DeadLetterReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TtlExpired => write!(f, "TTL expired"),
            Self::NoRoute => write!(f, "no route to target"),
            Self::MaxHopsExceeded => write!(f, "maximum hop count exceeded"),
        }
    }
}

// ── Knot event ────────────────────────────────────────────────────────────────

/// An observable event produced by a [`Knot`](crate::core::knot::Knot).
///
/// Subscribe via [`Knot::event_bus`](crate::core::knot::Knot::event_bus) to
/// receive a [`tokio::sync::broadcast::Receiver<KnotEvent>`].
#[derive(Debug, Clone)]
pub enum KnotEvent {
    // ── Lifecycle ─────────────────────────────────────────────────────────────
    /// The Knot's [`run`](crate::core::knot::Knot::run) loop has started.
    Started { knot_id: Uuid },

    /// The Knot's [`run`](crate::core::knot::Knot::run) loop has stopped.
    Stopped { knot_id: Uuid },

    // ── Routing ───────────────────────────────────────────────────────────────
    /// A packet addressed to this Knot has been received and is being executed.
    PacketReceived { packet: Packet },

    /// A packet has been forwarded to a known child Knot.
    PacketForwardedDown { target_id: Uuid, packet: Packet },

    /// A packet with an unknown target has been escalated to the parent Knot.
    PacketForwardedUp { packet: Packet },

    /// A packet could not be delivered and has been dropped.
    DeadLetter {
        packet: Packet,
        reason: DeadLetterReason,
    },

    // ── State & execution ─────────────────────────────────────────────────────
    /// Fired when an Object Knot's property changes.
    ObjectStateChanged {
        knot_id: Uuid,
        property: String,
        value: serde_json::Value,
    },

    /// Fired after an Action Knot's engine completes execution.
    ActionExecuted {
        knot_id: Uuid,
        engine_name: String,
        logs: Vec<String>,
    },

    // ── Graph mutations ───────────────────────────────────────────────────────
    /// A new Knot has been added to the topology via the API.
    KnotCreated {
        knot_id: Uuid,
        name: String,
        role: String,
        level: u32,
        parent_id: Option<Uuid>,
        x: f32,
        y: f32,
    },

    /// A Knot has been removed from the topology via the API.
    KnotDeleted { knot_id: Uuid },

    /// A Knot's metadata or canvas position was updated via the API.
    KnotUpdated {
        knot_id:     Uuid,
        name:        Option<String>,
        description: Option<String>,
        x:           Option<f32>,
        y:           Option<f32>,
    },

    /// Clients should re-fetch `GET /api/graph` to sync the full topology.
    GraphRefresh,
}

impl std::fmt::Display for KnotEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Started { knot_id } => write!(f, "[{knot_id}] started"),
            Self::Stopped { knot_id } => write!(f, "[{knot_id}] stopped"),
            Self::PacketReceived { packet } => {
                write!(
                    f,
                    "[{}] received packet from {}",
                    packet.target_id, packet.sender_id
                )
            }
            Self::PacketForwardedDown { target_id, packet } => {
                write!(f, "[{}] forwarded ↓ to child {target_id}", packet.sender_id)
            }
            Self::PacketForwardedUp { packet } => {
                write!(f, "[{}] forwarded ↑ to parent", packet.sender_id)
            }
            Self::DeadLetter { packet, reason } => {
                write!(
                    f,
                    "[{}→{}] dead-letter: {reason}",
                    packet.sender_id, packet.target_id
                )
            }
            Self::ObjectStateChanged {
                knot_id,
                property,
                value,
            } => {
                write!(f, "[{knot_id}] state changed: {property} = {value}")
            }
            Self::ActionExecuted {
                knot_id,
                engine_name,
                logs,
            } => {
                write!(
                    f,
                    "[{knot_id}] action executed by {engine_name} ({} log lines)",
                    logs.len()
                )
            }
            Self::KnotCreated {
                knot_id,
                name,
                role,
                ..
            } => write!(f, "[graph] knot created: {name} ({role}) id={knot_id}"),
            Self::KnotDeleted { knot_id } => write!(f, "[graph] knot deleted: {knot_id}"),
            Self::KnotUpdated { knot_id, name, description, .. } => {
                write!(f, "[graph] knot updated: {knot_id} name={name:?} desc={description:?}")
            }
            Self::GraphRefresh => write!(f, "[graph] topology refresh requested"),
        }
    }
}
