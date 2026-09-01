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
.overlay {
  position: fixed;
  inset: 0;
  z-index: 55;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  background: var(--bg);
}

.bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
}

.warn {
  color: var(--amber);
}

.grow {
  flex: 1;
}

.icon {
  padding: 4px 6px;
}
</style>
