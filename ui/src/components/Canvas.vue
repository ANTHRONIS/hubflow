<template>
  <div
    class="w-full h-full relative"
    @dragover.prevent="onDragOver"
    @drop.prevent="onDrop"
  >
    <VueFlow
      :nodes="store.vfNodes"
      :edges="store.vfEdges"
      :node-types="nodeTypes"
      connection-mode="loose"
      :nodes-connectable="true"
      :nodes-draggable="true"
      :edges-updatable="true"
      fit-view-on-init
      :min-zoom="0.15"
      :max-zoom="2.5"
      :snap-to-grid="snapGrid"
      :snap-grid="[20, 20]"
      class="hubflow-canvas"
      @node-click="onNodeClick"
      @node-drag-stop="onNodeDragStop"
      @pane-click="onPaneClick"
      @pane-context-menu="onPaneContextMenu"
      @node-context-menu="onNodeContextMenu"
      @connect="onConnect"
      @edge-update="onEdgeUpdate"
    >
      <Background variant="dots" :gap="28" :size="1.2" pattern-color="#1e293b" bg-color="#070c1a" />
      <Controls position="bottom-left" />
      <MiniMap
        position="bottom-right"
        :node-color="minimapColor"
        :node-stroke-color="minimapColor"
        node-stroke-width="2"
        :mask-color="'rgba(7,12,26,0.7)'"
      />
    </VueFlow>

    <!-- Snap-to-grid indicator -->
    <div
      v-if="snapGrid"
      class="absolute top-3 left-1/2 -translate-x-1/2 px-2.5 py-1 rounded-full
             bg-indigo-900/60 border border-indigo-600/50 text-[10px] font-mono text-indigo-300
             pointer-events-none"
    >
      Snap to grid: ON
    </div>

    <!-- Tidy-up button (floating, top-right) -->
    <button
      @click="tidyUp"
      title="Tidy Up layout"
      class="absolute top-3 right-3 flex items-center gap-1.5 px-3 py-1.5 rounded-lg
             bg-gray-800/80 border border-gray-700/60 text-xs text-gray-300 hover:text-gray-100
             hover:bg-gray-700 backdrop-blur transition-colors"
    >
      <LayoutDashboard class="w-3.5 h-3.5" />
      Tidy Up
    </button>

    <!-- Context menu -->
    <ContextMenu ref="ctxMenu" @action="onContextAction" />
  </div>
</template>

<script setup>
import { ref, markRaw } from 'vue'
import { VueFlow, useVueFlow } from '@vue-flow/core'
import { Background } from '@vue-flow/background'
import { Controls   } from '@vue-flow/controls'
import { MiniMap    } from '@vue-flow/minimap'
import '@vue-flow/controls/dist/style.css'
import '@vue-flow/minimap/dist/style.css'
import { LayoutDashboard } from 'lucide-vue-next'

import { useHubFlowStore, ROLE_COLOR } from '../stores/hubflow'
import HubNode    from './nodes/HubNode.vue'
import ClassNode  from './nodes/ClassNode.vue'
import ObjectNode from './nodes/ObjectNode.vue'
import ActionNode from './nodes/ActionNode.vue'
import ContextMenu from './ContextMenu.vue'

const store = useHubFlowStore()
const { screenToFlowCoordinate } = useVueFlow()

const nodeTypes = {
  hub:    markRaw(HubNode),
  class:  markRaw(ClassNode),
  object: markRaw(ObjectNode),
  action: markRaw(ActionNode),
}

// ── Snap to grid ──────────────────────────────────────────────────────────
const snapGrid = ref(false)

// ── Node selection ────────────────────────────────────────────────────────
function onNodeClick({ node }) {
  store.selectKnot(node.id)
}

function onPaneClick() {
  store.clearSelection()
}

// ── Position save on drag-stop ────────────────────────────────────────────
// Debounce map to avoid flooding PATCH requests
const _posTimers = {}
function onNodeDragStop({ node }) {
  clearTimeout(_posTimers[node.id])
  _posTimers[node.id] = setTimeout(() => {
    store.updateKnot(node.id, { x: node.position.x, y: node.position.y })
  }, 400)
}

// ── Drag-and-drop from NodeToolbar ────────────────────────────────────────
function onDragOver(event) {
  event.dataTransfer.dropEffect = 'copy'
}

function onDrop(event) {
  const role = event.dataTransfer.getData('hubflow/role')
  if (!role) return
  const pos = screenToFlowCoordinate({ x: event.clientX, y: event.clientY })
  const parentId = store.selectedKnotId

  store.createKnot({
    role,
    name:     `New ${role}`,
    parentId,
    x:        Math.round(pos.x),
    y:        Math.round(pos.y),
  })
}

// ── Connections (draw edge = create parent-child link) ────────────────────
async function onConnect({ source, target }) {
  if (source && target && source !== target) {
    await store.createLink(source, target)
  }
}

// ── Edge re-routing (drag an existing edge to a new target) ───────────────
async function onEdgeUpdate({ edge, connection }) {
  if (connection.source && connection.target && connection.source !== connection.target) {
    await store.createLink(connection.source, connection.target)
  }
}

// ── Context menu ──────────────────────────────────────────────────────────
const ctxMenu = ref(null)

function onPaneContextMenu(event) {
  event.preventDefault()
  const pos = screenToFlowCoordinate({ x: event.clientX, y: event.clientY })
  ctxMenu.value?.show(event, null, pos.x, pos.y)
}

function onNodeContextMenu({ event, node }) {
  event.preventDefault()
  store.selectKnot(node.id)
  ctxMenu.value?.show(event, node.id)
}

async function onContextAction({ type, nodeId, x, y }) {
  switch (type) {
    case 'create-hub':    openCreateDialog('Hub',    x, y); break
    case 'create-object': openCreateDialog('Object', x, y); break
    case 'create-action': openCreateDialog('Action', x, y); break
    case 'create-class':  openCreateDialog('Class',  x, y); break
    case 'tidy':          tidyUp();                         break
    case 'refresh':       store.fetchGraph();               break
    case 'delete':
      if (nodeId) await store.deleteKnot(nodeId)
      break
    case 'rename':
      if (nodeId) store.selectKnot(nodeId) // opens inspector for editing
      break
    case 'clone':
      if (nodeId) {
        const src = store.knotMap.get(nodeId)
        if (src) {
          store.createKnot({
            role:     src.role,
            name:     `${src.meta.name} (copy)`,
            parentId: src.parent_id,
            x:        (src.x ?? 0) + 220,
            y:        src.y ?? 0,
          })
        }
      }
      break
  }
}

// Toolbar calls this to open the NodeToolbar dialog at a canvas position
function openCreateDialog(role, x, y) {
  emit('open-create', { role, x, y })
}

const emit = defineEmits(['open-create'])

// ── Tidy Up ───────────────────────────────────────────────────────────────
async function tidyUp() {
  const knotList = store.knots
  if (!knotList.length) return

  const NODE_W = 210, H_GAP = 90, V_GAP = 170
  const byLevel = {}
  for (const k of knotList) ;(byLevel[k.level] ??= []).push(k)

  const patches = []
  for (const [lvlStr, levelKnots] of Object.entries(byLevel)) {
    const level  = Number(lvlStr)
    const total  = levelKnots.length * NODE_W + (levelKnots.length - 1) * H_GAP
    const startX = -total / 2
    levelKnots.forEach((knot, i) => {
      const x = Math.round(startX + i * (NODE_W + H_GAP))
      const y = level * V_GAP
      patches.push(store.updateKnot(knot.id, { x, y }))
    })
  }
  await Promise.all(patches)
}

// ── Minimap colour ────────────────────────────────────────────────────────
function minimapColor(node) {
  return ROLE_COLOR[node.data?.role] ?? '#6366f1'
}

// ── Expose for App.vue ────────────────────────────────────────────────────
defineExpose({ toggleSnapGrid: () => { snapGrid.value = !snapGrid.value }, tidyUp })
</script>

<style>
.hubflow-canvas { background: #070c1a; width: 100%; height: 100%; }

/* Always show handles — not just on hover */
.vue-flow__handle {
  width:   12px !important;
  height:  12px !important;
  border-radius: 50% !important;
  border: 2px solid currentColor !important;
  opacity: 0.65;
  transition: opacity 0.15s, transform 0.15s;
}
.vue-flow__handle:hover,
.vue-flow__handle.connecting,
.vue-flow__handle.valid {
  opacity: 1;
  transform: scale(1.35);
}

/* Connection line while dragging */
.vue-flow__connection-path {
  stroke: #6366f1 !important;
  stroke-width: 2px !important;
  stroke-dasharray: 6 3;
  animation: connDash 0.5s linear infinite;
}
@keyframes connDash { to { stroke-dashoffset: -18; } }

/* Selected node highlight */
.vue-flow__node.selected > * {
  outline: 2px solid rgba(99,102,241,0.8);
  outline-offset: 3px;
  border-radius: 12px;
}
</style>
