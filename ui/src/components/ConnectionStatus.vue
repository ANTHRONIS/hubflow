<template>
  <div class="flex items-center gap-2 select-none">

    <!-- Status pill -->
    <div
      class="flex items-center gap-1.5 px-3 py-1 rounded-full text-xs font-medium border
             transition-all duration-300"
      :class="pill.container"
    >
      <!-- Animated dot -->
      <div class="relative flex-shrink-0 w-2 h-2">
        <div class="w-2 h-2 rounded-full" :class="pill.dot" />
        <div
          v-if="status === 'connected'"
          class="absolute inset-0 rounded-full animate-ping opacity-75"
          :class="pill.dot"
        />
      </div>
      <span>{{ pill.label }}</span>
    </div>

    <!-- Manual reconnect button -->
    <button
      v-if="status === 'disconnected'"
      class="text-xs text-gray-500 hover:text-indigo-300 underline decoration-dotted
             transition-colors"
      @click="store.connect()"
    >
      reconnect
    </button>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { useHubFlowStore } from '../stores/hubflow'

const store  = useHubFlowStore()
const status = computed(() => store.connectionStatus)

const PILLS = {
  connected:    { container: 'bg-emerald-950/60 text-emerald-300 border-emerald-700/50', dot: 'bg-emerald-400', label: 'Connected'    },
  connecting:   { container: 'bg-yellow-950/60  text-yellow-300  border-yellow-700/50',  dot: 'bg-yellow-400',  label: 'Connecting…'  },
  disconnected: { container: 'bg-red-950/60     text-red-300     border-red-700/50',     dot: 'bg-red-500',     label: 'Disconnected' },
}

const pill = computed(() => PILLS[status.value] ?? PILLS.disconnected)
</script>
