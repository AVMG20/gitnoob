<script setup lang="ts">
import { computed } from 'vue'
import { Redo2, TriangleAlert, Undo2 } from 'lucide-vue-next'
import { relativeTime, useGit } from '~/composables/useGit'

const emit = defineEmits<{ close: [] }>()
const git = useGit()
const store = git.store

const undo = computed(() => store.history.undo)
const redo = computed(() => store.history.redo)
</script>

<template>
  <div class="scrim" @click="emit('close')">
    <div class="menu" @click.stop>
      <div class="section-title">Undo</div>
      <p v-if="!undo.length" class="empty faint">Nothing to undo yet.</p>
      <button
        v-for="(entry, index) in undo"
        :key="entry.id"
        class="item"
        :disabled="index > 0 || store.busy"
        :title="index > 0 ? 'Undo the step above first' : `Undo ${entry.label}`"
        @click="((emit('close')), git.undo())"
      >
        <Undo2 :size="14" class="glyph" />
        <span class="grow truncate">{{ entry.label }}</span>
        <TriangleAlert v-if="entry.destructive" :size="13" class="warn" title="Touches your working tree" />
        <span class="faint when">{{ relativeTime(entry.at) }}</span>
      </button>

      <template v-if="redo.length">
        <div class="divider" />
        <div class="section-title">Redo</div>
        <button
          v-for="(entry, index) in redo"
          :key="entry.id"
          class="item"
          :disabled="index > 0 || store.busy"
          @click="((emit('close')), git.redo())"
        >
          <Redo2 :size="14" class="glyph" />
          <span class="grow truncate">{{ entry.label }}</span>
          <span class="faint when">{{ relativeTime(entry.at) }}</span>
        </button>
      </template>

      <p class="note faint">
        Undo moves the branch pointer; it never throws away a commit object. Steps marked with a
        warning also change your working tree.
      </p>
    </div>
  </div>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 45;
}

.menu {
  position: absolute;
  /* Under the history button: the tab strip and the toolbar are 38 and 52
     pixels, and the button sits left of settings and the profile. */
  right: 150px;
  top: 84px;
  width: 380px;
  max-height: 60vh;
  overflow-y: auto;
  padding: 4px;
  background: var(--bg);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-pop);
  animation: pop-in 0.08s ease-out;
}

@keyframes pop-in {
  from {
    opacity: 0;
    transform: translateY(-3px);
  }
}

.item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 26px;
  padding: 3px 8px;
  border-radius: var(--radius-sm);
  text-align: left;
  font-size: 12.5px;
}

/* Filled in the accent under the pointer, as a native menu marks its row. */
.item:hover:not(:disabled) {
  background: var(--accent);
  color: var(--on-accent);
}

.item:hover:not(:disabled) .faint,
.item:hover:not(:disabled) .warn {
  color: var(--on-accent);
}

.item:disabled {
  opacity: 0.45;
}

.glyph {
  flex: none;
  opacity: 0.7;
}

.grow {
  flex: 1;
  min-width: 0;
}

.warn {
  color: var(--amber);
  flex: none;
}

.when {
  font-size: 10.5px;
  white-space: nowrap;
}

.empty,
.note {
  padding: 4px 8px 8px;
  font-size: 11.5px;
  margin: 0;
}

.note {
  border-top: 1px solid var(--line-soft);
  margin-top: 5px;
  padding-top: 8px;
}

.divider {
  height: 1px;
  margin: 4px 6px;
  background: var(--line-soft);
}
</style>
