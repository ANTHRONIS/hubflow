/**
 * HubFlow Pinia store — single source of truth for topology, live state,
 * WebSocket connection, and packet injection.
 *
 * Backend contract:
 *   GET  /api/graph          → RegistryEntry[]
 *   GET  /api/knot/:id/state → { [property: string]: JsonValue }
 *   POST /api/packet         → { target_id, priority?, ttl_ms?, payload }
 *   WS   /api/ws             → EventEnvelope stream + JSON-RPC 2.0
 *
 * WebSocket messages from server:
 *   EventEnvelope: { event_type: string, payload: object }
 *   JSON-RPC 2.0 response: { jsonrpc, result, id }
 *
 * JSON-RPC methods the client can send:
 *   get_graph    → { jsonrpc: "2.0", method: "get_graph", id: 1 }
 *   send_packet  → { jsonrpc: "2.0", method: "send_packet", params: {...}, id: n }
 */
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

const API_BASE = 'http://127.0.0.1:8080'
const WS_URL   = 'ws://127.0.0.1:8080/api/ws'

// ── Role → accent colour (must match Tailwind custom colours) ──────────────
export const ROLE_COLOR = {
  Hub:    '#6366f1',   // indigo
  Action: '#f59e0b',   // amber
  Object: '#10b981',   // emerald
  Class:  '#06b6d4',   // cyan
}

// ── Layout constants ──────────────────────────────────────────────────────
const NODE_W = 210
const NODE_H = 90
const H_GAP  = 90
const V_GAP  = 170

/**
 * Converts a flat RegistryEntry array into Vue Flow node positions using a
 * simple per-level horizontal distribution.
 */
function buildLayout(knotList, flashingIds, stateMap) {
  if (!knotList.length) return []

  // Group knots by their topology level
  const byLevel = {}
  for (const k of knotList) {
    ;(byLevel[k.level] ??= []).push(k)
  }

  const nodes = []

  for (const [lvlStr, levelKnots] of Object.entries(byLevel)) {
    const level     = Number(lvlStr)
    const total     = levelKnots.length * NODE_W + (levelKnots.length - 1) * H_GAP
    const startX    = -total / 2

    levelKnots.forEach((knot, i) => {
      nodes.push({
        id:        knot.id,
        type:      knot.role.toLowerCase(),   // 'hub' | 'action' | 'object' | 'class'
        position:  { x: startX + i * (NODE_W + H_GAP), y: level * V_GAP },
        draggable: true,
        data: {
          ...knot,
          _isFlashing: flashingIds.has(knot.id),
          _state:      stateMap[knot.id] ?? null,
        },
      })
    })
  }

  return nodes
}

export const useHubFlowStore = defineStore('hubflow', () => {

  // ── Core state ────────────────────────────────────────────────────────────
  /** All registered Knots fetched from the backend. */
  const knots            = ref([])
  /** Live Object-Knot property maps: { [knotId]: { [prop]: value } } */
  const knotStates       = ref({})
  /** UUID of the currently selected Knot, or null. */
  const selectedKnotId   = ref(null)
  /** WebSocket connection state. */
  const connectionStatus = ref('disconnected')  // 'connecting'|'connected'|'disconnected'
  /** Ring-buffer of the last 100 KnotEvents for the event log. */
  const eventLog         = ref([])

  // Internal: Sets are replaced wholesale to trigger Vue reactivity.
  const animatingEdgeIds = ref(new Set())
  const flashingKnotIds  = ref(new Set())

  // Internal WS reference — NOT reactive (just a JS reference).
  let _ws        = null
  let _reconnect = true

  // ── Derived ───────────────────────────────────────────────────────────────
  /** Fast O(1) lookup: UUID → RegistryEntry */
  const knotMap = computed(() =>
    new Map(knots.value.map(k => [k.id, k]))
  )

  /** The currently-selected Knot object, or null. */
  const selectedKnot = computed(() =>
    selectedKnotId.value ? (knotMap.value.get(selectedKnotId.value) ?? null) : null
  )

  /** Vue Flow node array derived from knots + flash state + live state. */
  const vfNodes = computed(() =>
    buildLayout(knots.value, flashingKnotIds.value, knotStates.value)
  )

  /** Vue Flow edge array derived from parent_id relationships. */
  const vfEdges = computed(() => {
    const animIds = animatingEdgeIds.value
    return knots.value
      .filter(k => k.parent_id)
      .map(k => {
        const id = `e-${k.parent_id}-${k.id}`
        const isAnim = animIds.has(id)
        return {
          id,
          source:    k.parent_id,
          target:    k.id,
          animated:  isAnim,
          style: {
            stroke:      ROLE_COLOR[k.role] ?? '#6366f1',
            strokeWidth: isAnim ? 2.5 : 1.5,
            opacity:     isAnim ? 1.0 : 0.55,
          },
        }
      })
  })

  // ── REST API ──────────────────────────────────────────────────────────────

  /** Fetch the full topology from GET /api/graph and replace knots. */
  async function fetchGraph() {
    try {
      const r = await fetch(`${API_BASE}/api/graph`)
      if (!r.ok) throw new Error(r.statusText)
      knots.value = await r.json()
    } catch (e) {
      console.warn('[HubFlow] fetchGraph:', e.message)
    }
  }

  /**
   * Fetch the live property state for an Object Knot.
   * Stores it in knotStates and returns it.
   */
  async function fetchKnotState(id) {
    try {
      const r = await fetch(`${API_BASE}/api/knot/${id}/state`)
      if (!r.ok) return null
      const data = await r.json()
      knotStates.value = { ...knotStates.value, [id]: data }
      return data
    } catch (e) {
      console.warn('[HubFlow] fetchKnotState:', e.message)
      return null
    }
  }

  /**
   * Inject a packet into the system via POST /api/packet.
   * Returns true on success.
   */
  async function injectPacket({ targetId, payload, priority = 128, ttlMs = 5000 }) {
    try {
      const r = await fetch(`${API_BASE}/api/packet`, {
        method:  'POST',
        headers: { 'Content-Type': 'application/json' },
        body:    JSON.stringify({ target_id: targetId, priority, ttl_ms: ttlMs, payload }),
      })
      return r.ok
    } catch (e) {
      console.warn('[HubFlow] injectPacket:', e.message)
      return false
    }
  }

  // ── WebSocket ─────────────────────────────────────────────────────────────

  /** Open (or reuse) the WebSocket connection. */
  function connect() {
    if (_ws?.readyState === WebSocket.OPEN) return
    connectionStatus.value = 'connecting'
    _reconnect = true

    _ws = new WebSocket(WS_URL)

    _ws.onopen = () => {
      connectionStatus.value = 'connected'
      // Ask for a full graph snapshot via JSON-RPC
      _ws.send(JSON.stringify({ jsonrpc: '2.0', method: 'get_graph', id: 1 }))
    }

    _ws.onmessage = (e) => {
      try { _dispatch(JSON.parse(e.data)) } catch { /* ignore malformed */ }
    }

    _ws.onerror = () => { connectionStatus.value = 'disconnected' }

    _ws.onclose = () => {
      connectionStatus.value = 'disconnected'
      if (_reconnect) setTimeout(connect, 3000)
    }
  }

  /** Close the WebSocket and suppress auto-reconnect. */
  function disconnect() {
    _reconnect = false
    _ws?.close()
    _ws = null
    connectionStatus.value = 'disconnected'
  }

  // ── Internal WS dispatch ──────────────────────────────────────────────────

  function _dispatch(msg) {
    // JSON-RPC 2.0 response (e.g. from get_graph)
    if (msg.jsonrpc === '2.0' && msg.result != null) {
      if (Array.isArray(msg.result)) knots.value = msg.result
      return
    }
    // EventEnvelope { event_type: string, payload: object }
    if (msg.event_type) {
      _logEvent(msg.event_type, msg.payload)
      _handleEvent(msg.event_type, msg.payload)
    }
  }

  function _logEvent(type, payload) {
    eventLog.value.unshift({ type, payload, ts: Date.now() })
    if (eventLog.value.length > 100) eventLog.value.length = 100
  }

  function _handleEvent(type, payload) {
    switch (type) {

      case 'PacketForwardedDown':
      case 'PacketForwardedUp': {
        // Highlight the edge the packet just crossed
        const packet     = payload.packet ?? payload
        const targetKnot = knotMap.value.get(packet.target_id)
        const parentId   = targetKnot?.parent_id
        if (parentId) {
          const eid  = `e-${parentId}-${packet.target_id}`
          const next = new Set(animatingEdgeIds.value)
          next.add(eid)
          animatingEdgeIds.value = next
          setTimeout(() => {
            const s = new Set(animatingEdgeIds.value)
            s.delete(eid)
            animatingEdgeIds.value = s
          }, 1400)
        }
        break
      }

      case 'PacketReceived':
        _flash(payload.packet?.target_id)
        break

      case 'ObjectStateChanged': {
        const { knot_id, property, value } = payload
        knotStates.value = {
          ...knotStates.value,
          [knot_id]: { ...(knotStates.value[knot_id] ?? {}), [property]: value },
        }
        _flash(knot_id)
        break
      }

      case 'ActionExecuted':
        _flash(payload.knot_id)
        break

      case 'DeadLetter':
        _flash(payload.packet?.target_id)
        break
    }
  }

  /** Flash a node for ~900 ms by toggling its ID in the flashing set. */
  function _flash(id) {
    if (!id) return
    const next = new Set(flashingKnotIds.value)
    next.add(id)
    flashingKnotIds.value = next
    setTimeout(() => {
      const s = new Set(flashingKnotIds.value)
      s.delete(id)
      flashingKnotIds.value = s
    }, 900)
  }

  // ── Selection ─────────────────────────────────────────────────────────────

  /**
   * Select a Knot by UUID.
   * Automatically fetches state for Object Knots.
   */
  function selectKnot(id) {
    selectedKnotId.value = id
    if (id && knotMap.value.get(id)?.role === 'Object') {
      fetchKnotState(id)
    }
  }

  function clearSelection() { selectedKnotId.value = null }

  // ── Public API ─────────────────────────────────────────────────────────────
  return {
    // state
    knots, knotStates, selectedKnotId, connectionStatus, eventLog,
    // computed
    knotMap, selectedKnot, vfNodes, vfEdges,
    // actions
    fetchGraph, fetchKnotState, injectPacket,
    connect, disconnect,
    selectKnot, clearSelection,
  }
})
