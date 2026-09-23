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
  right: 96px;
  top: 96px;
  width: 380px;
  max-height: 60vh;
  overflow-y: auto;
  padding: 5px;
  background: var(--bg);
  border-radius: var(--radius);
  box-shadow: var(--shadow-pop);
  animation: pop-in 0.12s ease-out;
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
  gap: 9px;
  width: 100%;
  min-height: 30px;
  padding: 5px 10px;
  border-radius: var(--radius-sm);
  text-align: left;
  font-size: 13px;
}

.item:hover:not(:disabled) {
  background: var(--bg-hover);
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
  padding: 4px 10px 8px;
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
  margin: 5px 8px;
  background: var(--line-soft);
}
</style>
