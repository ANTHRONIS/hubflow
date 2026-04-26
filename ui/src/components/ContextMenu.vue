<template>
  <Teleport to="body">
    <Transition name="ctx-fade">
      <div
        v-if="visible"
        :style="{ position: 'fixed', left: `${x}px`, top: `${y}px`, zIndex: 2000 }"
        class="bg-gray-800 border border-gray-700/80 rounded-xl shadow-2xl py-1.5 min-w-44
               backdrop-blur"
        @click.stop
        @contextmenu.prevent
      >
        <!-- Canvas context menu ───────────────────────────────────────── -->
        <template v-if="!targetNodeId">
          <CtxItem icon="Globe"           color="#6366f1" @click="action('create-hub')">New Hub</CtxItem>
          <CtxItem icon="Box"             color="#10b981" @click="action('create-object')">New Object</CtxItem>
          <CtxItem icon="Cpu"             color="#f59e0b" @click="action('create-action')">New Action</CtxItem>
          <CtxItem icon="FileJson"        color="#06b6d4" @click="action('create-class')">New Class</CtxItem>
          <div class="border-t border-gray-700/60 my-1" />
          <CtxItem icon="LayoutDashboard" color="#9ca3af" @click="action('tidy')">Tidy Up</CtxItem>
          <CtxItem icon="RefreshCw"       color="#9ca3af" @click="action('refresh')">Refresh Graph</CtxItem>
        </template>

        <!-- Node context menu ─────────────────────────────────────────── -->
        <template v-else>
          <CtxItem icon="Pencil"   color="#9ca3af" @click="action('rename')">Rename</CtxItem>
          <CtxItem icon="CopyPlus" color="#9ca3af" @click="action('clone')">Clone</CtxItem>
          <div class="border-t border-gray-700/60 my-1" />
          <CtxItem icon="Trash2"   color="#f87171" :class-extra="'text-red-400 hover:bg-red-950/40'" @click="action('delete')">
            Delete
          </CtxItem>
        </template>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import {
  Globe, Box, Cpu, FileJson,
  LayoutDashboard, RefreshCw,
  Pencil, CopyPlus, Trash2,
} from 'lucide-vue-next'

const emit = defineEmits(['action'])

// ── Icon lookup ────────────────────────────────────────────────────────────
const ICONS = { Globe, Box, Cpu, FileJson, LayoutDashboard, RefreshCw, Pencil, CopyPlus, Trash2 }

// ── Inline CtxItem sub-component ──────────────────────────────────────────
const CtxItem = {
  props: ['icon', 'color', 'classExtra'],
  emits: ['click'],
  setup(p, { emit: e }) {
    return { ICONS, e }
  },
  template: `
    <button
      @click="e('click')"
      class="flex items-center gap-2.5 w-full px-3 py-1.5 text-xs text-gray-300
             hover:bg-gray-700/60 hover:text-gray-100 transition-colors text-left"
      :class="classExtra"
    >
      <component :is="ICONS[icon]" class="w-3.5 h-3.5 flex-shrink-0" :style="{ color }" />
      <slot />
    </button>
  `,
}

// ── Visibility & position state ────────────────────────────────────────────
const visible      = ref(false)
const x            = ref(0)
const y            = ref(0)
const targetNodeId = ref(null)   // null = canvas, UUID = node

let canvasX = 0
let canvasY = 0

// ── Public API ─────────────────────────────────────────────────────────────

/**
 * Show the context menu.
 * @param {MouseEvent} event    - The triggering pointer event (for screen position).
 * @param {string|null} nodeId  - Node UUID, or null when clicking the canvas background.
 * @param {number} flowX        - Canvas-space X coordinate of the click.
 * @param {number} flowY        - Canvas-space Y coordinate of the click.
 */
function show(event, nodeId = null, flowX = 0, flowY = 0) {
  // Clamp so the menu doesn't overflow the viewport edges
  const menuW = 176  // min-w-44 = 11rem = 176px
  const menuH = nodeId ? 100 : 160
  x.value = Math.min(event.clientX, window.innerWidth  - menuW - 8)
  y.value = Math.min(event.clientY, window.innerHeight - menuH - 8)

  targetNodeId.value = nodeId
  canvasX            = flowX
  canvasY            = flowY
  visible.value      = true
}

function hide() {
  visible.value = false
}

function action(type) {
  emit('action', { type, nodeId: targetNodeId.value, x: canvasX, y: canvasY })
  hide()
}

// ── Close on outside click or Escape ──────────────────────────────────────
function onClickOutside() { hide() }
function onKey(e) { if (e.key === 'Escape') hide() }

onMounted(() => {
  document.addEventListener('click',   onClickOutside)
  document.addEventListener('keydown', onKey)
})

onUnmounted(() => {
  document.removeEventListener('click',   onClickOutside)
  document.removeEventListener('keydown', onKey)
})

defineExpose({ show, hide })
</script>

<style>
.ctx-fade-enter-active,
.ctx-fade-leave-active {
  transition: opacity 0.12s ease, transform 0.12s ease;
}
.ctx-fade-enter-from,
.ctx-fade-leave-to {
  opacity: 0;
  transform: scale(0.96) translateY(-4px);
}
</style>
