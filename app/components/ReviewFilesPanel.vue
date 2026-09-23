<script setup lang="ts">
import { computed, ref } from 'vue'
import { Check, ChevronDown, ChevronUp, Copy, Eye, EyeOff } from 'lucide-vue-next'
import FileList from './FileList.vue'
import { useReview } from '~/composables/useReview'
import { useFileView } from '~/composables/useFileView'
import { useContextMenu } from '~/composables/useContextMenu'
import { copyText, useGit } from '~/composables/useGit'

/**
 * The review's files, in the panel the commit's files already live in.
 *
 * While a review is being read, that panel has nothing to say about the
 * working tree — so it says what the review changes instead: every file
 * across every commit of it, with what has been said about each and how much
 * of it has been read. Clicking one opens it in the files page; the menu
 * marks it read.
 */
const review = useReview()
const store = review.store
const view = useFileView()
const menu = useContextMenu()
const git = useGit()

const stats = computed(() => ({
  files: store.files.length,
  additions: store.files.reduce((sum, file) => sum + file.additions, 0),
  deletions: store.files.reduce((sum, file) => sum + file.deletions, 0)
}))

/** What each file carries, for the badges the rows wear. */
const comments = computed(() => {
  const out: Record<string, number> = {}
  for (const file of store.files) {
    const count = review.countFor(file.path)
    if (count) out[file.path] = count
  }
  return out
})

const viewed = computed(() => [...store.viewed])

/**
 * Which of the review's files the list is showing.
 *
 * Two hundred files is a list nobody reads twice: "what have I not read" and
 * "what has somebody asked about" are the two questions that come up, so they
 * are one click each rather than a scroll.
 */
const filter = ref<'all' | 'unread' | 'talk'>('all')

const shown = computed(() => {
  const files = store.files
  if (filter.value === 'unread') return files.filter((file) => !store.viewed.has(file.path))
  // Settled threads are answered; what is left is what somebody is waiting on.
  if (filter.value === 'talk') return files.filter((file) => review.openFor(file.path) > 0)
  return files
})

const FILTERS = computed(() => [
  { id: 'all' as const, label: 'All', count: store.files.length },
  {
    id: 'unread' as const,
    label: 'Unread',
    count: store.files.length - review.viewedCount.value
  },
  {
    id: 'talk' as const,
    label: 'Open',
    count: store.files.filter((file) => review.openFor(file.path) > 0).length
  }
])

/** Opens the file in the files page, which is what a click here means. */
function pick(path: string) {
  store.tab = 'files'
  store.selectedPath = path
}

/** Where the open file sits in the list as it is filtered right now. */
const at = computed(() => shown.value.findIndex((file) => file.path === store.selectedPath))

/** Walks the list as drawn, so the arrows follow the filter rather than fight it. */
function step(delta: number) {
  const file = shown.value[at.value + delta]
  if (file) pick(file.path)
}

function fileMenu(event: MouseEvent, entry: { path: string }) {
  const read = store.viewed.has(entry.path)
  menu.show(
    event,
    [
      {
        label: read ? 'Mark as not viewed' : 'Mark as viewed',
        icon: read ? EyeOff : Eye,
        action: () => review.toggleViewed(entry.path)
      },
      { label: 'Copy path', icon: Copy, action: () => void copyText(entry.path, 'Path') },
      { label: git.revealLabel, action: () => void git.reveal(entry.path) }
    ],
    entry.path
  )
}

/** A folder's menu is the folder's whole contents, read in one gesture. */
function dirMenu(event: MouseEvent, path: string) {
  const inside = store.files.filter((file) => file.path.startsWith(`${path}/`))
  const all = inside.every((file) => store.viewed.has(file.path))
  menu.show(
    event,
    [
      {
        label: all ? 'Mark all inside as not viewed' : 'Mark all inside as viewed',
        icon: all ? EyeOff : Eye,
        hint: `${inside.length} ${inside.length === 1 ? 'file' : 'files'}`,
        action: () => {
          for (const file of inside) {
            if (store.viewed.has(file.path) === all) review.toggleViewed(file.path)
          }
        }
      }
    ],
    path
  )
}
</script>

<template>
  <aside class="panel">
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

    <div class="filters">
      <button
        v-for="one in FILTERS"
        :key="one.id"
        class="chip"
        :class="{ on: filter === one.id }"
        :data-testid="`filter-${one.id}`"
        @click="filter = one.id"
      >
        {{ one.label }}
        <span class="n">{{ one.count }}</span>
      </button>

      <span class="grow" />

      <!-- The list walked from where it is drawn, for a review whose files are
           read one after another rather than picked out of a tree. -->
      <button
        class="walk"
        title="The file above (↑)"
        data-testid="panel-prev"
        :disabled="at <= 0"
        @click="step(-1)"
      >
        <ChevronUp :size="13" />
      </button>
      <button
        class="walk"
        title="The file below (↓)"
        data-testid="panel-next"
        :disabled="at < 0 || at >= shown.length - 1"
        @click="step(1)"
      >
        <ChevronDown :size="13" />
      </button>
    </div>

    <FileList
      :files="
        shown.map((file) => ({
          path: file.path,
          kind: file.status,
          additions: file.additions,
          deletions: file.deletions
        }))
      "
      :selected="store.selectedPath"
      :comments="comments"
      :viewed="viewed"
      :empty="filter === 'all' ? 'No files in this review.' : 'Nothing left under that filter.'"
      @select="pick"
      @menu="fileMenu"
      @dirmenu="dirMenu"
    />

    <footer v-if="store.files.length" class="progress">
      <div class="bar" :title="`${review.viewedCount.value} of ${store.files.length} read`">
        <span
          class="fill"
          :style="{ width: `${(review.viewedCount.value / store.files.length) * 100}%` }"
        />
      </div>
      <span class="faint count">
        <Check v-if="store.files.every((file) => store.viewed.has(file.path))" :size="11" />
        {{ review.viewedCount.value }} of {{ store.files.length }} viewed
      </span>
    </footer>
  </aside>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

/* The same head the commit's file list wears, so the two panels read as one
   place with different answers rather than as two designs. */
.files-head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px 6px;
  font-size: 12px;
  color: var(--text-dim);
}

/* Path or tree, as a small segmented pill. */
.toggle {
  margin-left: auto;
  display: flex;
  gap: 1px;
  padding: 2px;
  border-radius: var(--radius-pill);
  background: var(--bg-raised);
}

.seg {
  padding: 1px 9px;
  border-radius: var(--radius-pill);
  font-size: 11px;
  color: var(--text-dim);
}

.seg:hover {
  color: var(--text);
}

.seg.on {
  background: var(--bg);
  box-shadow: var(--shadow-card);
  color: var(--text);
  font-weight: 600;
}

.plus {
  color: var(--success);
  font-size: 11.5px;
  font-weight: 600;
}

.minus {
  color: var(--danger);
  font-size: 11.5px;
  font-weight: 600;
}

/* Which files are worth looking at right now, one click each. */
.filters {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 12px 8px;
  border-bottom: 1px solid var(--line-soft);
}

.chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 10px;
  border-radius: var(--radius-pill);
  font-size: 11.5px;
  font-weight: 500;
  color: var(--text-dim);
  transition:
    background 0.12s,
    color 0.12s;
}

.chip:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.chip.on {
  color: var(--primary-fg);
  background: var(--primary);
}

.chip .n {
  font-variant-numeric: tabular-nums;
  opacity: 0.75;
}

.filters .grow {
  flex: 1;
}

.walk {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: var(--radius-pill);
  color: var(--text-dim);
}

.walk:hover:not(:disabled) {
  color: var(--text);
  background: var(--bg-hover);
}

.walk:disabled {
  opacity: 0.3;
}

/* How far the reading has gone, as a line rather than only a number: a
   two-hundred-file review is a progress bar whether or not it is drawn. */
.progress {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 14px 12px;
  border-top: 1px solid var(--line-soft);
  font-size: 11.5px;
}

.bar {
  height: 5px;
  border-radius: var(--radius-pill);
  background: var(--bg-raised);
  overflow: hidden;
}

.fill {
  display: block;
  height: 100%;
  border-radius: var(--radius-pill);
  background: var(--success);
  transition: width 0.15s ease-out;
}

.count {
  display: flex;
  align-items: center;
  gap: 5px;
}
</style>
