<template>
  <div
    class="class-node rounded-xl border transition-all duration-200 cursor-pointer select-none
           flex flex-col items-center justify-center gap-1.5 px-4 py-3 relative"
    :class="[
      selected
        ? 'border-cyan-400 shadow-[0_0_18px_rgba(6,182,212,0.45)]'
        : 'border-cyan-600/40 hover:border-cyan-400/70',
      data._isFlashing ? 'shadow-[0_0_18px_rgba(6,182,212,0.6)]' : '',
    ]"
    style="width:190px; min-height:76px"
  >
    <Handle type="target" :position="Position.Top"
      class="!w-2.5 !h-2.5 !border-2 !border-cyan-600 !bg-cyan-400" />

    <div class="flex items-center gap-2">
      <div class="w-8 h-8 rounded-lg bg-cyan-900/70 border border-cyan-500/40
                  flex items-center justify-center flex-shrink-0">
        <FileJson class="w-4 h-4 text-cyan-300" />
      </div>
      <div class="min-w-0">
        <p class="text-sm font-medium text-gray-100 truncate leading-tight">
          {{ data.meta.name }}
        </p>
        <p class="text-[10px] font-mono text-cyan-400/70 leading-tight">CLASS</p>
      </div>
    </div>

    <!-- Flash ring -->
    <div v-if="data._isFlashing"
         class="absolute inset-0 rounded-xl border-2 border-cyan-400 animate-ping opacity-60
                pointer-events-none" />

    <Handle type="source" :position="Position.Bottom"
      class="!w-2.5 !h-2.5 !border-2 !border-cyan-600 !bg-cyan-400" />
  </div>
</template>

<script setup>
import { Handle, Position } from '@vue-flow/core'
import { FileJson } from 'lucide-vue-next'
defineProps({
  data:     { type: Object,  required: true },
  selected: { type: Boolean, default:  false },
})
</script>

<style scoped>
.class-node {
  background: linear-gradient(135deg, rgba(22,78,99,0.5), rgba(8,51,68,0.7));
  backdrop-filter: blur(6px);
}
</style>
