<template>
  <div class="h-full min-h-0 flex flex-col overflow-hidden">
    <aside class="flex flex-col flex-1 min-h-0 border-b border-slate-800/80">
      <div class="px-3 py-2.5 border-b border-slate-800/60">
        <p class="text-[10px] font-mono uppercase tracking-widest text-slate-500">
          Draggable templates
        </p>
        <p class="text-xs text-slate-400 mt-0.5">Drag onto canvas to create a knot</p>
      </div>

      <div class="flex-1 overflow-y-auto p-3 space-y-2.5">
        <button
          v-for="item in TEMPLATES"
          :key="item.role"
          type="button"
          draggable="true"
          class="w-full text-left rounded-xl border p-3 transition-all cursor-grab active:cursor-grabbing
                 hover:ring-1 hover:ring-slate-600 flex items-start gap-3
                 border-slate-700/80 bg-slate-800/40 hover:bg-slate-800/70 select-none
                 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          :class="item.cardClass"
          :title="`Drag to create ${item.label}`"
          @dragstart="onTemplateDragStart($event, item.role)"
          @click="onCreate(item.role)"
        >
          <div
            class="w-10 h-10 rounded-lg flex items-center justify-center flex-shrink-0 border"
            :class="[item.iconBoxClass]"
          >
            <component :is="item.icon" class="w-5 h-5" />
          </div>
          <div class="min-w-0">
            <p class="text-sm font-semibold text-slate-100 leading-tight">{{ item.label }}</p>
            <p class="text-[11px] text-slate-500 mt-0.5">{{ item.hint }}</p>
          </div>
        </button>
      </div>
    </aside>

    <!-- Layout / save -->
    <div class="p-3 border-t border-slate-800/80 space-y-2 bg-slate-900/30">
      <p class="text-[10px] font-mono uppercase tracking-widest text-slate-500 px-0.5">Layout</p>
      <div class="flex flex-col gap-1.5">
        <button
          type="button"
          class="w-full flex items-center gap-2 px-2.5 py-2 rounded-lg text-slate-300 text-xs
                 hover:bg-slate-800 border border-transparent hover:border-slate-700"
          @click="emit('tidy')"
        >
          <LayoutDashboard class="w-3.5 h-3.5 text-slate-500" />
          Tidy up
        </button>
        <button
          type="button"
          class="w-full flex items-center gap-2 px-2.5 py-2 rounded-lg text-xs
                 border border-transparent hover:border-slate-700 hover:bg-slate-800"
          :class="snapGrid ? 'text-indigo-300' : 'text-slate-300'"
          @click="emit('toggle-snap')"
        >
          <Grid2x2 class="w-3.5 h-3.5" :class="snapGrid ? 'text-indigo-400' : 'text-slate-500'" />
          Snap grid
        </button>
        <button
          type="button"
          class="w-full flex items-center gap-2 px-2.5 py-2 rounded-lg text-emerald-300/90 text-xs
                 hover:bg-slate-800 border border-transparent hover:border-slate-700"
          @click="emit('save')"
        >
          <Save class="w-3.5 h-3.5 text-emerald-500" />
          Sync from server
        </button>
      </div>
    </div>

    <Teleport to="body">
    <Transition name="dialog-fade">
      <div
        v-if="dialog.open"
        class="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50"
        @click.self="dialog.open = false"
      >
        <div class="bg-gray-900 border border-gray-700 rounded-2xl shadow-2xl p-6 w-80">
          <div class="flex items-center gap-2 mb-4">
            <div
              class="w-8 h-8 rounded-lg flex items-center justify-center"
              :style="{ background: roleColor(dialog.role) + '30', border: `1px solid ${roleColor(dialog.role)}60` }"
            >
              <component :is="roleIconComponent(dialog.role)" class="w-4 h-4" :style="{ color: roleColor(dialog.role) }" />
            </div>
            <h2 class="text-sm font-semibold text-gray-100">New {{ dialog.role }}</h2>
          </div>

          <div class="space-y-3">
            <div>
              <label class="text-[10px] font-mono uppercase tracking-wider text-gray-500 block mb-1">Name *</label>
              <input
                ref="nameInput"
                v-model="dialog.name"
                @keyup.enter="confirmCreate"
                class="w-full bg-gray-800 border border-gray-700 focus:border-indigo-500
                       rounded-lg px-3 py-2 text-sm text-gray-100 outline-none transition-colors"
                :placeholder="`My ${dialog.role}`"
              />
            </div>
            <div>
              <label class="text-[10px] font-mono uppercase tracking-wider text-gray-500 block mb-1">Description</label>
              <input
                v-model="dialog.description"
                class="w-full bg-gray-800 border border-gray-700 focus:border-indigo-500
                       rounded-lg px-3 py-2 text-sm text-gray-100 outline-none transition-colors"
                placeholder="Optional"
              />
            </div>
            <div>
              <label class="text-[10px] font-mono uppercase tracking-wider text-gray-500 block mb-1">
                Parent ({{ parentName }})
              </label>
              <select
                v-model="dialog.parentId"
                class="w-full bg-gray-800 border border-gray-700 focus:border-indigo-500
                       rounded-lg px-3 py-2 text-sm text-gray-300 outline-none transition-colors"
              >
                <option :value="null">— none (root level) —</option>
                <option v-for="k in store.knots" :key="k.id" :value="k.id">
                  {{ k.meta.name }} ({{ k.role }})
                </option>
              </select>
            </div>
          </div>

          <div class="flex gap-2 mt-5">
            <button
              @click="dialog.open = false"
              class="flex-1 py-2 text-sm text-gray-400 hover:text-gray-100 border border-gray-700
                     hover:border-gray-500 rounded-lg transition-colors"
            >
              Cancel
            </button>
            <button
              @click="confirmCreate"
              :disabled="!dialog.name.trim() || creating"
              class="flex-1 py-2 text-sm font-medium rounded-lg transition-colors
                     bg-indigo-600 hover:bg-indigo-500 disabled:opacity-40 disabled:cursor-not-allowed
                     flex items-center justify-center gap-2"
            >
              <div v-if="creating" class="w-3.5 h-3.5 border-2 border-white border-t-transparent rounded-full animate-spin" />
              <Plus v-else class="w-3.5 h-3.5" />
              Create
            </button>
          </div>
        </div>
      </div>
    </Transition>
    </Teleport>
  </div>
</template>

<script setup>
import { ref, computed, nextTick } from 'vue'
import {
  Plus, Activity, Box, Zap, Layers,
  LayoutDashboard, Grid2x2, Save,
} from 'lucide-vue-next'
import { useHubFlowStore, ROLE_COLOR } from '../stores/hubflow'

const props = defineProps({
  snapGrid: { type: Boolean, default: false },
})

const emit = defineEmits(['tidy', 'toggle-snap', 'save'])
const store = useHubFlowStore()

/** Knot role strings as expected by the API: Hub | Object | Action | Class */
const TEMPLATES = [
  { role: 'Hub',    label: 'Hub',    hint: 'Root & routing', icon: Activity,  cardClass: 'ring-1 ring-indigo-500/20',  iconBoxClass: 'bg-indigo-950/60 border-indigo-500/30 text-indigo-300' },
  { role: 'Object', label: 'Object', hint: 'Data instance',  icon: Box,       cardClass: 'ring-1 ring-emerald-500/20',  iconBoxClass: 'bg-emerald-950/60 border-emerald-500/30 text-emerald-300' },
  { role: 'Action', label: 'Action', hint: 'Behaviour',      icon: Zap,      cardClass: 'ring-1 ring-amber-500/20',  iconBoxClass: 'bg-amber-950/60 border-amber-500/30 text-amber-300' },
  { role: 'Class',  label: 'Class',  hint: 'Schema / type',  icon: Layers,   cardClass: 'ring-1 ring-cyan-500/20',  iconBoxClass: 'bg-cyan-950/60 border-cyan-500/30 text-cyan-300' },
]

const dialog = ref({ open: false, role: 'Hub', name: '', description: '', parentId: null, x: 0, y: 0 })
const creating = ref(false)
const nameInput = ref(null)

const parentName = computed(() => {
  if (!dialog.value.parentId) return 'none'
  return store.knotMap.get(dialog.value.parentId)?.meta?.name ?? '?'
})

function roleColor(role) { return ROLE_COLOR[role] ?? '#6366f1' }
function roleIconComponent(role) {
  return { Hub: Activity, Object: Box, Action: Zap, Class: Layers }[role] ?? Activity
}

function onCreate(role) {
  dialog.value = {
    open: true,
    role,
    name: '',
    description: '',
    parentId: store.selectedKnotId,
    x: dialog.value.x,
    y: dialog.value.y,
  }
  nextTick(() => nameInput.value?.focus())
}

async function confirmCreate() {
  if (!dialog.value.name.trim() || creating.value) return
  creating.value = true
  await store.addKnot({
    name:        dialog.value.name.trim(),
    description: dialog.value.description.trim() || null,
    role:        dialog.value.role,
    parentId:    dialog.value.parentId,
    x:           dialog.value.x,
    y:           dialog.value.y,
  })
  creating.value = false
  dialog.value.open = false
}

/**
 * Vue Flow convention: payload type in application/vueflow (plus fallbacks).
 */
function onTemplateDragStart(event, role) {
  const payload = String(role)
  try {
    event.dataTransfer.setData('application/vueflow', payload)
  } catch {
    // ignore
  }
  event.dataTransfer.setData('text/plain', payload)
  event.dataTransfer.setData('hubflow/role', payload)
  event.dataTransfer.effectAllowed = 'copy'
}

function openAt(role, x, y) {
  dialog.value = {
    open: true,
    role,
    name: '',
    description: '',
    parentId: store.selectedKnotId,
    x,
    y,
  }
  nextTick(() => nameInput.value?.focus())
}

defineExpose({ openAt })
</script>

<style>
.dialog-fade-enter-active, .dialog-fade-leave-active { transition: opacity 0.2s; }
.dialog-fade-enter-from,   .dialog-fade-leave-to     { opacity: 0; }
</style>
