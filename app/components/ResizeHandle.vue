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
/* A column handle is the gutter between two cards, and shows itself as a short
   rounded bar in the middle of it when the pointer finds it. */
.handle {
  position: relative;
  width: var(--gutter);
  /* Positioned, because `z-index` says nothing about a static box, and a row
     handle overlaps its neighbours by its own margin: without this the pane on
     the later side of it takes the pointer over that overlap. */
  z-index: 5;
  cursor: col-resize;
  background: transparent;
}

.handle::after {
  content: '';
  position: absolute;
  left: 50%;
  top: 50%;
  width: 3px;
  height: 36px;
  border-radius: var(--radius-pill);
  background: var(--text-faint);
  opacity: 0;
  transform: translate(-50%, -50%);
  transition:
    opacity 0.15s,
    height 0.15s,
    background 0.15s;
}

/* The one edge that moves up and down rather than side to side. It lives
   inside a card rather than between two, so it keeps a thin footprint and
   borrows the neighbours' space. */
.handle.row {
  width: auto;
  height: 7px;
  margin: -3px 0;
  flex: none;
  cursor: row-resize;
}

.handle.row::after {
  width: 36px;
  height: 3px;
}

.handle:hover::after,
.handle.active::after {
  opacity: 0.6;
}

.handle:not(.row):hover::after,
.handle:not(.row).active::after {
  height: 56px;
}

.handle.active::after {
  background: var(--accent);
  opacity: 1;
}
</style>
