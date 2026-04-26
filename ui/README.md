# HubFlow Visual Editor

> Vue 3 · Vite · Tailwind CSS · Vue Flow · Pinia

A real-time visual editor for the HubFlow Rust backend.  
Connect, observe, and control your Knot-Graph directly from the browser.

---

## Prerequisites

| Requirement | Version |
|---|---|
| Node.js | ≥ 18 |
| npm | ≥ 9 |
| HubFlow backend | running on `127.0.0.1:8080` |

---

## Quick Start

### 1. Start the Rust backend

```bash
# From the project root (hubflow2/)
cargo run
```

The backend starts the axum API server on `http://127.0.0.1:8080`.

### 2. Start the UI dev server

```bash
# From hubflow2/ui/
npm install
npm run dev
```

Open **http://localhost:3000** in your browser.

---

## Available Scripts

| Command | Description |
|---|---|
| `npm run dev` | Start the Vite dev server with HMR |
| `npm run build` | Production build → `dist/` |
| `npm run preview` | Preview the production build locally |

---

## Architecture Overview

```
ui/src/
├── main.js                    # App bootstrap (Vue + Pinia)
├── App.vue                    # Root layout: header, canvas, inspector
├── assets/
│   └── main.css               # Tailwind + Vue Flow dark overrides
├── stores/
│   └── hubflow.js             # Pinia store: state, WebSocket, REST
└── components/
    ├── Canvas.vue             # Vue Flow graph canvas
    ├── Inspector.vue          # Right-side detail panel
    ├── ConnectionStatus.vue   # WS status indicator
    └── nodes/
        ├── HubNode.vue        # Indigo — planet/coordinator
        ├── ClassNode.vue      # Cyan — schema blueprint
        ├── ObjectNode.vue     # Emerald — live data container
        └── ActionNode.vue     # Amber — logic engine
```

---

## Features

### Live Graph Canvas

- Visualises all Knots fetched from `GET /api/graph`
- Hierarchical layout: Level 0 (root) at top, Satellites below
- Drag nodes to reposition them on the canvas
- Minimap + zoom controls in the bottom corners

### Custom Node Types

| Role | Visual | Accent |
|---|---|---|
| **Hub** | Large, globe icon, children count | Indigo `#6366f1` |
| **Class** | Blueprint / schema icon | Cyan `#06b6d4` |
| **Object** | Data container, live property preview | Emerald `#10b981` |
| **Action** | Engine / CPU icon, spins when active | Amber `#f59e0b` |

### Real-Time Event Stream

The editor subscribes to `ws://127.0.0.1:8080/api/ws` and reacts to every `KnotEvent`:

| Event | Visual Effect |
|---|---|
| `PacketForwardedDown/Up` | Edge animates (pulse along the path) |
| `PacketReceived` | Target node flashes |
| `ObjectStateChanged` | Node data updates + flash; property table refreshes |
| `ActionExecuted` | Action node flashes; CPU icon spins |
| `DeadLetter` | Node flashes |

### Inspector Panel (click any node)

**All Knots**
- UUID, level, parent link (click to navigate)
- Child list with role-colored dots (click to navigate)

**Object Knots**
- Live property table with typed value colouring
- Manual "Refresh" button → `GET /api/knot/:id/state`

**Action Knots**
- JSON payload editor with syntax validation
- Priority slider (0–255)
- "Send Packet" button → `POST /api/packet`

**Class Knots**
- "Query Schema" button injects a schema-request packet

### Connection Status

A pill indicator in the top bar shows:

- 🟢 **Connected** (sonar ping animation)
- 🟡 **Connecting…**
- 🔴 **Disconnected** + "reconnect" button

Auto-reconnect fires every 3 s on unexpected disconnection.

---

## Backend API Contract

| Method | Path | Used for |
|---|---|---|
| `GET` | `/api/graph` | Initial topology fetch |
| `GET` | `/api/knot/:id/state` | Object property state |
| `POST` | `/api/packet` | Inject packets from UI |
| `WS` | `/api/ws` | Live event stream + JSON-RPC |

### POST /api/packet body

```json
{
  "target_id": "<uuid>",
  "sender_id": "<uuid>",
  "priority":  200,
  "ttl_ms":    5000,
  "payload":   { "op": "set", "property": "temperature", "value": 42.5 }
}
```

### WebSocket JSON-RPC methods (client → server)

```json
{ "jsonrpc": "2.0", "method": "get_graph",   "id": 1 }
{ "jsonrpc": "2.0", "method": "send_packet",
  "params": { "target_id": "...", "payload": {} }, "id": 2 }
```

---

## Tech Stack

| Library | Version | Purpose |
|---|---|---|
| Vue 3 | ^3.4 | Framework (Composition API + `<script setup>`) |
| Vite | ^5.1 | Build tool + HMR dev server |
| Pinia | ^2.1 | State management |
| Vue Flow | ^1.33 | Interactive node-graph canvas |
| Tailwind CSS | ^3.4 | Utility-first styling |
| Lucide Vue Next | ^0.344 | Icon set |
| VueUse | ^10.9 | Composition utilities |

---

## Object Packet Protocol (quick reference)

Send these payloads to any **Object** Knot via the Inspector trigger or `POST /api/packet`:

```jsonc
// Read one property
{ "op": "get",    "property": "temperature" }

// Write one property
{ "op": "set",    "property": "temperature", "value": 42.5 }

// Write multiple at once
{ "op": "set_many", "properties": { "temperature": -5.0, "active": true } }

// Read all properties
{ "op": "get_all" }
```

---

## Extending the Editor

### Add a new node type

1. Create `src/components/nodes/MyNode.vue`
2. Register it in `Canvas.vue`:
   ```js
   import MyNode from './nodes/MyNode.vue'
   const nodeTypes = { ..., mytype: markRaw(MyNode) }
   ```
3. Return `role.toLowerCase()` matching `"mytype"` from the backend.

### Add a new backend event handler

In `src/stores/hubflow.js`, add a case to `_handleEvent`:

```js
case 'MyNewEvent':
  _flash(payload.knot_id)
  // ... custom logic
  break
```

---

## License

Part of the HubFlow project. See the root `README` for license information.