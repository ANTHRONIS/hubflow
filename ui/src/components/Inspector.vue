<template>
  <aside
    v-if="knot"
    class="flex flex-col h-full bg-gray-900 overflow-hidden"
  >
    <!-- ── Header ─────────────────────────────────────────────────────── -->
    <div class="flex items-center justify-between px-4 py-3 border-b border-gray-800 flex-shrink-0">
      <div class="flex items-center gap-2 min-w-0">
        <div
          class="w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0"
          :class="roleIconBg"
        >
          <component :is="roleIcon" class="w-3.5 h-3.5" :class="roleIconColor" />
        </div>
        <div class="min-w-0">
          <p class="text-sm font-semibold text-gray-100 truncate">{{ knot.meta.name }}</p>
          <p class="text-[10px] font-mono" :class="roleIconColor">{{ knot.role }}</p>
        </div>
      </div>
      <button
        @click="store.clearSelection()"
        class="p-1.5 text-gray-500 hover:text-gray-200 hover:bg-gray-700 rounded-md
               transition-colors flex-shrink-0"
        title="Close"
      >
        <X class="w-4 h-4" />
      </button>
    </div>

    <!-- ── Scrollable body ─────────────────────────────────────────────── -->
    <div class="flex-1 overflow-y-auto p-4 space-y-3">

      <!-- ── Identity card ─────────────────────────────────────────────── -->
      <div class="bg-gray-800/60 rounded-xl p-3 space-y-2.5">
        <InfoRow label="UUID">
          <span class="font-mono text-indigo-300 break-all text-[11px]">{{ knot.id }}</span>
        </InfoRow>
        <InfoRow label="Level">
          <span class="font-mono text-gray-300">{{ knot.level }}</span>
        </InfoRow>
        <InfoRow v-if="knot.parent_id" label="Parent">
          <button
            class="font-mono text-[11px] text-gray-400 hover:text-indigo-300 transition-colors truncate text-left"
            @click="store.selectKnot(knot.parent_id)"
          >{{ knot.parent_id }}</button>
        </InfoRow>
        <InfoRow v-if="knot.meta.description" label="Desc">
          <span class="text-gray-400 text-xs">{{ knot.meta.description }}</span>
        </InfoRow>
      </div>

      <!-- ── Children list ─────────────────────────────────────────────── -->
      <template v-if="knot.child_ids?.length">
        <div class="bg-gray-800/60 rounded-xl p-3">
          <p class="text-[10px] font-mono uppercase tracking-wider text-gray-500 mb-2">
            Children ({{ knot.child_ids.length }})
          </p>
          <ul class="space-y-1">
            <li
              v-for="cid in knot.child_ids"
              :key="cid"
              class="flex items-center gap-2 cursor-pointer group"
              @click="store.selectKnot(cid)"
            >
              <div
                class="w-2 h-2 rounded-full flex-shrink-0"
                :style="{ background: childColor(cid) }"
              />
              <span
                class="text-[11px] font-mono text-gray-500 group-hover:text-indigo-300
                       transition-colors truncate"
              >
                {{ childName(cid) || cid.slice(0, 16) + '…' }}
              </span>
            </li>
          </ul>
        </div>
      </template>

      <!-- ══ OBJECT: live property table ═════════════════════════════════ -->
      <template v-if="knot.role === 'Object'">
        <div class="border-t border-gray-800 pt-3">
          <div class="flex items-center justify-between mb-2">
            <p class="text-xs font-semibold text-gray-300 flex items-center gap-1.5">
              <Database class="w-3.5 h-3.5 text-emerald-400" />
              Live Properties
            </p>
            <button
              @click="refreshState"
              :disabled="loadingState"
              class="text-[11px] text-gray-500 hover:text-gray-200 flex items-center gap-1
                     transition-colors disabled:opacity-40"
            >
              <RefreshCw class="w-3 h-3" :class="{ 'animate-spin': loadingState }" />
              refresh
            </button>
          </div>

          <!-- Spinner -->
          <div v-if="loadingState" class="flex justify-center py-6">
            <div class="w-5 h-5 border-2 border-emerald-400 border-t-transparent
                        rounded-full animate-spin" />
          </div>

          <!-- Property rows -->
          <div v-else-if="stateEntries.length" class="space-y-1.5">
            <div
              v-for="[k, v] in stateEntries"
              :key="k"
              class="flex items-center justify-between py-1.5 px-3 bg-gray-800 rounded-lg"
            >
              <span class="text-[11px] font-mono text-gray-500">{{ k }}</span>
              <span class="text-[11px] font-mono font-medium" :class="valueClass(v)">
                {{ fmtValue(v) }}
              </span>
            </div>
          </div>

          <p v-else class="text-xs text-gray-600 text-center py-5 italic">
            No properties loaded
          </p>
        </div>
      </template>

      <!-- ══ ACTION: trigger panel ════════════════════════════════════════ -->
      <template v-if="knot.role === 'Action'">
        <div class="border-t border-gray-800 pt-3 space-y-3">
          <p class="text-xs font-semibold text-gray-300 flex items-center gap-1.5">
            <Zap class="w-3.5 h-3.5 text-amber-400" />
            Trigger Action
          </p>

          <!-- Payload editor -->
          <div>
            <label class="text-[10px] text-gray-500 uppercase tracking-wider font-mono mb-1.5 block">
              Payload (JSON)
            </label>
            <textarea
              v-model="triggerPayload"
              rows="5"
              spellcheck="false"
              class="w-full bg-gray-800 border rounded-lg px-3 py-2 text-xs font-mono
                     text-gray-200 focus:outline-none resize-none transition-colors"
              :class="payloadError
                ? 'border-red-600 focus:border-red-500'
                : 'border-gray-700 focus:border-amber-500'"
              placeholder='{ "n": 42 }'
            />
            <p v-if="payloadError" class="text-[10px] text-red-400 mt-1">{{ payloadError }}</p>
          </div>

          <!-- Priority slider -->
          <div class="flex items-center gap-3">
            <label class="text-[10px] text-gray-500 uppercase tracking-wider font-mono w-14 flex-shrink-0">
              Priority
            </label>
            <input
              v-model.number="triggerPriority"
              type="range" min="0" max="255" step="1"
              class="flex-1 accent-amber-500 h-1.5"
            />
            <span class="text-xs font-mono text-amber-400 w-8 text-right">
              {{ triggerPriority }}
            </span>
          </div>

          <!-- Send button -->
          <button
            @click="triggerAction"
            :disabled="!!payloadError || triggerLoading"
            class="w-full py-2 text-sm font-medium rounded-lg transition-colors
                   flex items-center justify-center gap-2
                   bg-amber-600 hover:bg-amber-500
                   disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <div v-if="triggerLoading"
                 class="w-4 h-4 border-2 border-white border-t-transparent rounded-full animate-spin" />
            <Zap v-else class="w-4 h-4" />
            {{ triggerLoading ? 'Sending…' : 'Send Packet' }}
          </button>

          <!-- Feedback -->
          <Transition name="fade-quick">
            <p v-if="triggerSuccess" class="text-xs text-emerald-400 flex items-center gap-1">
              <CheckCircle class="w-3.5 h-3.5" /> Packet queued
            </p>
          </Transition>
        </div>
      </template>

      <!-- ══ CLASS: schema hint ═══════════════════════════════════════════ -->
      <template v-if="knot.role === 'Class'">
        <div class="border-t border-gray-800 pt-3">
          <p class="text-xs font-semibold text-gray-300 flex items-center gap-1.5 mb-2">
            <FileJson class="w-3.5 h-3.5 text-cyan-400" />
            Schema
          </p>
          <p class="text-xs text-gray-600 italic">
            Send a packet to this Knot to receive its full schema definition.
          </p>
          <button
            @click="querySchema"
            class="mt-3 w-full py-1.5 text-xs font-medium rounded-lg border border-cyan-700/60
                   text-cyan-300 hover:bg-cyan-900/30 transition-colors"
          >
            Query Schema
          </button>
        </div>
      </template>

    </div><!-- end scroll body -->
  </aside>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import {
  X, Database, RefreshCw, Zap, CheckCircle,
  FileJson, Network, Box, Cpu,
} from 'lucide-vue-next'
import { useHubFlowStore, ROLE_COLOR } from '../stores/hubflow'

// ── Reusable tiny InfoRow component (local) ──────────────────────────────
const InfoRow = {
  props: ['label'],
  template: `
    <div class="flex items-start gap-2">
      <span class="text-[10px] font-mono uppercase tracking-wider text-gray-600 w-10 flex-shrink-0 pt-0.5">
        {{ label }}
      </span>
      <div class="flex-1 min-w-0"><slot /></div>
    </div>
  `,
}

const store = useHubFlowStore()
const knot  = computed(() => store.selectedKnot)

// ── Role visual mapping ──────────────────────────────────────────────────
const ROLE_ICON = { Hub: Network, Object: Box, Action: Cpu, Class: FileJson }
const ROLE_IBGC = {
  Hub:    'bg-indigo-900/60',
  Object: 'bg-emerald-900/60',
  Action: 'bg-amber-900/60',
  Class:  'bg-cyan-900/60',
}
const ROLE_ICLR = {
  Hub:    'text-indigo-300',
  Object: 'text-emerald-300',
  Action: 'text-amber-300',
  Class:  'text-cyan-300',
}

const roleIcon      = computed(() => ROLE_ICON[knot.value?.role] ?? Network)
const roleIconBg    = computed(() => ROLE_IBGC[knot.value?.role] ?? 'bg-gray-800')
const roleIconColor = computed(() => ROLE_ICLR[knot.value?.role] ?? 'text-gray-300')

// ── Children helpers ─────────────────────────────────────────────────────
function childName(cid)  { return store.knotMap?.get(cid)?.meta?.name ?? null }
function childColor(cid) { return ROLE_COLOR[store.knotMap?.get(cid)?.role] ?? '#6366f1' }

// ── Object state ─────────────────────────────────────────────────────────
const loadingState = ref(false)
const stateEntries = computed(() => {
  const s = knot.value ? store.knotStates[knot.value.id] : null
  return s ? Object.entries(s) : []
})

async function refreshState() {
  if (!knot.value) return
  loadingState.value = true
  await store.fetchKnotState(knot.value.id)
  loadingState.value = false
}

// Auto-refresh when the selected knot changes to an Object
watch(() => knot.value?.id, (id) => {
  if (id && knot.value?.role === 'Object') refreshState()
})

function fmtValue(v) {
  if (v === null || v === undefined) return 'null'
  if (typeof v === 'boolean') return v ? 'true' : 'false'
  if (typeof v === 'number')  return String(v)
  if (typeof v === 'string')  return `"${v}"`
  return JSON.stringify(v)
}

function valueClass(v) {
  if (typeof v === 'number')  return 'text-cyan-300'
  if (typeof v === 'boolean') return v ? 'text-emerald-300' : 'text-red-400'
  if (typeof v === 'string')  return 'text-amber-300'
  return 'text-gray-300'
}

// ── Action trigger ────────────────────────────────────────────────────────
const triggerPayload  = ref('{\n  "n": 42\n}')
const triggerPriority = ref(200)
const triggerLoading  = ref(false)
const triggerSuccess  = ref(false)

const payloadError = computed(() => {
  if (!triggerPayload.value.trim()) return null
  try { JSON.parse(triggerPayload.value); return null }
  catch (e) { return e.message }
})

async function triggerAction() {
  if (!knot.value || payloadError.value) return
  triggerLoading.value = true
  triggerSuccess.value = false
  const ok = await store.injectPacket({
    targetId: knot.value.id,
    payload:  JSON.parse(triggerPayload.value),
    priority: triggerPriority.value,
  })
  triggerLoading.value = false
  if (ok) {
    triggerSuccess.value = true
    setTimeout(() => { triggerSuccess.value = false }, 3000)
  }
}

// ── Class schema query ────────────────────────────────────────────────────
async function querySchema() {
  if (!knot.value) return
  await store.injectPacket({
    targetId: knot.value.id,
    payload:  { op: 'get_schema' },
    priority: 128,
  })
}
</script>

<style>
.fade-quick-enter-active, .fade-quick-leave-active { transition: opacity 0.25s; }
.fade-quick-enter-from,   .fade-quick-leave-to     { opacity: 0; }
</style>
