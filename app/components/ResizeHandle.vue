<script setup lang="ts">
import { isRow, usePanes, type Edge } from '~/composables/usePanes'

const props = defineProps<{ side: Edge }>()
const { layout, start, reset } = usePanes()
</script>

<template>
  <div
    class="handle"
    :class="{ active: layout.dragging === props.side, row: isRow(props.side) }"
    title="Drag to resize, double-click to reset"
    @pointerdown="start($event, props.side)"
    @dblclick="reset(props.side)"
  />
</template>

<style scoped>
/* A column handle is the hairline between two panes. It is one pixel wide in
   the layout and reaches three either side of itself for the pointer, and it
   turns the accent while it is being held or hovered. */
.handle {
  position: relative;
  width: 1px;
  /* Positioned, because `z-index` says nothing about a static box: without it
     the pane on the later side takes the pointer over the reach. */
  z-index: 5;
  cursor: col-resize;
  background: var(--line);
  transition: background 0.12s;
}

.handle::before {
  content: '';
  position: absolute;
  inset: 0 -3px;
}

/* The one edge that moves up and down rather than side to side. */
.handle.row {
  width: auto;
  height: 1px;
  flex: none;
  cursor: row-resize;
}

.handle.row::before {
  inset: -3px 0;
}

.handle:hover,
.handle.active {
  background: var(--accent);
}
</style>
