<template>
  <div class="flex flex-col h-screen overflow-hidden bg-[#070c1a] text-gray-100 font-sans">

    <!-- ── Top bar ──────────────────────────────────────────────────────── -->
    <header
      class="flex items-center justify-between px-5 py-2.5 bg-gray-900/80 border-b border-gray-800
             backdrop-blur flex-shrink-0 z-10"
    >
      <!-- Brand -->
      <div class="flex items-center gap-3">
        <div class="w-8 h-8 rounded-full bg-gradient-to-br from-indigo-500 to-indigo-700
                    flex items-center justify-center shadow-lg shadow-indigo-900/40">
          <Network class="w-4 h-4 text-white" />
        </div>
        <span class="text-base font-bold tracking-tight bg-gradient-to-r from-indigo-300 to-cyan-300
                     bg-clip-text text-transparent">HubFlow</span>
        <span class="hidden sm:block text-xs text-gray-600 font-mono">Visual Editor v0.3</span>
      </div>

      <!-- Right section -->
      <div class="flex items-center gap-4">
        <!-- Knot counter -->
        <span v-if="store.knots.length" class="hidden sm:block text-xs text-gray-500 font-mono">
          {{ store.knots.length }} knots
        </span>

        <!-- Refresh button -->
        <button
          @click="store.fetchGraph()"
          title="Refresh topology"
          class="p-1.5 text-gray-400 hover:text-gray-100 hover:bg-gray-700 rounded-md transition-colors"
        >
          <RefreshCw class="w-4 h-4" />
        </button>

        <ConnectionStatus />
      </div>
    </header>

    <!-- ── Main area ─────────────────────────────────────────────────────── -->
    <div class="flex flex-1 overflow-hidden relative">

      <!-- Left: Node creation toolbar -->
      <NodeToolbar
        ref="toolbar"
        :snap-grid="snapGrid"
        @tidy="canvasRef?.tidyUp()"
        @toggle-snap="toggleSnap"
        @save="onSave"
      />

      <!-- Center: Canvas -->
      <div class="flex-1 relative min-w-0">
        <Canvas ref="canvasRef" @open-create="onOpenCreate" />

        <!-- Empty-state overlay (shown before any knots load) -->
        <Transition name="fade">
          <div
            v-if="!store.knots.length && store.connectionStatus !== 'connecting'"
            class="absolute inset-0 flex flex-col items-center justify-center pointer-events-none"
          >
            <div class="text-center p-8 rounded-2xl bg-gray-900/60 backdrop-blur border border-gray-800 max-w-sm">
              <ServerOff class="w-14 h-14 text-gray-600 mx-auto mb-4" />
              <p class="text-gray-300 font-medium mb-1">No Knots visible</p>
              <p class="text-sm text-gray-500 mb-4">
                Make sure the HubFlow backend is running on
                <code class="font-mono text-indigo-400">:8080</code>.
              </p>
              <button
                class="pointer-events-auto px-4 py-2 bg-indigo-600 hover:bg-indigo-500
                       text-white text-sm rounded-lg transition-colors"
                @click="store.fetchGraph()"
              >
                Retry
              </button>
            </div>
          </div>
        </Transition>
      </div>

      <!-- Inspector slides in from the right when a knot is selected -->
      <Transition name="slide-right">
        <Inspector
          v-if="store.selectedKnot"
          class="w-80 flex-shrink-0 border-l border-gray-800"
        />
      </Transition>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { Network, RefreshCw, ServerOff } from 'lucide-vue-next'
import { useStorage } from '@vueuse/core'
import { useHubFlowStore } from './stores/hubflow'
import ConnectionStatus from './components/ConnectionStatus.vue'
import Canvas           from './components/Canvas.vue'
import Inspector        from './components/Inspector.vue'
import NodeToolbar      from './components/NodeToolbar.vue'

const store = useHubFlowStore()

// ── Toolbar / canvas refs ─────────────────────────────────────────────────
const toolbar   = ref(null)
const canvasRef = ref(null)
const snapGrid  = ref(false)

function toggleSnap() {
  snapGrid.value = !snapGrid.value
  canvasRef.value?.toggleSnapGrid()
}

function onOpenCreate({ role, x, y }) {
  toolbar.value?.openAt(role, x, y)
}

async function onSave() {
  await store.fetchGraph()  // sync from server = confirm everything is saved
  console.log('[HubFlow] Project synced from server (auto-save always active)')
}

// ── Viewport persistence ──────────────────────────────────────────────────
const _viewport = useStorage('hubflow-viewport', { x: 0, y: 0, zoom: 1 })

onMounted(async () => {
  await store.fetchGraph()   // get initial topology
  store.connect()            // then open WS for live updates
})

onUnmounted(() => store.disconnect())
</script>

<style>
/* ── Slide-right transition (Inspector panel) ─────────────────────────── */
.slide-right-enter-active,
.slide-right-leave-active {
  transition: transform 0.22s cubic-bezier(.4,0,.2,1),
              opacity   0.22s cubic-bezier(.4,0,.2,1);
}
.slide-right-enter-from,
.slide-right-leave-to  { transform: translateX(24px); opacity: 0; }

/* ── Fade transition (empty-state overlay) ────────────────────────────── */
.fade-enter-active, .fade-leave-active { transition: opacity 0.3s; }
.fade-enter-from,   .fade-leave-to     { opacity: 0; }
</style>
