<script setup lang="ts">
import { computed, watch } from 'vue'
import { Copy, FileText, GitCommitHorizontal, Hash } from 'lucide-vue-next'
import { copyText, fullTime, useGit } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useFileView } from '~/composables/useFileView'

const git = useGit()
const store = git.store
const menu = useContextMenu()
const view = useFileView()

const openFile = computed(() =>
  store.viewer?.commit === store.detail?.oid ? store.viewer?.path : null
)

const detail = computed(() => store.detail)
const stats = computed(() => {
  const files = detail.value?.files ?? []
  return {
    files: files.length,
    additions: files.reduce((sum, f) => sum + f.additions, 0),
    deletions: files.reduce((sum, f) => sum + f.deletions, 0)
  }
})

/** Opens the file across the graph area rather than inline. */
function show(path: string) {
  if (!detail.value) return
  store.viewer =
    store.viewer?.path === path && store.viewer?.commit === detail.value.oid
      ? null
      : { path, commit: detail.value.oid }
}

// Moving to another commit closes whatever file was open from the last one.
watch(
  () => store.detail?.oid,
  () => {
    if (store.viewer?.commit) store.viewer = null
  }
)

function fileMenu(event: MouseEvent, path: string) {
  menu.show(
    event,
    [
      { label: 'Copy path', icon: Copy, action: () => copyText(path, 'Path') },
      { label: 'Reveal in Finder', icon: FileText, action: () => git.reveal(path) }
    ],
    path
  )
}
</script>

<template>
  <div class="details">
    <p v-if="!detail" class="empty dim">Select a commit to see what it changed.</p>

    <template v-else>
      <div class="head">
        <button class="oid mono faint" title="Copy hash" @click="copyText(detail.oid, 'Hash')">
          <Hash :size="12" /> {{ detail.short }}
          <Copy :size="11" class="copy" />
        </button>
        <h3>{{ detail.summary }}</h3>
        <pre v-if="detail.body" class="body">{{ detail.body }}</pre>

        <div class="who">
          <div>
            <span class="dim">{{ detail.author }}</span>
            <span class="faint"> &lt;{{ detail.email }}&gt;</span>
          </div>
          <div class="faint">{{ fullTime(detail.time) }}</div>
          <div v-if="detail.committer !== detail.author" class="faint">
            committed by {{ detail.committer }}
          </div>
        </div>

        <div class="parents">
          <GitCommitHorizontal :size="12" class="faint" />
          <button
            v-for="parent in detail.parents"
            :key="parent"
            class="parent mono"
            @click="git.select(parent)"
          >
            {{ parent.slice(0, 7) }}
          </button>
          <span v-if="!detail.parents.length" class="faint">root commit</span>
        </div>
      </div>

      <div class="files-head">
        <span>{{ stats.files }} {{ stats.files === 1 ? 'file' : 'files' }}</span>
        <span class="plus">+{{ stats.additions }}</span>
        <span class="minus">−{{ stats.deletions }}</span>
        <span class="toggle">
          <button
            class="seg"
            :class="{ on: view.state.mode === 'path' }"
            @click="view.state.mode = 'path'"
          >
            Path
          </button>
          <button
            class="seg"
            :class="{ on: view.state.mode === 'tree' }"
            @click="view.state.mode = 'tree'"
          >
            Tree
          </button>
        </span>
      </div>

      <FileList
        :files="
          detail.files.map((file) => ({
            path: file.path,
            kind: file.status,
            additions: file.additions,
            deletions: file.deletions
          }))
        "
        :selected="openFile"
        empty="No files in this commit."
        @select="show"
        @menu="(event, entry) => fileMenu(event, entry.path)"
      />
    </template>
  </div>
</template>

<style scoped>
.details {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}

.empty {
  padding: 18px 14px;
  font-size: 12px;
}

.head {
  padding: 12px 14px;
  border-bottom: 1px solid var(--line);
}

.oid {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-bottom: 6px;
  padding: 2px 6px;
  border-radius: 4px;
}

.oid:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.copy {
  opacity: 0;
}

.oid:hover .copy {
  opacity: 0.7;
}

h3 {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  line-height: 1.35;
}

.body {
  margin: 8px 0 0;
  font-family: var(--font);
  font-size: 12px;
  color: var(--text-dim);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 160px;
  overflow: auto;
}

.who {
  margin-top: 11px;
  font-size: 12px;
  line-height: 1.5;
}

.parents {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 9px;
}

.parent {
  font-size: 11px;
  color: var(--text-faint);
  padding: 1px 5px;
  border-radius: 4px;
}

.parent:hover {
  background: var(--bg-hover);
  color: var(--accent);
}

.files-head {
  display: flex;
  gap: 10px;
  padding: 6px 14px;
  font-size: 11px;
  color: var(--text-faint);
  border-bottom: 1px solid var(--line-soft);
}

.toggle {
  margin-left: auto;
  display: flex;
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
  font-size: 11px;
}

.minus {
  color: var(--red);
  font-size: 11px;
}

</style>
