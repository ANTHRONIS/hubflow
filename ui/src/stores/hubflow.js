/**
 * HubFlow Pinia store — single source of truth for topology, live state,
 * WebSocket connection, and packet injection.
 *
 * Backend contract:
 *   GET    /api/graph          → RegistryEntry[]
 *   GET    /api/knot/:id/state → { [property: string]: JsonValue }
 *   POST   /api/packet         → { target_id, priority?, ttl_ms?, payload }
 *   POST   /api/knot           → { name, description?, role, parent_id?, x?, y? }
 *   DELETE /api/knot/:id       → 204 No Content
 *   PATCH  /api/knot/:id       → { name?, description?, x?, y? }
 *   WS     /api/ws             → EventEnvelope stream + JSON-RPC 2.0
 *
 * WebSocket messages from server:
 *   EventEnvelope: { event_type: string, payload: object }
 *   JSON-RPC 2.0 response: { jsonrpc, result, id }
 *
 * JSON-RPC methods the client can send:
 *   get_graph    → { jsonrpc: "2.0", method: "get_graph", id: 1 }
 *   send_packet  → { jsonrpc: "2.0", method: "send_packet", params: {...}, id: n }
 */
import { defineStore } from "pinia";
import { ref, computed } from "vue";

const API_BASE = "http://127.0.0.1:8080";
const WS_URL = "ws://127.0.0.1:8080/api/ws";

// ── Role → accent colour (must match Tailwind custom colours) ──────────────
export const ROLE_COLOR = {
  Hub: "#6366f1", // indigo
  Action: "#f59e0b", // amber
  Object: "#10b981", // emerald
  Class: "#06b6d4", // cyan
};

// ── Layout constants ──────────────────────────────────────────────────────
const NODE_W = 210;
const NODE_H = 90;
const H_GAP = 90;
const V_GAP = 170;

/**
 * Converts a flat RegistryEntry array into Vue Flow node positions.
 * Uses stored x/y when non-zero; falls back to automatic per-level
 * horizontal distribution otherwise.
 */
function buildLayout(knotList, flashingIds, stateMap) {
  if (!knotList.length) return [];

  // Auto-layout for nodes without stored positions
  const byLevel = {};
  for (const k of knotList) {
    (byLevel[k.level] ??= []).push(k);
  }
  const autoPos = {};
  for (const [lvlStr, levelKnots] of Object.entries(byLevel)) {
    const level = Number(lvlStr);
    const total = levelKnots.length * NODE_W + (levelKnots.length - 1) * H_GAP;
    const startX = -total / 2;
    levelKnots.forEach((knot, i) => {
      autoPos[knot.id] = { x: startX + i * (NODE_W + H_GAP), y: level * V_GAP };
    });
  }

  return knotList.map((knot) => {
    // Use stored position if meaningful (non-zero), otherwise auto-layout
    const hasStoredPos = knot.x !== 0 || knot.y !== 0;
    const pos = hasStoredPos
      ? { x: knot.x, y: knot.y }
      : (autoPos[knot.id] ?? { x: 0, y: 0 });
    return {
      id: knot.id,
      type: knot.role.toLowerCase(), // 'hub' | 'action' | 'object' | 'class'
      position: pos,
      draggable: true,
      data: {
        ...knot,
        _isFlashing: flashingIds.has(knot.id),
        _state: stateMap[knot.id] ?? null,
      },
    };
  });
}

export const useHubFlowStore = defineStore("hubflow", () => {
  // ── Core state ────────────────────────────────────────────────────────────
  /** All registered Knots fetched from the backend. */
  const knots = ref([]);
  /** Live Object-Knot property maps: { [knotId]: { [prop]: value } } */
  const knotStates = ref({});
  /** UUID of the currently selected Knot, or null. */
  const selectedKnotId = ref(null);
  /** WebSocket connection state. */
  const connectionStatus = ref("disconnected"); // 'connecting'|'connected'|'disconnected'
  /** Ring-buffer of the last 100 KnotEvents for the event log. */
  const eventLog = ref([]);

  // Internal: Sets are replaced wholesale to trigger Vue reactivity.
  const animatingEdgeIds = ref(new Set());
  const flashingKnotIds = ref(new Set());

  // Internal WS reference — NOT reactive (just a JS reference).
  let _ws = null;
  let _reconnect = true;

  // ── Derived ───────────────────────────────────────────────────────────────
  /** Fast O(1) lookup: UUID → RegistryEntry */
  const knotMap = computed(() => new Map(knots.value.map((k) => [k.id, k])));

  /** The currently-selected Knot object, or null. */
  const selectedKnot = computed(() =>
    selectedKnotId.value
      ? (knotMap.value.get(selectedKnotId.value) ?? null)
      : null,
  );

  /** Vue Flow node array derived from knots + flash state + live state. */
  const vfNodes = computed(() =>
    buildLayout(knots.value, flashingKnotIds.value, knotStates.value),
  );

  /** Vue Flow edge array derived from parent_id relationships. */
  const vfEdges = computed(() => {
    const animIds = animatingEdgeIds.value;
    return knots.value
      .filter((k) => k.parent_id)
      .map((k) => {
        const id = `e-${k.parent_id}-${k.id}`;
        const isAnim = animIds.has(id);
        return {
          id,
          source: k.parent_id,
          target: k.id,
          animated: isAnim,
          style: {
            stroke: ROLE_COLOR[k.role] ?? "#6366f1",
            strokeWidth: isAnim ? 2.5 : 1.5,
            opacity: isAnim ? 1.0 : 0.55,
          },
        };
      });
  });

  // ── REST API ──────────────────────────────────────────────────────────────

  /** Fetch the full topology from GET /api/graph and replace knots. */
  async function fetchGraph() {
    try {
      const r = await fetch(`${API_BASE}/api/graph`);
      if (!r.ok) throw new Error(r.statusText);
      knots.value = await r.json();
    } catch (e) {
      console.warn("[HubFlow] fetchGraph:", e.message);
    }
  }

  /**
   * Fetch the live property state for an Object Knot.
   * Stores it in knotStates and returns it.
   */
  async function fetchKnotState(id) {
    try {
      const r = await fetch(`${API_BASE}/api/knot/${id}/state`);
      if (!r.ok) return null;
      const data = await r.json();
      knotStates.value = { ...knotStates.value, [id]: data };
      return data;
    } catch (e) {
      console.warn("[HubFlow] fetchKnotState:", e.message);
      return null;
    }
  }

  /**
   * Inject a packet into the system via POST /api/packet.
   * Returns true on success.
   */
  async function injectPacket({
    targetId,
    payload,
    priority = 128,
    ttlMs = 5000,
  }) {
    try {
      const r = await fetch(`${API_BASE}/api/packet`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          target_id: targetId,
          priority,
          ttl_ms: ttlMs,
          payload,
        }),
      });
      return r.ok;
    } catch (e) {
      console.warn("[HubFlow] injectPacket:", e.message);
      return false;
    }
  }

  /**
   * Create a new Knot via POST /api/knot.
   * Immediately adds the entry to the local `knots` array (optimistic update);
   * corrects it when the confirmed entry arrives via WS KnotCreated event.
   */
  async function createKnot({
    name,
    role,
    parentId = null,
    x = 0,
    y = 0,
    description = null,
  }) {
    const body = { name, role, parent_id: parentId, x, y };
    if (description) body.description = description;
    try {
      const r = await fetch(`${API_BASE}/api/knot`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!r.ok) throw new Error(await r.text());
      const entry = await r.json();
      // Merge into local knots (WS event may arrive first)
      if (!knots.value.find((k) => k.id === entry.id)) {
        knots.value = [...knots.value, entry];
      }
      return entry;
    } catch (e) {
      console.warn("[HubFlow] createKnot:", e.message);
      return null;
    }
  }

  /**
   * Delete a Knot via DELETE /api/knot/:id.
   * Removes it from the local array immediately (optimistic).
   */
  async function deleteKnot(id) {
    // Optimistic removal
    knots.value = knots.value.filter((k) => k.id !== id);
    if (selectedKnotId.value === id) selectedKnotId.value = null;
    try {
      const r = await fetch(`${API_BASE}/api/knot/${id}`, { method: "DELETE" });
      return r.ok || r.status === 204;
    } catch (e) {
      console.warn("[HubFlow] deleteKnot:", e.message);
      return false;
    }
  }

  /**
   * Create a parent-child link via POST /api/link.
   * The source becomes the parent; the target becomes its child.
   * Triggers a GraphRefresh from the server so the edge appears.
   */
  async function createLink(sourceId, targetId, linkType = 'edge') {
    try {
      const r = await fetch(`${API_BASE}/api/link`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ source_id: sourceId, target_id: targetId, link_type: linkType }),
      })
      if (!r.ok) throw new Error(await r.text())
      // Optimistic: update parent_id on the child node locally
      knots.value = knots.value.map((k) => {
        if (k.id !== targetId) return k
        return { ...k, parent_id: sourceId }
      })
      // Also add targetId to the parent's child_ids
      knots.value = knots.value.map((k) => {
        if (k.id !== sourceId) return k
        const child_ids = k.child_ids?.includes(targetId) ? k.child_ids : [...(k.child_ids ?? []), targetId]
        return { ...k, child_ids }
      })
      return await r.json()
    } catch (e) {
      console.warn('[HubFlow] createLink:', e.message)
      return null
    }
  }

  /**
   * Update a Knot's metadata or canvas position via PATCH /api/knot/:id.
   * Updates the local entry immediately (optimistic).
   */
  async function updateKnot(id, patch) {
    // Optimistic update
    knots.value = knots.value.map((k) => {
      if (k.id !== id) return k;
      const updated = { ...k };
      if (patch.name != null) updated.meta = { ...k.meta, name: patch.name };
      if ('description' in patch)
        updated.meta = { ...updated.meta, description: patch.description };
      if (patch.x != null) updated.x = patch.x;
      if (patch.y != null) updated.y = patch.y;
      return updated;
    });
    try {
      const r = await fetch(`${API_BASE}/api/knot/${id}`, {
        method: "PATCH",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name: patch.name,
          description: patch.description,
          x: patch.x,
          y: patch.y,
        }),
      });
      return r.ok;
    } catch (e) {
      console.warn("[HubFlow] updateKnot:", e.message);
      return false;
    }
  }

  // ── WebSocket ─────────────────────────────────────────────────────────────

  /** Open (or reuse) the WebSocket connection. */
  function connect() {
    if (_ws?.readyState === WebSocket.OPEN) return;
    connectionStatus.value = "connecting";
    _reconnect = true;

    _ws = new WebSocket(WS_URL);

    _ws.onopen = () => {
      connectionStatus.value = "connected";
      // Ask for a full graph snapshot via JSON-RPC
      _ws.send(JSON.stringify({ jsonrpc: "2.0", method: "get_graph", id: 1 }));
    };

    _ws.onmessage = (e) => {
      try {
        _dispatch(JSON.parse(e.data));
      } catch {
        /* ignore malformed */
      }
    };

    _ws.onerror = () => {
      connectionStatus.value = "disconnected";
    };

    _ws.onclose = () => {
      connectionStatus.value = "disconnected";
      if (_reconnect) setTimeout(connect, 3000);
    };
  }

  /** Close the WebSocket and suppress auto-reconnect. */
  function disconnect() {
    _reconnect = false;
    _ws?.close();
    _ws = null;
    connectionStatus.value = "disconnected";
  }

  // ── Internal WS dispatch ──────────────────────────────────────────────────

  function _dispatch(msg) {
    // JSON-RPC 2.0 response (e.g. from get_graph)
    if (msg.jsonrpc === "2.0" && msg.result != null) {
      if (Array.isArray(msg.result)) knots.value = msg.result;
      return;
    }
    // EventEnvelope { event_type: string, payload: object }
    if (msg.event_type) {
      _logEvent(msg.event_type, msg.payload);
      _handleEvent(msg.event_type, msg.payload);
    }
  }

  function _logEvent(type, payload) {
    eventLog.value.unshift({ type, payload, ts: Date.now() });
    if (eventLog.value.length > 100) eventLog.value.length = 100;
  }

  function _handleEvent(type, payload) {
    switch (type) {
      case "PacketForwardedDown":
      case "PacketForwardedUp": {
        // Highlight the edge the packet just crossed
        const packet = payload.packet ?? payload;
        const targetKnot = knotMap.value.get(packet.target_id);
        const parentId = targetKnot?.parent_id;
        if (parentId) {
          const eid = `e-${parentId}-${packet.target_id}`;
          const next = new Set(animatingEdgeIds.value);
          next.add(eid);
          animatingEdgeIds.value = next;
          setTimeout(() => {
            const s = new Set(animatingEdgeIds.value);
            s.delete(eid);
            animatingEdgeIds.value = s;
          }, 1400);
        }
        break;
      }

      case "PacketReceived":
        _flash(payload.packet?.target_id);
        break;

      case "ObjectStateChanged": {
        const { knot_id, property, value } = payload;
        knotStates.value = {
          ...knotStates.value,
          [knot_id]: {
            ...(knotStates.value[knot_id] ?? {}),
            [property]: value,
          },
        };
        _flash(knot_id);
        break;
      }

      case "ActionExecuted":
        _flash(payload.knot_id);
        break;

      case "DeadLetter":
        _flash(payload.packet?.target_id);
        break;

      case "KnotCreated": {
        const { knot_id, name, role, level, parent_id, x, y } = payload;
        // Only add if not already present (optimistic update may have added it)
        if (!knots.value.find((k) => k.id === knot_id)) {
          knots.value = [
            ...knots.value,
            {
              id: knot_id,
              local_id: 0,
              meta: { name, description: null },
              role,
              level,
              parent_id,
              class_id: null,
              child_ids: [],
              x,
              y,
            },
          ];
        }
        break;
      }

      case "KnotDeleted": {
        const { knot_id } = payload;
        knots.value = knots.value.filter((k) => k.id !== knot_id);
        if (selectedKnotId.value === knot_id) selectedKnotId.value = null;
        break;
      }

      case "KnotUpdated": {
        const { knot_id, name, description, x, y } = payload;
        knots.value = knots.value.map((k) => {
          if (k.id !== knot_id) return k;
          const updated = { ...k };
          if (name != null) updated.meta = { ...k.meta, name };
          if (description !== undefined) updated.meta = { ...updated.meta, description };
          if (x != null) updated.x = x;
          if (y != null) updated.y = y;
          return updated;
        });
        break;
      }

      case "GraphRefresh":
        fetchGraph();
        break;
    }
  }

  /** Flash a node for ~900 ms by toggling its ID in the flashing set. */
  function _flash(id) {
    if (!id) return;
    const next = new Set(flashingKnotIds.value);
    next.add(id);
    flashingKnotIds.value = next;
    setTimeout(() => {
      const s = new Set(flashingKnotIds.value);
      s.delete(id);
      flashingKnotIds.value = s;
    }, 900);
  }

  // ── Selection ─────────────────────────────────────────────────────────────

  /**
   * Select a Knot by UUID.
   * Automatically fetches state for Object Knots.
   */
  function selectKnot(id) {
    selectedKnotId.value = id;
    if (id && knotMap.value.get(id)?.role === "Object") {
      fetchKnotState(id);
    }
  }

  function clearSelection() {
    selectedKnotId.value = null;
  }

  // ── Public API ─────────────────────────────────────────────────────────────
  return {
    // state
    knots,
    knotStates,
    selectedKnotId,
    connectionStatus,
    eventLog,
    // computed
    knotMap,
    selectedKnot,
    vfNodes,
    vfEdges,
    // actions
    fetchGraph,
    fetchKnotState,
    injectPacket,
    createKnot,
    createLink,
    deleteKnot,
    updateKnot,
    connect,
    disconnect,
    selectKnot,
    clearSelection,
  };
});
