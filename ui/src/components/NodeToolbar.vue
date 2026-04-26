<template>
  <!-- Collapsible left sidebar ──────────────────────────────────────────── -->
  <aside
    class="flex flex-col bg-gray-900 border-r border-gray-800 transition-all duration-200 flex-shrink-0"
    :class="open ? 'w-48' : 'w-14'"
  >
    <!-- Toggle button -->
    <button
      @click="open = !open"
      class="flex items-center justify-center h-10 w-full border-b border-gray-800
             text-gray-500 hover:text-gray-200 hover:bg-gray-800 transition-colors"
      :title="open ? 'Collapse toolbar' : 'Expand toolbar'"
    >
      <ChevronRight class="w-4 h-4 transition-transform" :class="open ? 'rotate-180' : ''" />
    </button>

    <!-- ── Create nodes ──────────────────────────────────────────────────── -->
    <div class="flex flex-col gap-1 p-2 flex-1">
      <p v-if="open" class="text-[9px] font-mono uppercase tracking-widest text-gray-600 px-1 pt-1 pb-0.5">
        Create
      </p>

      <ToolbarItem
        v-for="item in NODE_TYPES"
        :key="item.role"
        :icon="item.icon"
        :label="item.label"
        :color="item.color"
        :role="item.role"
        :expanded="open"
        @click="onCreate(item.role)"
        @dragstart="onDragStart($event, item.role)"
      />
    </div>

    <!-- ── Layout actions ────────────────────────────────────────────────── -->
    <div class="p-2 border-t border-gray-800 space-y-1">
      <p v-if="open" class="text-[9px] font-mono uppercase tracking-widest text-gray-600 px-1 pb-0.5">
        Layout
      </p>

      <ToolbarItem
        icon="LayoutDashboard"
        label="Tidy Up"
        color="#9ca3af"
        :expanded="open"
        @click="emit('tidy')"
      />
      <ToolbarItem
        icon="Grid2x2"
        label="Snap Grid"
        :color="snapGrid ? '#6366f1' : '#9ca3af'"
        :expanded="open"
        @click="emit('toggle-snap')"
      />
    </div>

    <!-- ── Save ──────────────────────────────────────────────────────────── -->
    <div class="p-2 border-t border-gray-800">
      <ToolbarItem
        icon="Save"
        label="Save"
        color="#10b981"
        :expanded="open"
        @click="emit('save')"
      />
    </div>
  </aside>

  <!-- ── "Create Knot" dialog ─────────────────────────────────────────── -->
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
</template>

<script setup>
import { ref, computed, nextTick } from 'vue'
import {
  ChevronRight, Plus, Globe, Box, Cpu, FileJson,
  LayoutDashboard, Grid2x2, Save,
} from 'lucide-vue-next'
import { useHubFlowStore, ROLE_COLOR } from '../stores/hubflow'

const props = defineProps({
  snapGrid: { type: Boolean, default: false },
})

const emit = defineEmits(['tidy', 'toggle-snap', 'save'])
const store = useHubFlowStore()

const open = ref(true)

// ── Node type definitions ──────────────────────────────────────────────────
const NODE_TYPES = [
  { role: 'Hub',    label: 'Hub',    icon: 'Globe',    color: ROLE_COLOR.Hub    },
  { role: 'Object', label: 'Object', icon: 'Box',      color: ROLE_COLOR.Object },
  { role: 'Action', label: 'Action', icon: 'Cpu',      color: ROLE_COLOR.Action },
  { role: 'Class',  label: 'Class',  icon: 'FileJson', color: ROLE_COLOR.Class  },
]

const ICON_MAP = { Globe, Box, Cpu, FileJson, LayoutDashboard, Grid2x2, Save }

// ── Draggable toolbar item ─────────────────────────────────────────────────
const ToolbarItem = {
  props: ['icon', 'label', 'color', 'role', 'expanded'],
  emits: ['click', 'dragstart'],
  setup(props, { emit: e }) {
    return { ICON_MAP, e }
  },
  template: `
    <button
      :draggable="!!role"
      @click="e('click')"
      @dragstart="e('dragstart', $event)"
      class="flex items-center gap-2.5 w-full px-2 py-2 rounded-lg text-left
             hover:bg-gray-800 transition-colors group"
      :title="label"
    >
      <div
        class="w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0"
        :style="{ background: color + '25', border: '1px solid ' + color + '50' }"
      >
        <component :is="ICON_MAP[icon]" class="w-3.5 h-3.5" :style="{ color }" />
      </div>
      <span v-if="expanded" class="text-xs font-medium text-gray-300 group-hover:text-gray-100 transition-colors">
        {{ label }}
      </span>
    </button>
  `,
}

// ── Dialog state ───────────────────────────────────────────────────────────
const dialog = ref({ open: false, role: 'Hub', name: '', description: '', parentId: null, x: 0, y: 0 })
const creating = ref(false)
const nameInput = ref(null)

const parentName = computed(() => {
  if (!dialog.value.parentId) return 'none'
  return store.knotMap.get(dialog.value.parentId)?.meta?.name ?? '?'
})

function roleColor(role) { return ROLE_COLOR[role] ?? '#6366f1' }
function roleIconComponent(role) { return { Hub: Globe, Object: Box, Action: Cpu, Class: FileJson }[role] ?? Globe }

function onCreate(role) {
  dialog.value = {
    open: true,
    role,
    name: '',
    description: '',
    parentId: store.selectedKnotId,  // default to currently selected knot
    x: dialog.value.x,
    y: dialog.value.y,
  }
  nextTick(() => nameInput.value?.focus())
}

async function confirmCreate() {
  if (!dialog.value.name.trim() || creating.value) return
  creating.value = true
  await store.createKnot({
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

// ── Drag start: store role in dataTransfer ─────────────────────────────────
function onDragStart(event, role) {
  event.dataTransfer.setData('hubflow/role', role)
  event.dataTransfer.effectAllowed = 'copy'
}

// ── Exposed: open dialog at a specific canvas position ─────────────────────
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
