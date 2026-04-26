<template>
  <div
    class="action-node rounded-xl border transition-all duration-200 cursor-pointer select-none
           flex flex-col items-center justify-center gap-1.5 px-4 py-3 relative"
    :class="[
      selected
        ? 'border-amber-400 shadow-[0_0_18px_rgba(245,158,11,0.45)]'
        : 'border-amber-600/40 hover:border-amber-400/70',
      data._isFlashing ? 'shadow-[0_0_18px_rgba(245,158,11,0.6)]' : '',
    ]"
    style="width:190px; min-height:76px"
  >
    <Handle type="target" :position="Position.Top"
      class="!border-amber-500 !bg-amber-800" />

    <div class="flex items-center gap-2">
      <div class="w-8 h-8 rounded-lg bg-amber-900/70 border border-amber-500/40
                  flex items-center justify-center flex-shrink-0">
        <!-- Gear spins while the node is active -->
        <Cpu
          class="w-4 h-4 text-amber-300 transition-transform duration-700"
          :class="{ 'animate-spin': data._isFlashing }"
        />
      </div>
      <div class="min-w-0">
        <p class="text-sm font-medium text-gray-100 truncate leading-tight">
          {{ data.meta.name }}
        </p>
        <p class="text-[10px] font-mono text-amber-400/70 leading-tight">ACTION</p>
      </div>
    </div>

    <!-- Active pulse ring -->
    <div v-if="data._isFlashing"
         class="absolute inset-0 rounded-xl border-2 border-amber-400 animate-ping opacity-60
                pointer-events-none" />

    <Handle type="source" :position="Position.Bottom"
      class="!border-amber-400 !bg-amber-600" />
  </div>
</template>

<script setup>
import { Handle, Position } from '@vue-flow/core'
import { Cpu } from 'lucide-vue-next'
defineProps({
  data:     { type: Object,  required: true },
  selected: { type: Boolean, default:  false },
})
</script>

<style scoped>
.action-node {
  background: linear-gradient(135deg, rgba(120,53,15,0.5), rgba(69,26,3,0.7));
  backdrop-filter: blur(6px);
}
</style>
