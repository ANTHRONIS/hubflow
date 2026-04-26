//! The Knot — the evolved, fully-functional core unit of HubFlow.
//!
//! A [`Knot`] is the central abstraction of the HubFlow ecosystem. It extends
//! the identity model of the original `Cell` with:
//!
//! - A **role-based variant** ([`KnotVariant`]) that carries role-specific
//!   logic and state (Class schema, Object property store, Action engine).
//! - An **async priority inbox** for local-first packet routing.
//! - A **registry handle** for cross-Knot state access.
//! - A **two-tier state store** (volatile RAM + pluggable persistent backend).
//!
//! # Topology (Solar System model)
//!
//! ```text
//!              ┌────────────────────────────────────┐
//!              │          Level 0 — Root Hub         │
//!              └──────────┬─────────────────────────┘
//!                         │
//!          ┌──────────────┼──────────────┐
//!          ▼              ▼              ▼
//!       Hub A          Hub B          Hub C        ← Level 1+
//!      /  |  \
//!  Action Object Class                             ← Satellites
//! ```
//!
//! # Routing rules (local-first)
//!
//! | Condition                         | Action                          |
//! |-----------------------------------|---------------------------------|
//! | `target == self.id`               | Execute locally via [`KnotVariant`] |
//! | `target` ∈ `children`             | Forward ↓ (down to child)       |
//! | `target` ∉ `children` + has parent | Forward ↑ (up to parent)      |
//! | TTL expired                       | Drop + emit `DeadLetter`        |
//! | No parent & unknown target        | Drop + emit `DeadLetter(NoRoute)` |
//!
//! # Object packet protocol
//!
//! Object Knots respond to packets with JSON payloads matching these schemas:
//!
//! | `op` field   | Description                        | Response op       |
//! |--------------|------------------------------------|-------------------|
//! | `get`        | Read one property                  | `get_response`    |
//! | `set`        | Write one property                 | `set_ack`         |
//! | `set_many`   | Write multiple properties at once  | `set_many_ack`    |
//! | `get_all`    | Read all properties                | `get_all_response`|

use crate::core::action::{ActionContext, EchoEngine, LogicEngine, ObjectSnapshot};
use crate::core::cell::{CellMeta, LocalId};
use crate::core::class::ClassDefinition;
use crate::core::event::{DeadLetterReason, KnotEvent};
use crate::core::inbox::Inbox;
use crate::core::packet::Packet;
use crate::core::registry::{KnotRegistry, RegistryEntry};
use crate::core::state::{
    InMemoryStore, NoopPersistentStore, PersistentStore, StateKey, VolatileStore,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, broadcast, watch};
use uuid::Uuid;

// ── KnotRole ──────────────────────────────────────────────────────────────────

/// The functional role of a Knot within the HubFlow topology.
///
/// Role determines the *semantic* purpose and the execute-dispatch behaviour of
/// a Knot. All roles participate in the same routing protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnotRole {
    /// Coordinates a sub-system of Satellites. May have child Knots of any role.
    Hub,
    /// Encapsulates executable logic via a [`LogicEngine`].
    Action,
    /// Holds structured, mutable data initialised from a [`ClassDefinition`].
    Object,
    /// Acts as a blueprint/schema for creating Object Knots.
    Class,
}

impl std::fmt::Display for KnotRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hub => write!(f, "Hub"),
            Self::Action => write!(f, "Action"),
            Self::Object => write!(f, "Object"),
            Self::Class => write!(f, "Class"),
        }
    }
}

// ── KnotVariant ───────────────────────────────────────────────────────────────

/// Role-specific state and behaviour carried by a [`Knot`].
///
/// The active variant determines how `execute()` processes locally-delivered
/// packets. All variants share the same routing, inbox, and event-bus logic.
pub enum KnotVariant {
    /// No local execution — Hub Knots only coordinate routing.
    Hub,

    /// Stores a [`ClassDefinition`] and responds to schema-query packets.
    Class {
        /// The property blueprint managed by this Class Knot.
        definition: ClassDefinition,
    },

    /// Maintains a named property store initialised from a Class schema.
    Object {
        /// UUID of the Class this Object was instantiated from, if any.
        class_id: Option<Uuid>,
        /// Shared, async-safe property map.
        ///
        /// Written by the Object's own execute handler and by the registry's
        /// `set_state()` method. Read by linked Action Knots via snapshots.
        shared_state: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    },

    /// Runs a [`LogicEngine`] on every locally-delivered packet.
    Action {
        /// The pluggable computation engine for this Action Knot.
        engine: Arc<dyn LogicEngine>,
        /// UUIDs of Object Knots whose state is available to the engine.
        linked_object_ids: Vec<Uuid>,
    },
}

// ── ChildHandle ───────────────────────────────────────────────────────────────

/// A lightweight, cloneable reference to a child Knot's inbox.
///
/// The parent Knot holds one [`ChildHandle`] per registered child and uses it
/// to forward packets downward without borrowing the child directly.
#[derive(Clone)]
pub struct ChildHandle {
    /// System-wide UUID of the child Knot.
    pub id: Uuid,
    /// Shared handle to the child's priority inbox.
    pub inbox: Arc<Inbox>,
}

// ── Knot ──────────────────────────────────────────────────────────────────────

/// The evolved, async core unit of HubFlow.
///
/// # Construction
///
/// Use the typed constructors for clarity:
///
/// ```rust,no_run
/// use hubflow::core::cell::CellMeta;
/// use hubflow::core::knot::{Knot, KnotRole};
/// use hubflow::core::class::{ClassDefinition, PropertySchema, DataType};
/// use hubflow::core::action::EchoEngine;
/// use std::sync::Arc;
///
/// // Hub (root)
/// let (mut hub, _events) = Knot::new_hub(0, CellMeta::new("Root"), 0, None, None);
///
/// // Class blueprint
/// let class = ClassDefinition::new("Sensor")
///     .with_property(PropertySchema::new("temperature", DataType::Number));
/// let (class_knot, _) = Knot::new_class(1, CellMeta::new("SensorClass"), class.clone(), 1, Some(hub.id()), Some(hub.inbox()));
///
/// // Object instantiated from class
/// let (obj, _) = Knot::new_object(2, CellMeta::new("Sensor1"), Some(&class), 1, Some(hub.id()), Some(hub.inbox()));
///
/// // Action with echo engine
/// let (act, _) = Knot::new_action(3, CellMeta::new("Processor"), Arc::new(EchoEngine), 1, Some(hub.id()), Some(hub.inbox()));
/// ```
pub struct Knot {
    // ── Identity ─────────────────────────────────────────────────────────────
    /// System-wide unique identifier (UUID v4).
    id: Uuid,
    /// Node-local identifier for fast in-process lookup / indexing.
    local_id: LocalId,
    /// Human-readable metadata (name + optional description).
    meta: CellMeta,

    // ── Topology ─────────────────────────────────────────────────────────────
    /// Semantic role used for routing decisions and display.
    role: KnotRole,
    /// Depth in the topology tree (0 = root).
    level: u32,
    /// UUID of the parent Knot, or `None` for the root.
    parent_id: Option<Uuid>,

    // ── Communication ────────────────────────────────────────────────────────
    /// This Knot's own priority inbox (shared via `Arc` for external push).
    inbox: Arc<Inbox>,
    /// Inbox of the parent Knot — used to forward packets upward.
    parent_inbox: Option<Arc<Inbox>>,
    /// Routing table mapping child UUIDs to their inbox handles.
    children: HashMap<Uuid, ChildHandle>,

    // ── Events ───────────────────────────────────────────────────────────────
    /// Broadcast sender for the Knot's observable event stream.
    event_tx: broadcast::Sender<KnotEvent>,

    // ── Variant-specific state ────────────────────────────────────────────────
    /// Role-specific data and behaviour (see [`KnotVariant`]).
    variant: KnotVariant,

    // ── State stores ──────────────────────────────────────────────────────────
    /// General-purpose volatile in-memory property store.
    volatile: InMemoryStore,
    /// Pluggable persistent backend (defaults to no-op; swap for SQLite etc.).
    persistent: Arc<dyn PersistentStore>,

    // ── Registry ─────────────────────────────────────────────────────────────
    /// Optional system-wide Knot directory for cross-Knot state access.
    registry: Option<Arc<KnotRegistry>>,
}

impl Knot {
    // ── Internal constructor ──────────────────────────────────────────────────

    fn new_with_variant(
        id: Uuid,
        local_id: LocalId,
        meta: CellMeta,
        role: KnotRole,
        variant: KnotVariant,
        level: u32,
        parent_id: Option<Uuid>,
        parent_inbox: Option<Arc<Inbox>>,
    ) -> (Self, broadcast::Receiver<KnotEvent>) {
        let (event_tx, event_rx) = broadcast::channel(256);
        let knot = Self {
            id,
            local_id,
            meta,
            role,
            level,
            parent_id,
            inbox: Inbox::new(),
            parent_inbox,
            children: HashMap::new(),
            event_tx,
            variant,
            volatile: InMemoryStore::default(),
            persistent: Arc::new(NoopPersistentStore::default()),
            registry: None,
        };
        (knot, event_rx)
    }

    // ── Typed constructors ────────────────────────────────────────────────────

    /// Creates a **Hub** Knot with a freshly generated UUID.
    pub fn new_hub(
        local_id: LocalId,
        meta: CellMeta,
        level: u32,
        parent_id: Option<Uuid>,
        parent_inbox: Option<Arc<Inbox>>,
    ) -> (Self, broadcast::Receiver<KnotEvent>) {
        Self::new_with_variant(
            Uuid::new_v4(),
            local_id,
            meta,
            KnotRole::Hub,
            KnotVariant::Hub,
            level,
            parent_id,
            parent_inbox,
        )
    }

    /// Creates a **Class** Knot that stores `definition` as its schema.
    pub fn new_class(
        local_id: LocalId,
        meta: CellMeta,
        definition: ClassDefinition,
        level: u32,
        parent_id: Option<Uuid>,
        parent_inbox: Option<Arc<Inbox>>,
    ) -> (Self, broadcast::Receiver<KnotEvent>) {
        Self::new_with_variant(
            Uuid::new_v4(),
            local_id,
            meta,
            KnotRole::Class,
            KnotVariant::Class { definition },
            level,
            parent_id,
            parent_inbox,
        )
    }

    /// Creates an **Object** Knot, optionally initialised from a [`ClassDefinition`].
    ///
    /// If `class` is `Some`, the Object's property store is pre-populated with
    /// the class's default values (see [`ClassDefinition::default_state`]).
    pub fn new_object(
        local_id: LocalId,
        meta: CellMeta,
        class: Option<&ClassDefinition>,
        level: u32,
        parent_id: Option<Uuid>,
        parent_inbox: Option<Arc<Inbox>>,
    ) -> (Self, broadcast::Receiver<KnotEvent>) {
        let class_id = class.map(|c| c.id);
        let initial_state = class.map(|c| c.default_state()).unwrap_or_default();
        let shared_state = Arc::new(RwLock::new(initial_state));

        Self::new_with_variant(
            Uuid::new_v4(),
            local_id,
            meta,
            KnotRole::Object,
            KnotVariant::Object {
                class_id,
                shared_state,
            },
            level,
            parent_id,
            parent_inbox,
        )
    }

    /// Creates an **Action** Knot with the given [`LogicEngine`].
    pub fn new_action(
        local_id: LocalId,
        meta: CellMeta,
        engine: Arc<dyn LogicEngine>,
        level: u32,
        parent_id: Option<Uuid>,
        parent_inbox: Option<Arc<Inbox>>,
    ) -> (Self, broadcast::Receiver<KnotEvent>) {
        Self::new_with_variant(
            Uuid::new_v4(),
            local_id,
            meta,
            KnotRole::Action,
            KnotVariant::Action {
                engine,
                linked_object_ids: Vec::new(),
            },
            level,
            parent_id,
            parent_inbox,
        )
    }

    /// **Backward-compatible** generic constructor.
    ///
    /// Creates a Knot with a role-appropriate default [`KnotVariant`]:
    /// - `Hub`    → [`KnotVariant::Hub`]
    /// - `Action` → [`KnotVariant::Action`] with [`EchoEngine`]
    /// - `Object` → [`KnotVariant::Object`] with empty state
    /// - `Class`  → [`KnotVariant::Class`] with an unnamed empty definition
    pub fn new(
        local_id: LocalId,
        meta: CellMeta,
        role: KnotRole,
        level: u32,
        parent_id: Option<Uuid>,
        parent_inbox: Option<Arc<Inbox>>,
    ) -> (Self, broadcast::Receiver<KnotEvent>) {
        let variant = match &role {
            KnotRole::Hub => KnotVariant::Hub,
            KnotRole::Action => KnotVariant::Action {
                engine: Arc::new(EchoEngine),
                linked_object_ids: Vec::new(),
            },
            KnotRole::Object => KnotVariant::Object {
                class_id: None,
                shared_state: Arc::new(RwLock::new(HashMap::new())),
            },
            KnotRole::Class => KnotVariant::Class {
                definition: ClassDefinition::new(""),
            },
        };
        Self::new_with_variant(
            Uuid::new_v4(),
            local_id,
            meta,
            role,
            variant,
            level,
            parent_id,
            parent_inbox,
        )
    }

    /// Reconstructs a Knot from persisted identity components (restore from DB).
    ///
    /// Prefer the typed constructors for new Knots; use `from_parts` only when
    /// rehydrating from a serialised snapshot.
    pub fn from_parts(
        id: Uuid,
        local_id: LocalId,
        meta: CellMeta,
        role: KnotRole,
        level: u32,
        parent_id: Option<Uuid>,
        parent_inbox: Option<Arc<Inbox>>,
    ) -> (Self, broadcast::Receiver<KnotEvent>) {
        let variant = match &role {
            KnotRole::Hub => KnotVariant::Hub,
            KnotRole::Action => KnotVariant::Action {
                engine: Arc::new(EchoEngine),
                linked_object_ids: Vec::new(),
            },
            KnotRole::Object => KnotVariant::Object {
                class_id: None,
                shared_state: Arc::new(RwLock::new(HashMap::new())),
            },
            KnotRole::Class => KnotVariant::Class {
                definition: ClassDefinition::new(""),
            },
        };
        Self::new_with_variant(
            id,
            local_id,
            meta,
            role,
            variant,
            level,
            parent_id,
            parent_inbox,
        )
    }

    // ── Child management ──────────────────────────────────────────────────────

    /// Registers a child Knot in the routing table.
    ///
    /// Packets addressed to `handle.id` will be forwarded directly to the
    /// child's inbox instead of being escalated to the parent.
    pub fn attach_child(&mut self, handle: ChildHandle) {
        tracing::debug!(knot_id = %self.id, child_id = %handle.id, "child attached");
        self.children.insert(handle.id, handle);
    }

    /// Removes a child from the routing table and returns its handle.
    pub fn detach_child(&mut self, child_id: &Uuid) -> Option<ChildHandle> {
        let h = self.children.remove(child_id);
        if h.is_some() {
            tracing::debug!(knot_id = %self.id, %child_id, "child detached");
        }
        h
    }

    /// Returns a [`ChildHandle`] pointing at this Knot's inbox.
    ///
    /// Pass this to a parent Knot's [`attach_child`](Self::attach_child) so
    /// the parent can route packets down to this Knot.
    pub fn as_child_handle(&self) -> ChildHandle {
        ChildHandle {
            id: self.id,
            inbox: Arc::clone(&self.inbox),
        }
    }

    // ── Packet entry-point ────────────────────────────────────────────────────

    /// Pushes a packet directly into this Knot's inbox from external code.
    pub async fn send(&self, packet: Packet) {
        self.inbox.push(packet).await;
    }

    // ── Routing ───────────────────────────────────────────────────────────────

    /// Routes a packet according to HubFlow's **local-first** rules.
    pub async fn handle_packet(&mut self, packet: Packet) {
        // ── TTL guard ─────────────────────────────────────────────────────────
        if packet.is_expired() {
            tracing::warn!(
                knot_id   = %self.id,
                target_id = %packet.target_id,
                "packet TTL expired — dead-lettering"
            );
            let _ = self.event_tx.send(KnotEvent::DeadLetter {
                packet,
                reason: DeadLetterReason::TtlExpired,
            });
            return;
        }

        // ── Local delivery ────────────────────────────────────────────────────
        if packet.target_id == self.id {
            tracing::debug!(
                knot_id   = %self.id,
                sender_id = %packet.sender_id,
                priority  = packet.priority,
                "packet delivered locally"
            );
            let _ = self.event_tx.send(KnotEvent::PacketReceived {
                packet: packet.clone(),
            });
            self.execute(packet).await;
            return;
        }

        // ── Forward down (known child) ────────────────────────────────────────
        // Clone the Arc before borrowing event_tx so the borrow checker does not
        // see an overlap between self.children and self.event_tx.
        let child_inbox = self
            .children
            .get(&packet.target_id)
            .map(|c| Arc::clone(&c.inbox));

        if let Some(inbox) = child_inbox {
            tracing::debug!(
                knot_id   = %self.id,
                target_id = %packet.target_id,
                "forwarding packet ↓ to child"
            );
            let _ = self.event_tx.send(KnotEvent::PacketForwardedDown {
                target_id: packet.target_id,
                packet: packet.clone(),
            });
            inbox.push(packet).await;
            return;
        }

        // ── Forward up (unknown target → escalate) ────────────────────────────
        let parent_inbox = self.parent_inbox.clone();

        if let Some(inbox) = parent_inbox {
            tracing::debug!(
                knot_id   = %self.id,
                target_id = %packet.target_id,
                "forwarding packet ↑ to parent"
            );
            let _ = self.event_tx.send(KnotEvent::PacketForwardedUp {
                packet: packet.clone(),
            });
            inbox.push(packet).await;
            return;
        }

        // ── No route ──────────────────────────────────────────────────────────
        tracing::warn!(knot_id = %self.id, target_id = %packet.target_id, "no route — dead-lettering");
        let _ = self.event_tx.send(KnotEvent::DeadLetter {
            packet,
            reason: DeadLetterReason::NoRoute,
        });
    }

    // ── Execute dispatch ──────────────────────────────────────────────────────

    /// Dispatches a locally-delivered packet to the active [`KnotVariant`] handler.
    ///
    /// Response packets produced by the handler are pushed back into this
    /// Knot's own inbox for transparent re-routing via [`handle_packet`].
    async fn execute(&mut self, packet: Packet) {
        // Extract variant-specific data as owned values so `self` is free
        // to be borrowed again for handler calls and event emission.
        enum Dispatch {
            Hub,
            Class(serde_json::Value),
            Object(Arc<RwLock<HashMap<String, serde_json::Value>>>),
            Action(Arc<dyn LogicEngine>, Vec<Uuid>),
        }

        let dispatch = match &self.variant {
            KnotVariant::Hub => Dispatch::Hub,
            KnotVariant::Class { definition } => {
                Dispatch::Class(serde_json::to_value(definition).unwrap_or(serde_json::Value::Null))
            }
            KnotVariant::Object { shared_state, .. } => Dispatch::Object(Arc::clone(shared_state)),
            KnotVariant::Action {
                engine,
                linked_object_ids,
            } => Dispatch::Action(Arc::clone(engine), linked_object_ids.clone()),
        };

        // `self.variant` is no longer borrowed — full `self` is accessible.
        let response_packets: Vec<Packet> = match dispatch {
            Dispatch::Hub => {
                tracing::debug!(knot_id = %self.id, "hub: local packet — no execute action");
                vec![]
            }

            Dispatch::Class(schema_json) => {
                // Respond with the class schema so clients can introspect it.
                vec![Packet::new(
                    self.id,
                    packet.sender_id,
                    packet.priority,
                    Duration::from_secs(5),
                    serde_json::json!({ "op": "class_schema", "schema": schema_json }),
                )]
            }

            Dispatch::Object(shared_state) => {
                self.handle_object_packet(&packet, shared_state).await
            }

            Dispatch::Action(engine, linked_ids) => {
                self.handle_action_packet(packet, engine, linked_ids).await
            }
        };

        // Push response packets back into our own inbox — handle_packet will
        // route them (forward ↓ or ↑) without triggering execute() again,
        // because their target_id differs from self.id.
        for p in response_packets {
            self.inbox.push(p).await;
        }
    }

    /// Handles a property operation packet directed at an **Object** Knot.
    ///
    /// Supported `op` values: `get`, `set`, `set_many`, `get_all`.
    async fn handle_object_packet(
        &self,
        packet: &Packet,
        shared_state: Arc<RwLock<HashMap<String, serde_json::Value>>>,
    ) -> Vec<Packet> {
        let op = packet
            .payload
            .get("op")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut responses = Vec::new();

        match op {
            // ── Read one property ─────────────────────────────────────────────
            "get" => {
                if let Some(name) = packet.payload.get("property").and_then(|v| v.as_str()) {
                    let value = shared_state
                        .read()
                        .await
                        .get(name)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    responses.push(Packet::new(
                        self.id, packet.sender_id, packet.priority,
                        Duration::from_secs(5),
                        serde_json::json!({ "op": "get_response", "property": name, "value": value }),
                    ));
                }
            }

            // ── Write one property ────────────────────────────────────────────
            "set" => {
                let name = packet
                    .payload
                    .get("property")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let value = packet.payload.get("value").cloned();

                if let (Some(name), Some(val)) = (name, value) {
                    shared_state.write().await.insert(name.clone(), val.clone());

                    // Sync registry snapshot
                    if let Some(reg) = &self.registry {
                        reg.set_state(self.id, name.clone(), val.clone()).await;
                    }

                    let _ = self.event_tx.send(KnotEvent::ObjectStateChanged {
                        knot_id: self.id,
                        property: name.clone(),
                        value: val,
                    });

                    responses.push(Packet::new(
                        self.id,
                        packet.sender_id,
                        packet.priority,
                        Duration::from_secs(5),
                        serde_json::json!({ "op": "set_ack", "property": name }),
                    ));
                }
            }

            // ── Write multiple properties ─────────────────────────────────────
            "set_many" => {
                if let Some(map) = packet
                    .payload
                    .get("properties")
                    .and_then(|v| v.as_object())
                    .cloned()
                {
                    {
                        let mut state = shared_state.write().await;
                        for (k, v) in &map {
                            state.insert(k.clone(), v.clone());
                        }
                    }
                    for (k, v) in &map {
                        if let Some(reg) = &self.registry {
                            reg.set_state(self.id, k.clone(), v.clone()).await;
                        }
                        let _ = self.event_tx.send(KnotEvent::ObjectStateChanged {
                            knot_id: self.id,
                            property: k.clone(),
                            value: v.clone(),
                        });
                    }
                    responses.push(Packet::new(
                        self.id,
                        packet.sender_id,
                        packet.priority,
                        Duration::from_secs(5),
                        serde_json::json!({ "op": "set_many_ack", "count": map.len() }),
                    ));
                }
            }

            // ── Read all properties ───────────────────────────────────────────
            "get_all" => {
                let props = shared_state.read().await.clone();
                let props_val = serde_json::to_value(&props).unwrap_or(serde_json::Value::Null);
                responses.push(Packet::new(
                    self.id,
                    packet.sender_id,
                    packet.priority,
                    Duration::from_secs(5),
                    serde_json::json!({ "op": "get_all_response", "properties": props_val }),
                ));
            }

            other => {
                tracing::warn!(knot_id = %self.id, op = other, "object: unknown operation");
            }
        }

        responses
    }

    /// Runs the [`LogicEngine`] for an **Action** Knot.
    ///
    /// Gathers state snapshots of all linked Object Knots from the registry,
    /// invokes the engine, applies property updates, and returns response
    /// packets for routing.
    async fn handle_action_packet(
        &self,
        packet: Packet,
        engine: Arc<dyn LogicEngine>,
        linked_ids: Vec<Uuid>,
    ) -> Vec<Packet> {
        // Collect snapshots of linked objects (best-effort; missing = absent).
        let mut objects: HashMap<Uuid, ObjectSnapshot> = HashMap::new();
        if let Some(reg) = &self.registry {
            for &id in &linked_ids {
                if let Some(state) = reg.get_state_snapshot(&id).await {
                    objects.insert(
                        id,
                        ObjectSnapshot {
                            id,
                            class_id: reg.get(&id).await.and_then(|e| e.class_id),
                            properties: state,
                        },
                    );
                }
            }
        }

        let ctx = ActionContext {
            self_id: self.id,
            incoming: packet,
            objects,
        };

        let output = engine.execute(&ctx).await;

        // Apply property updates to linked objects via registry.
        if let Some(reg) = &self.registry {
            for update in &output.updates {
                reg.set_state(
                    update.object_id,
                    update.property_name.clone(),
                    update.value.clone(),
                )
                .await;
            }
        }

        // Surface engine logs to the event bus.
        if !output.logs.is_empty() {
            let _ = self.event_tx.send(KnotEvent::ActionExecuted {
                knot_id: self.id,
                engine_name: engine.name().to_string(),
                logs: output.logs,
            });
        }

        output.packets
    }

    // ── Heartbeat (event loop) ────────────────────────────────────────────────

    /// Starts the Knot's async event loop — its **heartbeat**.
    ///
    /// Continuously dequeues the highest-priority packet from the inbox and
    /// routes it via [`handle_packet`](Self::handle_packet).
    ///
    /// # Shutdown
    ///
    /// Supply the receiver side of a [`tokio::sync::watch`] channel initialised
    /// to `false`. Send `true` (or drop the sender) to trigger a graceful stop:
    ///
    /// ```rust,no_run
    /// # use tokio::sync::watch;
    /// let (tx, rx) = watch::channel(false);
    /// tokio::spawn(async move { knot.run(rx).await; });
    /// let _ = tx.send(true); // graceful stop
    /// ```
    ///
    /// Packets remaining in the inbox when shutdown is signalled are abandoned.
    pub async fn run(&mut self, mut shutdown_rx: watch::Receiver<bool>) {
        // Clone the inbox Arc so we can await `inbox.pop()` inside select!
        // without holding a borrow on `self` (needed for handle_packet).
        let inbox = Arc::clone(&self.inbox);

        let _ = self.event_tx.send(KnotEvent::Started { knot_id: self.id });
        tracing::info!(
            knot_id = %self.id,
            role    = %self.role,
            level   = self.level,
            name    = %self.meta.name,
            "knot started"
        );

        // Register in registry if available.
        if let Some(reg) = &self.registry {
            reg.upsert(self.registry_entry()).await;
        }

        loop {
            tokio::select! {
                biased; // Check shutdown before processing packets.

                result = shutdown_rx.changed() => {
                    if result.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }

                packet = inbox.pop() => {
                    self.handle_packet(packet).await;
                }
            }
        }

        // Deregister from registry on shutdown.
        if let Some(reg) = &self.registry {
            reg.remove(&self.id).await;
        }

        let _ = self.event_tx.send(KnotEvent::Stopped { knot_id: self.id });
        tracing::info!(knot_id = %self.id, role = %self.role, "knot stopped");
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    /// Returns the system-wide unique identifier.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Returns the node-local identifier.
    pub fn local_id(&self) -> LocalId {
        self.local_id
    }

    /// Returns a reference to the human-readable metadata.
    pub fn meta(&self) -> &CellMeta {
        &self.meta
    }

    /// Returns a mutable reference to the metadata.
    pub fn meta_mut(&mut self) -> &mut CellMeta {
        &mut self.meta
    }

    /// Returns the semantic role.
    pub fn role(&self) -> &KnotRole {
        &self.role
    }

    /// Returns the topology level (0 = root).
    pub fn level(&self) -> u32 {
        self.level
    }

    /// Returns the UUID of the parent Knot, or `None` for the root.
    pub fn parent_id(&self) -> Option<Uuid> {
        self.parent_id
    }

    /// Returns a clone of the shared inbox handle.
    pub fn inbox(&self) -> Arc<Inbox> {
        Arc::clone(&self.inbox)
    }

    /// Returns a clone of the event-bus broadcast sender.
    ///
    /// Call `.subscribe()` on the returned sender to obtain a new
    /// [`broadcast::Receiver<KnotEvent>`].
    pub fn event_bus(&self) -> broadcast::Sender<KnotEvent> {
        self.event_tx.clone()
    }

    /// Returns the human-readable name of the active variant.
    pub fn variant_name(&self) -> &'static str {
        match &self.variant {
            KnotVariant::Hub => "Hub",
            KnotVariant::Class { .. } => "Class",
            KnotVariant::Object { .. } => "Object",
            KnotVariant::Action { .. } => "Action",
        }
    }

    // ── Variant-specific accessors ────────────────────────────────────────────

    /// Returns the UUID of the Class this Knot is linked to, if applicable.
    ///
    /// - Object Knots: the class they were instantiated from.
    /// - Class  Knots: the definition's own UUID.
    /// - Others: `None`.
    pub fn class_id(&self) -> Option<Uuid> {
        match &self.variant {
            KnotVariant::Object { class_id, .. } => *class_id,
            KnotVariant::Class { definition } => Some(definition.id),
            _ => None,
        }
    }

    /// Returns the shared property-state handle for **Object** Knots.
    ///
    /// Returns `None` for all other roles.
    pub fn shared_state(&self) -> Option<Arc<RwLock<HashMap<String, serde_json::Value>>>> {
        match &self.variant {
            KnotVariant::Object { shared_state, .. } => Some(Arc::clone(shared_state)),
            _ => None,
        }
    }

    /// Reads a single property from an **Object** Knot's shared state.
    ///
    /// Returns `None` for non-Object Knots or absent properties.
    pub async fn get_property(&self, name: &str) -> Option<serde_json::Value> {
        let state = self.shared_state()?;
        state.read().await.get(name).cloned()
    }

    // ── Registry helpers ──────────────────────────────────────────────────────

    /// Attaches this Knot to a system-wide [`KnotRegistry`].
    ///
    /// Once attached, Action Knots can read Object state snapshots via the
    /// registry, and the API server can enumerate all live Knots.
    pub fn set_registry(&mut self, registry: Arc<KnotRegistry>) {
        self.registry = Some(registry);
    }

    /// Links an Object Knot's UUID to this **Action** Knot.
    ///
    /// Linked Objects' state is available as [`ObjectSnapshot`]s inside the
    /// [`ActionContext`] during engine execution.
    ///
    /// Has no effect on non-Action Knots.
    pub fn link_object(&mut self, object_id: Uuid) {
        if let KnotVariant::Action {
            linked_object_ids, ..
        } = &mut self.variant
        {
            if !linked_object_ids.contains(&object_id) {
                linked_object_ids.push(object_id);
            }
        }
    }

    /// Builds a serialisable [`RegistryEntry`] from this Knot's current state.
    pub fn registry_entry(&self) -> RegistryEntry {
        RegistryEntry {
            id: self.id,
            local_id: self.local_id,
            meta: self.meta.clone(),
            role: self.role.clone(),
            level: self.level,
            parent_id: self.parent_id,
            class_id: self.class_id(),
            child_ids: self.children.keys().cloned().collect(),
        }
    }

    // ── Persistent-store swap ─────────────────────────────────────────────────

    /// Replaces the persistent backend with `store`.
    ///
    /// The default backend is [`NoopPersistentStore`] (discards all writes).
    /// Pass an [`HflowStore`](crate::persistence::HflowStore) to enable
    /// SQLite-backed durability.
    pub fn set_persistent_store(&mut self, store: Arc<dyn PersistentStore>) {
        self.persistent = store;
    }

    // ── State management ──────────────────────────────────────────────────────

    /// Reads a **volatile** property by `ObjectID.PropertyID` key.
    pub fn get(&self, key: &StateKey) -> Option<&serde_json::Value> {
        self.volatile.get(key)
    }

    /// Writes a **volatile** property by `ObjectID.PropertyID` key.
    pub fn set(&mut self, key: StateKey, value: serde_json::Value) {
        self.volatile.set(key, value);
    }

    /// Loads a **persistent** property from the configured backend.
    pub async fn load(&self, key: &StateKey) -> Option<serde_json::Value> {
        self.persistent.load(key).await
    }

    /// Saves a **persistent** property to the configured backend.
    pub async fn save(&self, key: StateKey, value: serde_json::Value) {
        self.persistent.save(key, value).await;
    }
}

// ── Display ───────────────────────────────────────────────────────────────────

impl std::fmt::Display for Knot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Knot {{ id: {}, role: {}, level: {}, name: {:?} }}",
            self.id, self.role, self.level, self.meta.name
        )
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::action::TransformEngine;
    use crate::core::class::{ClassDefinition, DataType, PropertySchema};
    use serde_json::json;
    use std::time::Duration;
    use tokio::sync::watch;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn hub(local_id: LocalId) -> (Knot, broadcast::Receiver<KnotEvent>) {
        Knot::new(
            local_id,
            CellMeta::with_description("TestHub", "hub for tests"),
            KnotRole::Hub,
            0,
            None,
            None,
        )
    }

    fn sensor_class() -> ClassDefinition {
        ClassDefinition::new("Sensor")
            .with_property(
                PropertySchema::new("temperature", DataType::Number)
                    .with_default(json!(20.0))
                    .with_unit("°C"),
            )
            .with_property(
                PropertySchema::new("active", DataType::Boolean).with_default(json!(true)),
            )
    }

    fn packet_to(target_id: Uuid, sender_id: Uuid, priority: u8) -> Packet {
        Packet::new(
            sender_id,
            target_id,
            priority,
            Duration::from_secs(30),
            json!({ "test": true }),
        )
    }

    fn expired_packet_to(target_id: Uuid) -> Packet {
        Packet::new(Uuid::new_v4(), target_id, 128, Duration::ZERO, json!(null))
    }

    // ── Identity ──────────────────────────────────────────────────────────────

    #[test]
    fn new_knots_have_unique_uuids() {
        let (a, _) = hub(1);
        let (b, _) = hub(2);
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn knot_preserves_local_id_and_role() {
        let (k, _) = Knot::new(42, CellMeta::new("X"), KnotRole::Action, 2, None, None);
        assert_eq!(k.local_id(), 42);
        assert_eq!(k.role(), &KnotRole::Action);
        assert_eq!(k.level(), 2);
        assert!(k.parent_id().is_none());
    }

    #[test]
    fn from_parts_preserves_uuid() {
        let id = Uuid::new_v4();
        let (k, _) = Knot::from_parts(
            id,
            7,
            CellMeta::new("Restored"),
            KnotRole::Object,
            1,
            None,
            None,
        );
        assert_eq!(k.id(), id);
    }

    // ── Variant constructors ──────────────────────────────────────────────────

    #[test]
    fn new_object_initialises_class_defaults() {
        let class = sensor_class();
        let (obj, _) = Knot::new_object(1, CellMeta::new("Obj"), Some(&class), 1, None, None);
        assert_eq!(obj.class_id(), Some(class.id));
        assert!(obj.shared_state().is_some());
    }

    #[test]
    fn new_class_stores_definition() {
        let class = sensor_class();
        let class_id = class.id;
        let (ck, _) = Knot::new_class(1, CellMeta::new("SensorClass"), class, 1, None, None);
        assert_eq!(ck.class_id(), Some(class_id));
        assert_eq!(ck.variant_name(), "Class");
    }

    #[test]
    fn new_action_has_correct_variant() {
        let (ak, _) =
            Knot::new_action(1, CellMeta::new("Act"), Arc::new(EchoEngine), 1, None, None);
        assert_eq!(ak.variant_name(), "Action");
    }

    // ── Child management ──────────────────────────────────────────────────────

    #[test]
    fn attach_and_detach_child() {
        let (mut parent, _) = hub(0);
        let (child, _) = Knot::new(
            1,
            CellMeta::new("Child"),
            KnotRole::Action,
            1,
            Some(parent.id()),
            None,
        );
        let child_id = child.id();
        parent.attach_child(child.as_child_handle());
        let detached = parent.detach_child(&child_id);
        assert!(detached.is_some());
        assert_eq!(detached.unwrap().id, child_id);
    }

    // ── Routing ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn handle_packet_local_delivery() {
        let (mut knot, mut events) = hub(1);
        knot.handle_packet(packet_to(knot.id(), Uuid::new_v4(), 128))
            .await;
        let event = events.recv().await.expect("event");
        assert!(matches!(event, KnotEvent::PacketReceived { .. }));
    }

    #[tokio::test]
    async fn handle_packet_ttl_expired_emits_dead_letter() {
        let (mut knot, mut events) = hub(1);
        knot.handle_packet(expired_packet_to(knot.id())).await;
        let event = events.recv().await.expect("event");
        assert!(matches!(
            event,
            KnotEvent::DeadLetter {
                reason: DeadLetterReason::TtlExpired,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn handle_packet_forwards_down_to_child() {
        let (mut parent, mut events) = hub(0);
        let (child, _) = Knot::new(
            1,
            CellMeta::new("Child"),
            KnotRole::Action,
            1,
            Some(parent.id()),
            None,
        );
        let child_id = child.id();
        let child_inbox = child.inbox();
        parent.attach_child(child.as_child_handle());

        parent
            .handle_packet(packet_to(child_id, Uuid::new_v4(), 200))
            .await;

        let event = events.recv().await.expect("event");
        assert!(matches!(event, KnotEvent::PacketForwardedDown { .. }));
        assert_eq!(child_inbox.len().await, 1);
    }

    #[tokio::test]
    async fn handle_packet_forwards_up_to_parent() {
        let (parent, _) = hub(0);
        let parent_inbox = parent.inbox();
        let (mut child, mut child_events) = Knot::new(
            1,
            CellMeta::new("Child"),
            KnotRole::Action,
            1,
            Some(parent.id()),
            Some(Arc::clone(&parent_inbox)),
        );

        child
            .handle_packet(packet_to(Uuid::new_v4(), child.id(), 50))
            .await;

        let event = child_events.recv().await.expect("event");
        assert!(matches!(event, KnotEvent::PacketForwardedUp { .. }));
        assert_eq!(parent_inbox.len().await, 1);
    }

    #[tokio::test]
    async fn handle_packet_no_route_emits_dead_letter() {
        let (mut root, mut events) = hub(0);
        root.handle_packet(packet_to(Uuid::new_v4(), Uuid::new_v4(), 1))
            .await;
        let event = events.recv().await.expect("event");
        assert!(matches!(
            event,
            KnotEvent::DeadLetter {
                reason: DeadLetterReason::NoRoute,
                ..
            }
        ));
    }

    // ── Object execute ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn object_set_updates_shared_state() {
        let class = sensor_class();
        let (mut obj, _) = Knot::new_object(1, CellMeta::new("Obj"), Some(&class), 1, None, None);
        let obj_id = obj.id();

        let set_pkt = Packet::new(
            Uuid::new_v4(),
            obj_id,
            128,
            Duration::from_secs(5),
            json!({ "op": "set", "property": "temperature", "value": 99.5 }),
        );
        obj.handle_packet(set_pkt).await;

        assert_eq!(obj.get_property("temperature").await, Some(json!(99.5)));
    }

    #[tokio::test]
    async fn object_get_queues_response_in_inbox() {
        let class = sensor_class();
        let (mut obj, _) = Knot::new_object(1, CellMeta::new("Obj"), Some(&class), 1, None, None);
        let obj_id = obj.id();
        let inbox = obj.inbox();
        let sender = Uuid::new_v4();

        let get_pkt = Packet::new(
            sender,
            obj_id,
            128,
            Duration::from_secs(5),
            json!({ "op": "get", "property": "temperature" }),
        );
        obj.handle_packet(get_pkt).await;

        // execute() pushes the "get_response" back into the inbox.
        let response = inbox.try_pop().await;
        assert!(response.is_some());
        assert_eq!(
            response.unwrap().payload.get("op").and_then(|v| v.as_str()),
            Some("get_response")
        );
    }

    #[tokio::test]
    async fn object_set_many_updates_multiple_properties() {
        let class = sensor_class();
        let (mut obj, _) = Knot::new_object(1, CellMeta::new("Obj"), Some(&class), 1, None, None);
        let obj_id = obj.id();

        let pkt = Packet::new(
            Uuid::new_v4(),
            obj_id,
            128,
            Duration::from_secs(5),
            json!({ "op": "set_many", "properties": { "temperature": 55.0, "active": false } }),
        );
        obj.handle_packet(pkt).await;

        assert_eq!(obj.get_property("temperature").await, Some(json!(55.0)));
        assert_eq!(obj.get_property("active").await, Some(json!(false)));
    }

    #[tokio::test]
    async fn object_initialised_with_class_defaults() {
        let class = sensor_class();
        let (obj, _) = Knot::new_object(1, CellMeta::new("Obj"), Some(&class), 1, None, None);

        assert_eq!(obj.get_property("temperature").await, Some(json!(20.0)));
        assert_eq!(obj.get_property("active").await, Some(json!(true)));
    }

    // ── Action execute ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn action_echo_engine_queues_response() {
        let (mut act, mut events) =
            Knot::new_action(1, CellMeta::new("Act"), Arc::new(EchoEngine), 1, None, None);
        let act_id = act.id();
        let inbox = act.inbox();
        let sender = Uuid::new_v4();

        let pkt = Packet::new(
            sender,
            act_id,
            200,
            Duration::from_secs(5),
            json!({ "msg": "ping" }),
        );
        act.handle_packet(pkt).await;

        // Should emit PacketReceived then ActionExecuted.
        let _received = events.recv().await.expect("PacketReceived");

        // The echo response is addressed back to `sender` → queued in inbox for routing.
        let response = inbox.try_pop().await;
        assert!(response.is_some(), "expected echo response in inbox");
        let rsp = response.unwrap();
        assert_eq!(rsp.target_id, sender);
        assert_eq!(rsp.payload, json!({ "msg": "ping" }));
    }

    #[tokio::test]
    async fn action_transform_engine_applies_closure() {
        let engine = TransformEngine::new("double", |v| {
            let n = v.get("n").and_then(|x| x.as_f64()).unwrap_or(0.0);
            json!({ "n": n * 2.0 })
        });
        let (mut act, _) =
            Knot::new_action(1, CellMeta::new("Act"), Arc::new(engine), 1, None, None);
        let act_id = act.id();
        let inbox = act.inbox();

        act.handle_packet(Packet::new(
            Uuid::new_v4(),
            act_id,
            100,
            Duration::from_secs(5),
            json!({ "n": 7.0 }),
        ))
        .await;

        let rsp = inbox.try_pop().await.expect("response");
        assert_eq!(rsp.payload, json!({ "n": 14.0 }));
    }

    // ── link_object ───────────────────────────────────────────────────────────

    #[test]
    fn link_object_adds_id_to_action() {
        let (mut act, _) =
            Knot::new_action(1, CellMeta::new("Act"), Arc::new(EchoEngine), 1, None, None);
        let obj_id = Uuid::new_v4();
        act.link_object(obj_id);
        act.link_object(obj_id); // idempotent

        if let KnotVariant::Action {
            linked_object_ids, ..
        } = &act.variant
        {
            assert_eq!(linked_object_ids.len(), 1);
            assert_eq!(linked_object_ids[0], obj_id);
        } else {
            panic!("expected Action variant");
        }
    }

    // ── registry_entry ────────────────────────────────────────────────────────

    #[test]
    fn registry_entry_reflects_knot_state() {
        let class = sensor_class();
        let class_id = class.id;
        let (mut obj, _) = Knot::new_object(5, CellMeta::new("Obj"), Some(&class), 2, None, None);
        let child_id = Uuid::new_v4();
        // Attach a dummy child so child_ids is non-empty.
        let dummy_inbox = Inbox::new();
        obj.attach_child(ChildHandle {
            id: child_id,
            inbox: dummy_inbox,
        });

        let entry = obj.registry_entry();
        assert_eq!(entry.id, obj.id());
        assert_eq!(entry.level, 2);
        assert_eq!(entry.role, KnotRole::Object);
        assert_eq!(entry.class_id, Some(class_id));
        assert!(entry.child_ids.contains(&child_id));
    }

    // ── Run loop ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_emits_started_and_stopped() {
        let (mut knot, mut events) = hub(0);
        let (tx, rx) = watch::channel(false);

        let h = tokio::spawn(async move {
            knot.run(rx).await;
        });
        let started = events.recv().await.expect("started");
        assert!(matches!(started, KnotEvent::Started { .. }));

        let _ = tx.send(true);
        h.await.expect("task ok");

        let stopped = events.recv().await.expect("stopped");
        assert!(matches!(stopped, KnotEvent::Stopped { .. }));
    }

    #[tokio::test]
    async fn run_processes_packets_from_inbox() {
        let (mut knot, mut events) = hub(0);
        let inbox = knot.inbox();
        let knot_id = knot.id();
        let (tx, rx) = watch::channel(false);

        inbox.push(packet_to(knot_id, Uuid::new_v4(), 255)).await;
        tokio::spawn(async move {
            knot.run(rx).await;
        });

        let _ = events.recv().await; // Started
        let event = events.recv().await.expect("packet event");
        assert!(matches!(event, KnotEvent::PacketReceived { .. }));

        let _ = tx.send(true);
    }

    // ── Volatile state ────────────────────────────────────────────────────────

    #[test]
    fn volatile_get_set_round_trip() {
        let (mut knot, _) = hub(0);
        let key = StateKey::new(knot.id(), "temperature");
        knot.set(key.clone(), json!(42.5));
        assert_eq!(knot.get(&key), Some(&json!(42.5)));
    }

    #[test]
    fn volatile_get_missing_returns_none() {
        let (knot, _) = hub(0);
        assert!(
            knot.get(&StateKey::new(Uuid::new_v4(), "missing"))
                .is_none()
        );
    }

    #[tokio::test]
    async fn persistent_load_noop_returns_none() {
        let (knot, _) = hub(0);
        assert!(knot.load(&StateKey::new(knot.id(), "data")).await.is_none());
    }
}
