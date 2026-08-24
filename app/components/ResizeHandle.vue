<script setup lang="ts">
import { usePanes } from '~/composables/usePanes'

const props = defineProps<{ side: 'sidebar' | 'panel' }>()
const { layout, start, reset } = usePanes()
</script>

<template>
  <div
    class="handle"
    :class="{ active: layout.dragging === props.side }"
    title="Drag to resize, double-click to reset"
    @pointerdown="start($event, props.side)"
    @dblclick="reset(props.side)"
  />
</template>

<style scoped>
.handle {
  width: 5px;
  margin: 0 -2px;
  z-index: 5;
  cursor: col-resize;
  background: transparent;
  transition: background 0.12s;
}

.handle:hover,
.handle.active {
  background: var(--accent);
}
</style>
