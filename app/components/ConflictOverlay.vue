<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'
import { TriangleAlert, X } from 'lucide-vue-next'
import { useGit } from '~/composables/useGit'

const git = useGit()
const store = git.store

const remaining = computed(() => store.status?.conflicted.length ?? 0)

function close() {
  store.resolving = null
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') close()
}

onMounted(() => window.addEventListener('keydown', onKey))
onUnmounted(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <div class="overlay">
    <header class="bar">
      <TriangleAlert :size="15" class="warn" />
      <strong>Resolve conflicts</strong>
      <span class="faint">
        {{ remaining }} {{ remaining === 1 ? 'file' : 'files' }} left
      </span>
      <span class="grow" />
      <button
        v-if="store.progress?.restoring"
        class="btn"
        :disabled="store.busy"
        title="Put the working tree back, undo the step that moved it, and restore your changes"
        @click="git.undoRestore()"
      >
        Put it back
      </button>
      <button
        v-else-if="store.progress?.rebasing"
        class="btn"
        :disabled="store.busy"
        @click="git.abortRebase()"
      >
        Abort rebase
      </button>
      <button
        v-else-if="store.progress?.merging"
        class="btn"
        :disabled="store.busy"
        @click="git.abortMerge()"
      >
        Abort merge
      </button>
      <button class="btn icon" title="Close (Esc)" @click="close">
        <X :size="16" />
      </button>
    </header>
    <ConflictView />
  </div>
</template>

<style scoped>
/*
 * The resolver takes the window, but as a sheet laid over it rather than a
 * page swapped in: the repository stays visible, blurred, round its edge, so
 * it is clear this is a step you finish and come back out of.
 */
.overlay {
  position: fixed;
  z-index: 55;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  background: var(--bg);
  /* The sheet is the element itself, inset from the window by a gutter; the
     shadow's spread paints the dimmed, blurred backdrop round it. */
  inset: calc(var(--gutter) * 2);
  border-radius: var(--radius-lg);
  box-shadow:
    var(--shadow-pop),
    0 0 0 100vmax var(--overlay);
  overflow: hidden;
}

/* The blur has to come from something behind the sheet, and the sheet's own
   shadow cannot carry a filter, so a fixed layer under it does. */
.overlay::before {
  content: '';
  position: fixed;
  inset: 0;
  z-index: -1;
  backdrop-filter: blur(3px);
  pointer-events: none;
}

.bar {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 52px;
  padding: 10px 12px 10px 18px;
  border-bottom: 1px solid var(--line-soft);
}

.bar strong {
  font-size: 15px;
  font-weight: 650;
  letter-spacing: -0.01em;
}

/* The count of what is left, as a soft pill beside the title. */
.bar .faint {
  padding: 1px 9px;
  border-radius: var(--radius-pill);
  background: var(--warning-bg);
  color: var(--warning-soft);
  font-size: 11.5px;
  font-weight: 600;
}

.warn {
  color: var(--amber);
}

.grow {
  flex: 1;
}

/* Aborting throws away the whole merge or rebase, so it is an outlined pill
   rather than a word that looks like every other button in the bar. */
.bar .btn:not(.icon) {
  border-radius: var(--radius-pill);
  padding: 5px 14px;
  color: var(--text);
  box-shadow: inset 0 0 0 1px var(--line);
}

.bar .btn:not(.icon):hover:not(:disabled) {
  color: var(--red-soft);
  background: var(--danger-bg);
  box-shadow: inset 0 0 0 1px var(--danger-line);
}

.icon {
  width: 32px;
  padding: 0;
  border-radius: var(--radius-pill);
}
</style>
