<template>
  <VueFlow
    :nodes="store.vfNodes"
    :edges="store.vfEdges"
    :node-types="nodeTypes"
    :edges-updatable="false"
    fit-view-on-init
    :min-zoom="0.15"
    :max-zoom="2.5"
    class="hubflow-canvas"
    @node-click="onNodeClick"
    @pane-click="store.clearSelection()"
  >
    <!-- Dotted dark background grid -->
    <Background
      variant="dots"
      :gap="28"
      :size="1.2"
      pattern-color="#1e293b"
      bg-color="#070c1a"
    />

    <!-- Zoom / fit controls (styled dark in global CSS) -->
    <Controls position="bottom-left" />

    <!-- Mini-map with role-based node colours -->
    <MiniMap
      position="bottom-right"
      :node-color="minimapColor"
      :node-stroke-color="minimapColor"
      node-stroke-width="2"
      :mask-color="'rgba(7,12,26,0.7)'"
    />
  </VueFlow>
</template>

<script setup>
import { markRaw } from 'vue'
import { VueFlow  } from '@vue-flow/core'
import { Background } from '@vue-flow/background'
import { Controls   } from '@vue-flow/controls'
import { MiniMap    } from '@vue-flow/minimap'
import '@vue-flow/controls/dist/style.css'
import '@vue-flow/minimap/dist/style.css'
import { useHubFlowStore, ROLE_COLOR } from '../stores/hubflow'

import HubNode    from './nodes/HubNode.vue'
import ClassNode  from './nodes/ClassNode.vue'
import ObjectNode from './nodes/ObjectNode.vue'
import ActionNode from './nodes/ActionNode.vue'

const store = useHubFlowStore()

// markRaw prevents Vue from making the component reactive (required by Vue Flow)
const nodeTypes = {
  hub:    markRaw(HubNode),
  class:  markRaw(ClassNode),
  object: markRaw(ObjectNode),
  action: markRaw(ActionNode),
}

function onNodeClick({ node }) {
  store.selectKnot(node.id)
}

function minimapColor(node) {
  return ROLE_COLOR[node.data?.role] ?? '#6366f1'
}
</script>

<style>
/* Scoped overrides for the canvas container */
.hubflow-canvas {
  background: #070c1a;
  width: 100%;
  height: 100%;
}

/* Selected node ring */
.vue-flow__node.selected > * {
  outline: 2px solid rgba(99,102,241,0.8);
  outline-offset: 3px;
  border-radius: 12px;
}
</style>
