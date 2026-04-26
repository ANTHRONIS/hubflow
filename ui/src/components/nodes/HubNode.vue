<template>
  <div
    class="hub-node rounded-xl border transition-all duration-200 cursor-pointer select-none
           flex flex-col items-center justify-center gap-1.5 px-4 py-3 relative"
    :class="[
      selected
        ? 'border-indigo-400 shadow-[0_0_22px_rgba(99,102,241,0.45)]'
        : 'border-indigo-600/40 hover:border-indigo-400/70',
      data._isFlashing ? 'shadow-[0_0_22px_rgba(99,102,241,0.6)]' : '',
    ]"
    style="width:210px; min-height:84px"
  >
    <!-- Top connection handle (target: receives connections) -->
    <Handle type="target" :position="Position.Top"
      class="!border-indigo-500 !bg-indigo-800" />

    <!-- Icon row -->
    <div class="flex items-center gap-2">
      <div class="w-9 h-9 rounded-full bg-indigo-900/70 border border-indigo-500/40
                  flex items-center justify-center flex-shrink-0 shadow-inner">
        <Globe class="w-5 h-5 text-indigo-300" />
      </div>
      <div class="min-w-0">
        <p class="text-sm font-semibold text-gray-100 truncate leading-tight">
          {{ data.meta.name }}
        </p>
        <p class="text-[10px] font-mono text-indigo-400/70 leading-tight">
          HUB · L{{ data.level }}
        </p>
      </div>
    </div>

    <!-- Child count badge -->
    <div v-if="data.child_ids?.length" class="flex items-center gap-1">
      <span
        class="px-1.5 py-0.5 rounded-full bg-indigo-900/50 text-indigo-300 text-[9px] font-mono"
      >{{ data.child_ids.length }} children</span>
    </div>

    <!-- Flash ring -->
    <div v-if="data._isFlashing"
         class="absolute inset-0 rounded-xl border-2 border-indigo-400 animate-ping opacity-60
                pointer-events-none" />

    <!-- Bottom connection handle (source: starts connections) -->
    <Handle type="source" :position="Position.Bottom"
      class="!border-indigo-400 !bg-indigo-600" />
  </div>
</template>

<script setup>
import { Handle, Position } from '@vue-flow/core'
import { Globe } from 'lucide-vue-next'

defineProps({
  data:     { type: Object,  required: true },
  selected: { type: Boolean, default:  false },
})
</script>

<style scoped>
.hub-node {
  background: linear-gradient(135deg, rgba(49,46,129,0.5), rgba(30,27,75,0.7));
  backdrop-filter: blur(6px);
}
</style>
