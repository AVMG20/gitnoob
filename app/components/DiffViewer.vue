<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { Check, Copy, FolderOpen, Minus, Undo2, X } from 'lucide-vue-next'
import { copyText, useGit, type FileDiff } from '~/composables/useGit'
import { labelFor } from '~/composables/useHighlight'
import { diffMode } from '~/composables/useDiffMode'

const git = useGit()
const store = git.store

const diff = ref<FileDiff | null>(null)
const loading = ref(false)
/** The file itself, for the whole-file view. Only read when that view is on. */
const text = ref<string | null>(null)
const textError = ref<string | null>(null)

const target = computed(() => store.viewer)
const language = computed(() => (target.value ? labelFor(target.value.path) : null))
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
  await loadText()
  loading.value = false
}

/**
 * Reads the file itself, which only the whole-file view needs.
 *
 * Deferred until that view is asked for: the diff view already has everything
 * it shows, and reading a file to throw it away is a cost on every click.
 */
async function loadText() {
  const current = target.value
  if (!current || diffMode.mode !== 'file') return
  textError.value = null
  try {
    text.value = await git.fileText(current.path, current.commit, current.side ?? 'unstaged')
  } catch (error) {
    text.value = null
    textError.value = String(error)
  }
}

watch(() => diffMode.mode, loadText)

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

      <!-- Two ways to read the same change: the patch, or the file with the
           change marked in it. Which one you prefer is remembered. -->
      <span class="modes">
        <button
          class="seg"
          :class="{ on: diffMode.mode === 'diff' }"
          title="The changed lines, in hunks"
          @click="diffMode.mode = 'diff'"
        >
          Diff
        </button>
        <button
          class="seg"
          :class="{ on: diffMode.mode === 'file' }"
          title="The whole file, with the changes marked down the side"
          @click="diffMode.mode = 'file'"
        >
          File
        </button>
      </span>

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
      <FileView
        v-if="diffMode.mode === 'file'"
        :diff="diff"
        :text="text"
        :loading="loading"
        :error="textError"
      />
      <DiffView
        v-else
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

/* The same segmented control the file panel uses for path and tree. */
.modes {
  display: flex;
  flex: none;
  border: 1px solid var(--line);
  border-radius: 5px;
  overflow: hidden;
}

.seg {
  padding: 1px 7px;
  font-size: 10.5px;
  color: var(--text-faint);
}

.seg:hover {
  color: var(--text);
}

.seg.on {
  background: var(--bg-active);
  color: var(--text);
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
