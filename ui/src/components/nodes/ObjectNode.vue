<template>
  <div
    class="object-node rounded-xl border transition-all duration-200 cursor-pointer select-none
           flex flex-col gap-1.5 px-3 py-2.5 relative"
    :class="[
      selected
        ? 'border-emerald-400 shadow-[0_0_18px_rgba(16,185,129,0.45)]'
        : 'border-emerald-600/40 hover:border-emerald-400/70',
      data._isFlashing ? 'shadow-[0_0_18px_rgba(16,185,129,0.6)]' : '',
    ]"
    style="width:210px; min-height:82px"
  >
    <Handle type="target" :position="Position.Top"
      class="!w-2.5 !h-2.5 !border-2 !border-emerald-600 !bg-emerald-400" />

    <!-- Header row -->
    <div class="flex items-center gap-2">
      <div class="w-8 h-8 rounded-lg bg-emerald-900/70 border border-emerald-500/40
                  flex items-center justify-center flex-shrink-0">
        <Box class="w-4 h-4 text-emerald-300" />
      </div>
      <div class="min-w-0">
        <p class="text-xs font-semibold text-gray-100 truncate leading-tight">
          {{ data.meta.name }}
        </p>
        <p class="text-[10px] font-mono text-emerald-400/70 leading-tight">OBJECT</p>
      </div>
    </div>

    <!-- Live property preview (up to 3 rows) -->
    <div
      v-if="previewEntries.length"
      class="border-t border-emerald-900/40 pt-1.5 space-y-0.5"
    >
      <div
        v-for="([key, val]) in previewEntries"
        :key="key"
        class="flex items-center justify-between"
      >
        <span class="text-[9px] font-mono text-gray-600 truncate max-w-[90px]">{{ key }}</span>
        <span class="text-[9px] font-mono font-medium" :class="valColor(val)">
          {{ fmtVal(val) }}
        </span>
      </div>
      <p v-if="extraCount > 0" class="text-[9px] text-gray-700 text-right leading-tight">
        +{{ extraCount }} more
      </p>
    </div>

    <!-- Flash ring -->
    <div v-if="data._isFlashing"
         class="absolute inset-0 rounded-xl border-2 border-emerald-400 animate-ping opacity-60
                pointer-events-none" />

    <Handle type="source" :position="Position.Bottom"
      class="!w-2.5 !h-2.5 !border-2 !border-emerald-600 !bg-emerald-400" />
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { Handle, Position } from '@vue-flow/core'
import { Box } from 'lucide-vue-next'

const MAX = 3
const props = defineProps({
  data:     { type: Object,  required: true },
  selected: { type: Boolean, default:  false },
})

const allEntries     = computed(() => props.data._state ? Object.entries(props.data._state) : [])
const previewEntries = computed(() => allEntries.value.slice(0, MAX))
const extraCount     = computed(() => Math.max(0, allEntries.value.length - MAX))

function fmtVal(v) {
  if (v === null)             return 'null'
  if (typeof v === 'boolean') return String(v)
  if (typeof v === 'number')  return Number.isInteger(v) ? String(v) : v.toFixed(2).replace(/\.?0+$/, '')
  if (typeof v === 'string')  return v.length > 12 ? v.slice(0, 12) + '…' : v
  return '…'
}

function valColor(v) {
  if (typeof v === 'number')  return 'text-cyan-300'
  if (typeof v === 'boolean') return v ? 'text-emerald-300' : 'text-red-400'
  if (typeof v === 'string')  return 'text-amber-300'
  return 'text-gray-400'
}
</script>

<style scoped>
.object-node {
  background: linear-gradient(135deg, rgba(6,78,59,0.5), rgba(2,44,34,0.7));
  backdrop-filter: blur(6px);
}
</style>
