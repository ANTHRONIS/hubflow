//! The Knot — the evolved, fully-functional core unit of HubFlow.
//!
//! A [`Knot`] is the central abstraction of the HubFlow ecosystem. It extends
//! the identity model of the original `Cell` with a topology role, an async
//! priority inbox, local-first packet routing, and a two-tier state store.
//!
//! # Topology (Solar System model)
//!
//! ```text
//!              ┌─────────────────────────────────────┐
//!              │            Level 0 — Root            │
//!              │        (the central "star")          │
//!              └──────────────┬──────────────────────┘
//!                             │
//!          ┌──────────────────┼──────────────────┐
//!          ▼                  ▼                  ▼
//!       Hub A              Hub B              Hub C           ← Levels 1+
//!      (planet)           (planet)           (planet)
//!      /  |  \
//!  Action Object Class                                        ← Satellites
//! ```
//!
//! # Routing rules (local-first)
//!
//! 1. `target == self`        → execute locally.
//! 2. `target` is a child    → forward **down**.
//! 3. `target` is unknown    → forward **up** to parent.
//! 4. TTL expired            → drop + emit [`KnotEvent::DeadLetter`].
//! 5. No parent & unknown    → emit [`KnotEvent::DeadLetter`] with [`DeadLetterReason::NoRoute`].

use crate::core::cell::{CellMeta, LocalId};
use crate::core::event::{DeadLetterReason, KnotEvent};
use crate::core::inbox::Inbox;
use crate::core::packet::Packet;
use crate::core::state::{
    InMemoryStore, NoopPersistentStore, PersistentStore, StateKey, VolatileStore,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

// ── Role ──────────────────────────────────────────────────────────────────────

/// The functional role of a Knot within the HubFlow topology.
///
/// Roles determine the *semantic* purpose of a Knot, not its routing behaviour.
/// Every role participates in the same routing protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnotRole {
    /// A Hub coordinates a sub-system of Satellites (the "planet" in the
    /// Solar System metaphor). Hubs typically have children.
    Hub,

    /// An Action Satellite encapsulates executable logic (a function, handler,
    /// or micro-service).
    Action,

    /// An Object Satellite holds structured, mutable data.
    Object,

    /// A Class Satellite acts as a blueprint or template for spawning new Knots.
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

// ── Child handle ──────────────────────────────────────────────────────────────

/// A lightweight, cloneable reference to a child Knot's inbox.
///
/// The parent Knot holds one [`ChildHandle`] per registered child and uses it
/// to forward packets downward without holding a lock on the child itself.
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
/// Use [`Knot::new`] to create a Knot and receive its event-bus subscriber:
///
/// ```rust,no_run
/// use hubflow::core::cell::CellMeta;
/// use hubflow::core::knot::{Knot, KnotRole};
///
/// let (mut hub, mut events) = Knot::new(
///     1,
///     CellMeta::with_description("MainHub", "Root hub of the system"),
///     KnotRole::Hub,
///     0,       // level
///     None,    // no parent (this is the root)
///     None,    // no parent inbox
/// );
/// ```
///
/// # Lifecycle
///
/// ```rust,no_run
/// # use tokio::sync::watch;
/// let (shutdown_tx, shutdown_rx) = watch::channel(false);
///
/// tokio::spawn(async move {
///     hub.run(shutdown_rx).await;
/// });
///
/// // … later …
/// let _ = shutdown_tx.send(true);
/// ```
pub struct Knot {
    // ── Identity (from Cell) ─────────────────────────────────────────────────
    /// System-wide unique identifier (UUID v4).
    id: Uuid,

    /// Node-local identifier for fast in-process lookup / indexing.
    local_id: LocalId,

    /// Human-readable metadata (name + optional description).
    meta: CellMeta,

    // ── Topology ─────────────────────────────────────────────────────────────
    /// Semantic role of this Knot in the HubFlow ecosystem.
    role: KnotRole,

    /// Depth in the topology tree. Level 0 = root ("the star").
    level: u32,

    /// UUID of the parent Knot, or `None` for the root.
    parent_id: Option<Uuid>,

    // ── Communication ────────────────────────────────────────────────────────
    /// This Knot's own priority inbox. Shared as [`Arc`] so other Knots can
    /// push packets into it without borrowing `self`.
    inbox: Arc<Inbox>,

    /// Inbox of the parent Knot, used to forward packets upward.
    parent_inbox: Option<Arc<Inbox>>,

    /// Routing table: maps child UUIDs to their inbox handles.
    children: HashMap<Uuid, ChildHandle>,

    // ── Events ───────────────────────────────────────────────────────────────
    /// Broadcast sender for the Knot's observable event stream.
    event_tx: broadcast::Sender<KnotEvent>,

    // ── State ────────────────────────────────────────────────────────────────
    /// Volatile (in-process RAM) state store.
    volatile: InMemoryStore,

    /// Persistent (cross-restart durable) state store.
    persistent: NoopPersistentStore,
}

impl Knot {
    // ── Construction ──────────────────────────────────────────────────────────

    /// Creates a new [`Knot`], generating a fresh UUID v4 automatically.
    ///
    /// Returns the Knot and a [`broadcast::Receiver`] pre-subscribed to its
    /// event bus. Additional receivers can be obtained via [`Knot::event_bus`].
    ///
    /// # Arguments
    ///
    /// * `local_id`      — Caller-assigned in-process identifier.
    /// * `meta`          — Human-readable name and description.
    /// * `role`          — Semantic role (`Hub`, `Action`, `Object`, `Class`).
    /// * `level`         — Depth in the topology tree (0 = root).
    /// * `parent_id`     — UUID of the parent Knot, or `None` for the root.
    /// * `parent_inbox`  — Shared inbox of the parent, used for upward routing.
    pub fn new(
        local_id: LocalId,
        meta: CellMeta,
        role: KnotRole,
        level: u32,
        parent_id: Option<Uuid>,
        parent_inbox: Option<Arc<Inbox>>,
    ) -> (Self, broadcast::Receiver<KnotEvent>) {
        let (event_tx, event_rx) = broadcast::channel(256);

        let knot = Self {
            id: Uuid::new_v4(),
            local_id,
            meta,
            role,
            level,
            parent_id,
            inbox: Inbox::new(),
            parent_inbox,
            children: HashMap::new(),
            event_tx,
            volatile: InMemoryStore::default(),
            persistent: NoopPersistentStore::default(),
        };

        (knot, event_rx)
    }

    /// Reconstructs a [`Knot`] from persisted identity components.
    ///
    /// Prefer [`Knot::new`] for fresh Knots; use this only when restoring
    /// state from a database or snapshot.
    pub fn from_parts(
        id: Uuid,
        local_id: LocalId,
        meta: CellMeta,
        role: KnotRole,
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
            volatile: InMemoryStore::default(),
            persistent: NoopPersistentStore::default(),
        };

        (knot, event_rx)
    }

    // ── Child management ──────────────────────────────────────────────────────

    /// Registers a child Knot in this Knot's routing table.
    ///
    /// After registration, packets addressed to `handle.id` will be forwarded
    /// directly to the child's inbox instead of being escalated to the parent.
    pub fn attach_child(&mut self, handle: ChildHandle) {
        tracing::debug!(
            knot_id = %self.id,
            child_id = %handle.id,
            "child attached"
        );
        self.children.insert(handle.id, handle);
    }

    /// Removes a child from this Knot's routing table and returns its handle.
    ///
    /// Returns `None` if `child_id` was not registered.
    pub fn detach_child(&mut self, child_id: &Uuid) -> Option<ChildHandle> {
        let handle = self.children.remove(child_id);
        if handle.is_some() {
            tracing::debug!(knot_id = %self.id, child_id = %child_id, "child detached");
        }
        handle
    }

    /// Returns a [`ChildHandle`] pointing at this Knot's own inbox.
    ///
    /// Pass this to a parent Knot's [`attach_child`](Knot::attach_child) to
    /// register `self` as a routable child.
    pub fn as_child_handle(&self) -> ChildHandle {
        ChildHandle {
            id: self.id,
            inbox: Arc::clone(&self.inbox),
        }
    }

    // ── Packet entry-point ────────────────────────────────────────────────────

    /// Pushes a packet into this Knot's inbox from the outside.
    ///
    /// This is the primary way for external callers (tests, other Knots, …)
    /// to deliver a packet without going through the routing logic.
    pub async fn send(&self, packet: Packet) {
        self.inbox.push(packet).await;
    }

    // ── Routing ───────────────────────────────────────────────────────────────

    /// Routes a packet according to HubFlow's **local-first** rules:
    ///
    /// | Condition                        | Action                          |
    /// |----------------------------------|---------------------------------|
    /// | `target == self.id`              | Execute locally                 |
    /// | `target` ∈ `children`            | Forward ↓ (down to child)       |
    /// | `target` ∉ `children` + parent   | Forward ↑ (up to parent)        |
    /// | TTL expired                      | Drop + emit `DeadLetter`        |
    /// | No parent & target unknown       | Drop + emit `DeadLetter`        |
    pub async fn handle_packet(&mut self, packet: Packet) {
        // ── 1. TTL guard ─────────────────────────────────────────────────────
        if packet.is_expired() {
            tracing::warn!(
                knot_id   = %self.id,
                target_id = %packet.target_id,
                sender_id = %packet.sender_id,
                "packet TTL expired — emitting dead-letter"
            );
            let _ = self.event_tx.send(KnotEvent::DeadLetter {
                packet,
                reason: DeadLetterReason::TtlExpired,
            });
            return;
        }

        // ── 2. Local delivery ─────────────────────────────────────────────────
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

        // ── 3. Forward down (known child) ─────────────────────────────────────
        //
        // Clone the inbox Arc *before* borrowing event_tx so the borrow
        // checker does not see overlapping mutable / immutable borrows of self.
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

        // ── 4. Forward up (unknown target → escalate to parent) ───────────────
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

        // ── 5. No route ───────────────────────────────────────────────────────
        tracing::warn!(
            knot_id   = %self.id,
            target_id = %packet.target_id,
            "no route to target — emitting dead-letter"
        );
        let _ = self.event_tx.send(KnotEvent::DeadLetter {
            packet,
            reason: DeadLetterReason::NoRoute,
        });
    }

    /// Processes a packet that has been locally delivered to this Knot.
    ///
    /// This is where role-specific dispatch will happen in a future iteration.
    /// For now it logs the arrival and emits no further events, leaving the
    /// [`KnotEvent::PacketReceived`] already sent by [`handle_packet`] as the
    /// observable signal.
    ///
    /// Override this behaviour by consuming the event bus and reacting to
    /// [`KnotEvent::PacketReceived`] outside the Knot.
    async fn execute(&mut self, packet: Packet) {
        tracing::debug!(
            knot_id   = %self.id,
            role      = %self.role,
            sender_id = %packet.sender_id,
            priority  = packet.priority,
            payload   = %packet.payload,
            "executing packet"
        );
        // TODO: dispatch to role-specific handler based on payload schema.
    }

    // ── Heartbeat (event loop) ────────────────────────────────────────────────

    /// Starts the Knot's async event loop — its **heartbeat**.
    ///
    /// The loop continuously dequeues the highest-priority packet from the
    /// inbox and routes it via [`handle_packet`](Knot::handle_packet).
    ///
    /// # Shutdown
    ///
    /// The caller supplies the *receiver* side of a
    /// [`tokio::sync::watch`] channel initialised to `false`. Sending `true`
    /// (or dropping the sender) signals a clean shutdown:
    ///
    /// ```rust,no_run
    /// # use tokio::sync::watch;
    /// let (tx, rx) = watch::channel(false);
    /// tokio::spawn(async move { knot.run(rx).await; });
    /// let _ = tx.send(true); // graceful stop
    /// ```
    ///
    /// Any packets still in the inbox when shutdown is requested are **not**
    /// drained — they are simply abandoned. A drain-on-shutdown pass can be
    /// added in a future iteration.
    pub async fn run(&mut self, mut shutdown_rx: watch::Receiver<bool>) {
        // Clone the inbox Arc so we can await `inbox.pop()` inside select!
        // without holding a borrow on `self` when `handle_packet(&mut self)`
        // is called in the same branch.
        let inbox = Arc::clone(&self.inbox);

        let _ = self.event_tx.send(KnotEvent::Started { knot_id: self.id });
        tracing::info!(
            knot_id  = %self.id,
            role     = %self.role,
            level    = self.level,
            name     = %self.meta.name,
            "knot started"
        );

        loop {
            tokio::select! {
                // Biased: always check the shutdown signal first so a burst of
                // packets cannot starve the shutdown path.
                biased;

                // ── Shutdown branch ──────────────────────────────────────────
                result = shutdown_rx.changed() => {
                    // The sender was dropped (Err) or explicitly set true.
                    if result.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }

                // ── Packet branch ────────────────────────────────────────────
                packet = inbox.pop() => {
                    self.handle_packet(packet).await;
                }
            }
        }

        let _ = self.event_tx.send(KnotEvent::Stopped { knot_id: self.id });
        tracing::info!(
            knot_id = %self.id,
            role    = %self.role,
            "knot stopped"
        );
    }

    // ── Accessors ─────────────────────────────────────────────────────────────

    /// Returns the system-wide unique identifier of this Knot.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Returns the node-local identifier of this Knot.
    pub fn local_id(&self) -> LocalId {
        self.local_id
    }

    /// Returns a reference to this Knot's human-readable metadata.
    pub fn meta(&self) -> &CellMeta {
        &self.meta
    }

    /// Returns a mutable reference to this Knot's metadata.
    pub fn meta_mut(&mut self) -> &mut CellMeta {
        &mut self.meta
    }

    /// Returns the semantic role of this Knot.
    pub fn role(&self) -> &KnotRole {
        &self.role
    }

    /// Returns the depth of this Knot in the topology tree (0 = root).
    pub fn level(&self) -> u32 {
        self.level
    }

    /// Returns the UUID of the parent Knot, or `None` if this is the root.
    pub fn parent_id(&self) -> Option<Uuid> {
        self.parent_id
    }

    /// Returns a clone of the shared inbox handle.
    ///
    /// Use this to push packets into the Knot from external code.
    pub fn inbox(&self) -> Arc<Inbox> {
        Arc::clone(&self.inbox)
    }

    /// Returns a clone of the broadcast sender for the Knot's event bus.
    ///
    /// Call `.subscribe()` on the returned sender to obtain a new
    /// [`broadcast::Receiver<KnotEvent>`].
    pub fn event_bus(&self) -> broadcast::Sender<KnotEvent> {
        self.event_tx.clone()
    }

    // ── State management ──────────────────────────────────────────────────────

    /// Reads a **volatile** (in-memory) property by `ObjectID.PropertyID` key.
    ///
    /// Returns `None` if the key has not been set.
    pub fn get(&self, key: &StateKey) -> Option<&serde_json::Value> {
        self.volatile.get(key)
    }

    /// Writes a **volatile** (in-memory) property by `ObjectID.PropertyID` key.
    ///
    /// Data is lost when the Knot stops. Use [`Knot::save`] for durable state.
    pub fn set(&mut self, key: StateKey, value: serde_json::Value) {
        self.volatile.set(key, value);
    }

    /// Loads a **persistent** property from the backing store.
    ///
    /// Returns `None` if the key does not exist or if no real backend is
    /// configured (the default [`NoopPersistentStore`] always returns `None`).
    pub async fn load(&self, key: &StateKey) -> Option<serde_json::Value> {
        self.persistent.load(key).await
    }

    /// Saves a **persistent** property to the backing store.
    ///
    /// With the default [`NoopPersistentStore`] this is a no-op. Replace the
    /// store with a real backend to enable durability.
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
    use crate::core::cell::CellMeta;
    use serde_json::json;
    use std::time::Duration;
    use tokio::sync::watch;

    // ── Helpers ───────────────────────────────────────────────────────────────

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
    fn knot_has_unique_uuid() {
        let (a, _) = hub(1);
        let (b, _) = hub(2);
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn knot_preserves_local_id_and_role() {
        let (knot, _) = Knot::new(42, CellMeta::new("X"), KnotRole::Action, 2, None, None);
        assert_eq!(knot.local_id(), 42);
        assert_eq!(knot.role(), &KnotRole::Action);
        assert_eq!(knot.level(), 2);
        assert!(knot.parent_id().is_none());
    }

    #[test]
    fn from_parts_preserves_uuid() {
        let id = Uuid::new_v4();
        let (knot, _) = Knot::from_parts(
            id,
            7,
            CellMeta::new("Restored"),
            KnotRole::Object,
            1,
            None,
            None,
        );
        assert_eq!(knot.id(), id);
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

        let handle = child.as_child_handle();
        let child_id = handle.id;

        parent.attach_child(handle);
        let detached = parent.detach_child(&child_id);
        assert!(detached.is_some());
        assert_eq!(detached.unwrap().id, child_id);
    }

    // ── Routing ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn handle_packet_local_delivery() {
        let (mut knot, mut events) = hub(1);
        let p = packet_to(knot.id(), Uuid::new_v4(), 128);

        knot.handle_packet(p).await;

        let event = events.recv().await.expect("expected an event");
        assert!(matches!(event, KnotEvent::PacketReceived { .. }));
    }

    #[tokio::test]
    async fn handle_packet_ttl_expired_emits_dead_letter() {
        let (mut knot, mut events) = hub(1);
        let p = expired_packet_to(knot.id());

        knot.handle_packet(p).await;

        let event = events.recv().await.expect("expected an event");
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

        let handle = child.as_child_handle();
        let child_id = handle.id;
        let child_inbox = child.inbox();

        parent.attach_child(handle);

        let p = packet_to(child_id, Uuid::new_v4(), 200);
        parent.handle_packet(p).await;

        // The parent should emit a ForwardedDown event.
        let event = events.recv().await.expect("expected event");
        assert!(matches!(event, KnotEvent::PacketForwardedDown { .. }));

        // The child's inbox should contain the packet.
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

        let unknown_target = Uuid::new_v4();
        let p = packet_to(unknown_target, child.id(), 50);
        child.handle_packet(p).await;

        // Child should emit a ForwardedUp event.
        let event = child_events.recv().await.expect("expected event");
        assert!(matches!(event, KnotEvent::PacketForwardedUp { .. }));

        // Parent inbox should now hold the packet.
        assert_eq!(parent_inbox.len().await, 1);
    }

    #[tokio::test]
    async fn handle_packet_no_route_emits_dead_letter() {
        // Root Knot: no parent, unknown target → DeadLetter(NoRoute)
        let (mut root, mut events) = hub(0);
        let p = packet_to(Uuid::new_v4(), Uuid::new_v4(), 1);

        root.handle_packet(p).await;

        let event = events.recv().await.expect("expected event");
        assert!(matches!(
            event,
            KnotEvent::DeadLetter {
                reason: DeadLetterReason::NoRoute,
                ..
            }
        ));
    }

    // ── Run loop ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_emits_started_and_stopped_events() {
        let (mut knot, mut events) = hub(0);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handle = tokio::spawn(async move {
            knot.run(shutdown_rx).await;
        });

        // First event should be Started.
        let started = events.recv().await.expect("started event");
        assert!(matches!(started, KnotEvent::Started { .. }));

        // Trigger shutdown.
        let _ = shutdown_tx.send(true);
        handle.await.expect("task panicked");

        // Next event should be Stopped.
        let stopped = events.recv().await.expect("stopped event");
        assert!(matches!(stopped, KnotEvent::Stopped { .. }));
    }

    #[tokio::test]
    async fn run_processes_packets_from_inbox() {
        let (mut knot, mut events) = hub(0);
        let inbox = knot.inbox();
        let knot_id = knot.id();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Send a self-addressed packet before starting the loop.
        let p = packet_to(knot_id, Uuid::new_v4(), 255);
        inbox.push(p).await;

        tokio::spawn(async move {
            knot.run(shutdown_rx).await;
        });

        // Consume Started event.
        let _ = events.recv().await;

        // The next event must be PacketReceived.
        let event = events.recv().await.expect("packet event");
        assert!(matches!(event, KnotEvent::PacketReceived { .. }));

        let _ = shutdown_tx.send(true);
    }

    // ── State ─────────────────────────────────────────────────────────────────

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
        let key = StateKey::new(Uuid::new_v4(), "missing");
        assert!(knot.get(&key).is_none());
    }

    #[tokio::test]
    async fn persistent_load_noop_returns_none() {
        let (knot, _) = hub(0);
        let key = StateKey::new(knot.id(), "data");
        assert!(knot.load(&key).await.is_none());
    }
}
