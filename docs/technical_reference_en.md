# HubFlow Technical Reference

> Version 0.2.0 — Rust Core

## Table of Contents
1. [Overview](#1-overview)
2. [Architecture — Solar System Topology](#2-architecture--solar-system-topology)
3. [Core Types](#3-core-types)
4. [Routing Protocol](#4-routing-protocol)
5. [The Triumvirat — Class, Object, Action](#5-the-triumvirat--class-object-action)
6. [Packet Schema](#6-packet-schema)
7. [State Management](#7-state-management)
8. [The .hflow Database Schema](#8-the-hflow-database-schema)
9. [Frontend API (WebSocket + REST)](#9-frontend-api-websocket--rest)
10. [Glossary](#10-glossary)

---

## 1. Overview

HubFlow is a decentralised, event-driven system built around a hierarchy of autonomous **Knots**. Each Knot is an independently addressable unit with its own async event loop, priority inbox, and pluggable logic engine. Communication is purely packet-based: no shared memory, no direct function calls across Knot boundaries.

Key design goals:

- **Decentralisation** — no single point of failure; every Knot can route independently.
- **Priority-first processing** — high-priority packets skip the queue.
- **Observability** — every routing decision emits a structured event.
- **Portability** — the entire workspace is a single `.hflow` SQLite file.

---

## 2. Architecture — Solar System Topology

HubFlow uses a tree topology metaphorically described as a "Solar System":

```
Level 0   ┌────────────────────┐
          │      Root Hub       │   ← The "star"
          └─────────┬──────────┘
                    │
Level 1   ┌─────────┼──────────┐
          ▼         ▼          ▼
        Hub A     Hub B      Hub C   ← "Planets"
        /  |  \
Level 2  Act  Obj  Cls              ← "Satellites"
```

| Level | Role   | Metaphor  | Responsibility                    |
|-------|--------|-----------|-----------------------------------|
| 0     | Hub    | Star      | System root, global routing       |
| 1+    | Hub    | Planet    | Sub-system coordinator            |
| Any   | Action | Satellite | Executable logic                  |
| Any   | Object | Satellite | Structured data                   |
| Any   | Class  | Satellite | Schema / blueprint                |

### KnotRole

```rust
pub enum KnotRole { Hub, Action, Object, Class }
```

The role is for **semantic classification only**. All roles participate in the identical routing protocol.

---

## 3. Core Types

### Cell (identity primitive)

```
LocalId   = u64            // fast in-process lookup
CellMeta  = { name, description? }
Uuid      = UUID v4        // globally unique, cross-system identity
```

### Knot

The central abstraction. Every Knot has:

| Field        | Type                        | Purpose                            |
|--------------|-----------------------------|------------------------------------|
| `id`         | `Uuid`                      | Global identity                    |
| `local_id`   | `u64`                       | In-process index                   |
| `meta`       | `CellMeta`                  | Name + description                 |
| `role`       | `KnotRole`                  | Semantic classification            |
| `level`      | `u32`                       | Tree depth                         |
| `parent_id`  | `Option<Uuid>`              | Parent Knot UUID                   |
| `inbox`      | `Arc<Inbox>`                | Async priority queue               |
| `variant`    | `KnotVariant`               | Role-specific state                |
| `volatile`   | `InMemoryStore`             | RAM state store                    |
| `persistent` | `Arc<dyn PersistentStore>`  | Durable backend                    |
| `registry`   | `Option<Arc<KnotRegistry>>` | Cross-Knot directory               |

### Inbox

The `Inbox` is a `BinaryHeap<PrioritizedPacket>` protected by a `tokio::sync::Mutex` and notified via a `tokio::sync::Notify`. It guarantees:

- **Priority ordering**: packets with `priority = 255` are served before `priority = 0`.
- **FIFO within same priority**: earlier arrivals are served first.
- **Cancellation safety**: `pop()` uses the `Notify` permit mechanism — no wakeup is ever lost.

---

## 4. Routing Protocol

### Rules (evaluated in order)

1. **TTL check** — if `now - created_at > ttl`, emit `DeadLetter(TtlExpired)` and drop.
2. **Local delivery** — if `target_id == self.id`, call `execute()`.
3. **Forward down** — if `target_id ∈ children`, push to child's inbox.
4. **Forward up** — if parent exists, push to parent's inbox.
5. **No route** — emit `DeadLetter(NoRoute)` and drop.

### Shutdown

Knots use a `tokio::sync::watch::Receiver<bool>`. Send `true` or drop the sender to trigger a clean stop. The run loop completes the current packet before stopping.

### Event Bus

Every Knot exposes a `broadcast::Sender<KnotEvent>`. Subscribers receive:

| Event                 | Trigger                              |
|-----------------------|--------------------------------------|
| `Started`             | Run loop begins                      |
| `Stopped`             | Run loop ends                        |
| `PacketReceived`      | Local delivery                       |
| `PacketForwardedDown` | Child routing                        |
| `PacketForwardedUp`   | Parent escalation                    |
| `DeadLetter`          | Undeliverable packet                 |
| `ObjectStateChanged`  | Object property written              |
| `ActionExecuted`      | Action engine completed              |

---

## 5. The Triumvirat — Class, Object, Action

### Class Knot

Stores a `ClassDefinition` — an ordered list of `PropertySchema` entries:

```rust
PropertySchema {
    name:          String,
    data_type:     DataType,            // String | Number | Boolean | Json
    default_value: Option<Value>,
    unit:          Option<String>,
    description:   Option<String>,
}
```

When a Class Knot receives a packet, it responds with its schema (`op: "class_schema"`).

### Object Knot

Holds a `HashMap<String, Value>` (the **shared state**) initialised from a Class's `default_state()`. Handles four operations via packet payload:

| `op`        | Description         | Response `op`       |
|-------------|---------------------|---------------------|
| `get`       | Read one property   | `get_response`      |
| `set`       | Write one property  | `set_ack`           |
| `set_many`  | Write multiple      | `set_many_ack`      |
| `get_all`   | Read all            | `get_all_response`  |

State changes emit `ObjectStateChanged` events and are synced to the `KnotRegistry` for Action Knots to read.

### Action Knot

Runs a `LogicEngine` on every locally-delivered packet:

```rust
pub trait LogicEngine: Send + Sync {
    fn name(&self) -> &str;
    fn execute<'a>(&'a self, ctx: &'a ActionContext) -> BoxFuture<'a, ActionOutput>;
}
```

**Built-in engines:**

| Engine            | Behaviour                                               |
|-------------------|---------------------------------------------------------|
| `EchoEngine`      | Reflects packet back to sender                          |
| `TransformEngine` | Applies user closure to payload                         |
| `ScriptEngine`    | Placeholder for Python (pyo3) — currently no-op        |

The engine receives `ActionContext { self_id, incoming: Packet, objects: HashMap<Uuid, ObjectSnapshot> }` and returns `ActionOutput { updates, packets, logs }`.

---

## 6. Packet Schema

```
Packet {
    target_id : Uuid          // destination Knot
    sender_id : Uuid          // originating Knot
    priority  : u8            // 0 (low) … 255 (high)
    ttl       : Duration      // serialised as milliseconds (u64)
    payload   : JSON Value    // arbitrary structured data
}
```

### Object operation payloads

```json
// Read one property
{ "op": "get",      "property": "temperature" }

// Write one property
{ "op": "set",      "property": "temperature", "value": 42.5 }

// Batch write
{ "op": "set_many", "properties": { "temperature": 42.5, "active": true } }

// Read all properties
{ "op": "get_all" }
```

---

## 7. State Management

### Two-tier model

| Tier       | Trait              | Default impl           | Lifetime         |
|------------|--------------------|------------------------|------------------|
| Volatile   | `VolatileStore`    | `InMemoryStore`        | Process lifetime |
| Persistent | `PersistentStore`  | `NoopPersistentStore`  | Cross-restart    |

### ObjectID.PropertyID addressing

```
StateKey { object_id: Uuid, property_id: String }
Display:  "<uuid>.<property_name>"
```

### PersistentStore (object-safe)

```rust
pub trait PersistentStore: Send + Sync {
    fn load<'a>(&'a self, key: &'a StateKey)           -> PinBoxFuture<'a, Option<Value>>;
    fn save<'a>(&'a self, key: StateKey, value: Value) -> PinBoxFuture<'a, ()>;
    fn delete<'a>(&'a self, key: &'a StateKey)         -> PinBoxFuture<'a, ()>;
}
```

---

## 8. The .hflow Database Schema

A `.hflow` file is a SQLite database. All database operations are wrapped in `spawn_blocking` calls to avoid blocking the Tokio async executor.

### Table: `knots`

```sql
CREATE TABLE knots (
    id          TEXT    PRIMARY KEY,   -- UUID
    local_id    INTEGER NOT NULL,
    name        TEXT    NOT NULL,
    description TEXT,
    role        TEXT    NOT NULL,      -- "Hub" | "Action" | "Object" | "Class"
    level       INTEGER NOT NULL,
    parent_id   TEXT,                  -- NULL for root
    class_id    TEXT                   -- NULL unless Object
);
```

### Table: `properties`

```sql
CREATE TABLE properties (
    object_id   TEXT NOT NULL,
    property_id TEXT NOT NULL,
    value       TEXT NOT NULL,         -- JSON-encoded
    PRIMARY KEY (object_id, property_id)
);
```

### Table: `class_schemas`

```sql
CREATE TABLE class_schemas (
    class_id       TEXT    NOT NULL,
    property_name  TEXT    NOT NULL,
    data_type      TEXT    NOT NULL,   -- "string" | "number" | "boolean" | "json"
    default_value  TEXT,               -- JSON or NULL
    unit           TEXT,
    description    TEXT,
    sort_order     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (class_id, property_name)
);
```

### Table: `links`

```sql
CREATE TABLE links (
    id          TEXT PRIMARY KEY,      -- UUID
    source_id   TEXT NOT NULL,         -- Knot UUID
    target_id   TEXT NOT NULL,         -- Knot UUID
    link_type   TEXT NOT NULL          -- e.g. "action_to_object", "data_flow"
);
```

---

## 9. Frontend API (WebSocket + REST)

The `ApiServer` (powered by axum 0.7) exposes both a REST interface and a persistent WebSocket connection for real-time event streaming.

### REST Endpoints

| Method | Path                   | Response                               |
|--------|------------------------|----------------------------------------|
| `GET`  | `/api/graph`           | JSON array of all `RegistryEntry`      |
| `GET`  | `/api/knot/:id`        | Single `RegistryEntry` or 404          |
| `GET`  | `/api/knot/:id/state`  | Object property map or 404             |
| `POST` | `/api/packet`          | `{"status": "queued"}`                 |

### POST /api/packet — Request Body

```json
{
  "target_id": "<uuid>",
  "sender_id": "<uuid>",
  "priority":  200,
  "ttl_ms":    5000,
  "payload":   { }
}
```

| Field       | Type     | Required | Default        | Notes                            |
|-------------|----------|----------|----------------|----------------------------------|
| `target_id` | `string` | Yes      | —              | Destination Knot UUID            |
| `sender_id` | `string` | No       | nil UUID       | Originating Knot UUID            |
| `priority`  | `u8`     | No       | `128`          | 0 (lowest) to 255 (highest)      |
| `ttl_ms`    | `u64`    | No       | `5000`         | Time-to-live in milliseconds     |
| `payload`   | `object` | No       | `{}`           | Arbitrary JSON payload           |

### WebSocket — GET /api/ws

The WebSocket endpoint provides a bidirectional channel. The server pushes structured events to all connected clients; clients may send JSON-RPC 2.0 commands.

#### Server → Client: EventEnvelope

```json
{
  "event_type": "ObjectStateChanged",
  "payload": {
    "knot_id":  "<uuid>",
    "property": "temperature",
    "value":    42.5
  }
}
```

All event types mirror the `KnotEvent` enum documented in [Section 4](#4-routing-protocol). Clients should handle unknown `event_type` values gracefully to remain forward-compatible.

#### Client → Server: JSON-RPC 2.0

```json
{ "jsonrpc": "2.0", "method": "send_packet", "params": { "target_id": "<uuid>", "payload": {} }, "id": 1 }
{ "jsonrpc": "2.0", "method": "get_graph",   "id": 2 }
```

**Supported methods:**

| Method        | Params                         | Result                         |
|---------------|--------------------------------|--------------------------------|
| `send_packet` | Same fields as POST /api/packet | `{"status": "queued"}`        |
| `get_graph`   | _(none)_                       | Array of `RegistryEntry`       |

---

## 10. Glossary

| Term              | Definition                                                                              |
|-------------------|-----------------------------------------------------------------------------------------|
| **Knot**          | The fundamental independently addressable unit of HubFlow.                              |
| **Hub**           | A Knot that coordinates child Knots; the "planet" in the solar system metaphor.         |
| **Satellite**     | An Action, Object, or Class Knot attached to a Hub.                                     |
| **Inbox**         | The async priority queue owned by each Knot.                                            |
| **Packet**        | The sole unit of communication between Knots; contains target, sender, priority, TTL, and payload. |
| **TTL**           | Time-To-Live: maximum packet lifetime before it is dead-lettered.                       |
| **Dead Letter**   | A packet that could not be delivered (TTL expired or no route found).                   |
| **LogicEngine**   | A pluggable async computation hook attached to Action Knots.                            |
| **ClassDefinition** | A schema describing the set of typed properties an Object Knot may hold.             |
| **KnotRegistry**  | The in-memory directory mapping UUIDs to live Knot handles and their current state.     |
| **.hflow**        | A portable SQLite workspace file containing the full topology, schemas, and persisted state. |
| **Triumvirat**    | The three non-Hub Knot roles: Class, Object, and Action.                                |
| **EventBus**      | The per-Knot `broadcast::Sender<KnotEvent>` through which all routing decisions are observable. |