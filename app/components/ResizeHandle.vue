<script setup lang="ts">
import { usePanes, type Edge } from '~/composables/usePanes'

const props = defineProps<{ side: Edge }>()
const { layout, start, reset } = usePanes()
</script>

<template>
  <div
    class="handle"
    :class="{ active: layout.dragging === props.side, row: props.side === 'result' }"
    title="Drag to resize, double-click to reset"
    @pointerdown="start($event, props.side)"
    @dblclick="reset(props.side)"
  />
</template>

<style scoped>
.handle {
  position: relative;
  width: 5px;
  margin: 0 -2px;
  /* Positioned, because the handle overlaps its neighbours by its own margin
     and `z-index` says nothing about a static box: without this the pane on the
     later side of it takes the pointer over that overlap. */
  z-index: 5;
  cursor: col-resize;
  background: transparent;
  transition: background 0.12s;
}

/* The one edge that moves up and down rather than side to side. */
.handle.row {
  width: auto;
  height: 5px;
  margin: -2px 0;
  flex: none;
  cursor: row-resize;
}

.handle:hover,
.handle.active {
  background: var(--accent);
}
</style>
