<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { Check, Copy, FolderOpen, Minus, Undo2, X } from 'lucide-vue-next'
import { copyText, useGit, type FileDiff } from '~/composables/useGit'
import { languageFor } from '~/composables/useHighlight'

const git = useGit()
const store = git.store

const diff = ref<FileDiff | null>(null)
const loading = ref(false)

const target = computed(() => store.viewer)
const language = computed(() => (target.value ? languageFor(target.value.path) : null))
const stats = computed(() => {
  const lines = (diff.value?.hunks ?? []).flatMap((hunk) => hunk.lines)
  return {
    additions: lines.filter((line) => line.origin === '+').length,
    deletions: lines.filter((line) => line.origin === '-').length
  }
})

async function load() {
  const current = target.value
  if (!current) return
  loading.value = true
  diff.value = current.commit
    ? await git.commitFileDiff(current.commit, current.path)
    : await git.workingFileDiff(current.path, current.side ?? 'unstaged')
  loading.value = false
}

function close() {
  store.viewer = null
}

/** Stage, unstage or discard one hunk, then reload so the view is honest. */
async function onHunk(index: number, action: 'stage' | 'unstage' | 'discard') {
  const current = target.value
  if (!current || current.commit) return
  await git.applyHunk(current.path, index, action)
  await load()
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') close()
}

watch(target, load, { deep: true })
// Staging changes which side a file lives on, so follow the status.
watch(() => store.status, load)

onMounted(() => {
  load()
  window.addEventListener('keydown', onKey)
})
onUnmounted(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <section v-if="target" class="viewer">
    <header class="bar">
      <span class="path mono truncate" :title="target.path">{{ target.path }}</span>
      <span v-if="language" class="pill">{{ language }}</span>
      <span v-if="target.commit" class="pill">{{ target.commit.slice(0, 7) }}</span>
      <span v-else class="pill">{{ target.side }}</span>
      <span class="plus">+{{ stats.additions }}</span>
      <span class="minus">−{{ stats.deletions }}</span>

      <span class="grow" />

      <!-- Discard left, stage right, matching the hunk buttons in the diff
           below: the destructive one is never where the hand already is. -->
      <template v-if="!target.commit">
        <button
          class="btn danger"
          :disabled="store.busy"
          title="Throw away the changes to this file"
          @click="git.discard([target.path])"
        >
          <Undo2 :size="14" /> Discard
        </button>
        <button
          v-if="target.side === 'unstaged'"
          class="btn"
          :disabled="store.busy"
          @click="git.stage([target.path])"
        >
          <Check :size="14" /> Stage file
        </button>
        <button v-else class="btn" :disabled="store.busy" @click="git.unstage([target.path])">
          <Minus :size="14" /> Unstage file
        </button>
      </template>

      <button class="btn" title="Copy path" @click="copyText(target.path, 'Path')">
        <Copy :size="14" />
      </button>
      <button class="btn" :title="git.revealLabel" @click="git.reveal(target.path)">
        <FolderOpen :size="14" />
      </button>
      <button class="btn" title="Close (Esc)" @click="close">
        <X :size="16" />
      </button>
    </header>

    <div class="body">
      <DiffView
        :diff="diff"
        :loading="loading"
        :side="target.commit ? null : (target.side ?? 'unstaged')"
        :busy="store.busy"
        @hunk="onHunk"
      />
    </div>
  </section>
</template>

<style scoped>
.viewer {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  min-width: 0;
  background: var(--bg);
}

.bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
}

.path {
  max-width: 46%;
  color: var(--text);
}

.grow {
  flex: 1;
}

.plus {
  color: var(--green);
  font-size: 11.5px;
}

.minus {
  color: var(--red);
  font-size: 11.5px;
}

.danger {
  color: #ef8d9c;
}

.body {
  overflow: auto;
}
</style>
