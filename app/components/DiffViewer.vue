<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { Check, Copy, FolderOpen, Minus, Undo2, X } from 'lucide-vue-next'
import { copyText, useGit, type FileDiff } from '~/composables/useGit'
import { labelFor } from '~/composables/useHighlight'
import { diffMode } from '~/composables/useDiffMode'
import { stepFile, useFileView, walkOrder, type FileStep } from '~/composables/useFileView'

const git = useGit()
const store = git.store
const view = useFileView()

const diff = ref<FileDiff | null>(null)
const loading = ref(false)
/** The file itself, for the whole-file view. Only read when that view is on. */
const text = ref<string | null>(null)
const textError = ref<string | null>(null)

/** The scrolling half of the viewer, which the ruler measures. */
const body = ref<HTMLElement | null>(null)

const target = computed(() => store.viewer)
const language = computed(() => (target.value ? labelFor(target.value.path) : null))
const stats = computed(() => {
  const lines = (diff.value?.hunks ?? []).flatMap((hunk) => hunk.lines)
  return {
    additions: lines.filter((line) => line.origin === '+').length,
    deletions: lines.filter((line) => line.origin === '-').length
  }
})

/**
 * Reads the file and its diff.
 *
 * `settle` says whether the view may be taken down while it reads. Opening a
 * file has nothing to show yet and says so; a reload behind an open file must
 * not, because it is triggered by the filesystem watcher — a build writing to
 * the work tree replaces `store.status`, which lands here — and blanking the
 * page for "Loading file…" every time anything on disk moved is what made the
 * file flicker while it was being read.
 */
async function load(settle = true) {
  const current = target.value
  if (!current) return
  if (settle) loading.value = true
  const fresh = current.commit
    ? await git.commitFileDiff(current.commit, current.path)
    : await git.workingFileDiff(current.path, current.side ?? 'unstaged')
  // Another file was opened while this one was being read; that load owns the
  // view now.
  if (target.value !== current) return
  diff.value = fresh
  await loadText()
  loading.value = false
  if (settle) await toFirstChange()
}

/**
 * Puts the first change on screen when the whole file is shown.
 *
 * A file is opened from a list of what changed, so the top of the file is
 * almost never what is being looked for — and in a long file the change can be
 * hundreds of lines down. The diff view needs none of this: it has nothing in
 * it but the changes.
 */
async function toFirstChange() {
  if (diffMode.mode !== 'file') return
  await nextTick()
  const box = body.value
  if (!box) return
  const first = box.querySelector('.line.added, .line.changed, .gone')
  if (!first) return
  const origin = box.getBoundingClientRect().top - box.scrollTop
  // A few lines of what came before, so the change has somewhere to sit.
  box.scrollTop = Math.max(0, first.getBoundingClientRect().top - origin - 72)
}

/**
 * Reads the file itself.
 *
 * Both views want it now. The whole-file view is made of it, and the diff view
 * colours from it: highlighting a patch line by line cannot see anything that
 * spans lines, and in a `.vue` or `.html` file — painted with the xml grammar,
 * which hands the inside of a `<script>` block to javascript — a lone line out
 * of that block has no tags in it and comes out with no colour at all. Reading
 * the file is one call against a file already on disk, and the diff view was
 * the one place that could not tell you what it was looking at.
 */
async function loadText() {
  const current = target.value
  if (!current) return
  textError.value = null
  try {
    text.value = await git.fileText(current.path, current.commit, current.side ?? 'unstaged')
  } catch (error) {
    text.value = null
    textError.value = String(error)
  }
}

watch(() => diffMode.mode, async () => {
  await loadText()
  await toFirstChange()
})

/** What the ruler is looking at; a change here means measuring again. */
const shown = computed(
  () => `${target.value?.path ?? ''}:${target.value?.commit ?? target.value?.side ?? ''}` +
    `:${diffMode.mode}:${loading.value}`
)

function close() {
  store.viewer = null
}

/**
 * The files the arrows walk: the commit's own when a commit is open, and the
 * working tree's two lists otherwise — unstaged first, which is the order the
 * panel stacks them in.
 */
const order = computed<FileStep[]>(() =>
  target.value?.commit
    ? walkOrder(
        [
          {
            files: (store.detail?.files ?? []).map((file) => ({
              path: file.path,
              kind: file.status
            }))
          }
        ],
        view.state.mode,
        view.state.collapsed
      )
    : walkOrder(
        [
          { files: store.status?.unstaged ?? [], side: 'unstaged' },
          { files: store.status?.staged ?? [], side: 'staged' }
        ],
        view.state.mode,
        view.state.collapsed
      )
)

/** Opens the file `by` steps along, leaving the viewer where it is if there is none. */
function move(by: number) {
  const current = target.value
  if (!current) return
  const from: FileStep = current.commit
    ? { path: current.path }
    : { path: current.path, side: current.side ?? 'unstaged' }
  const next = stepFile(order.value, from, by)
  if (!next) return
  store.viewer = current.commit
    ? { path: next.path, commit: current.commit }
    : { path: next.path, side: next.side }
}

/** Stage, unstage or discard one hunk, then reload so the view is honest. */
async function onHunk(index: number, action: 'stage' | 'unstage' | 'discard') {
  const current = target.value
  if (!current || current.commit) return
  await git.applyHunk(current.path, index, action)
  await load()
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    close()
    return
  }
  if (typing(event) || covered() || event.altKey || event.ctrlKey || event.metaKey) return
  // Tab flips between the patch and the file, which is the one thing anyone
  // does twice while reading a change. Left alone wherever it still means
  // "next field", and wherever a modifier makes it mean something else.
  if (event.key === 'Tab') {
    event.preventDefault()
    diffMode.mode = diffMode.mode === 'file' ? 'diff' : 'file'
    return
  }
  // The same two keys the commit list uses, and free while the viewer is open:
  // it stands where the list would be, so the list is not mounted to want them.
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    move(1)
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    move(-1)
  }
}

/**
 * True while something sits on top of the viewer.
 *
 * A dialog, a menu and a picker draw their own scrim, and the conflict resolver
 * its own overlay; whichever it is, the keys are theirs.
 */
function covered() {
  return !!document.querySelector('.scrim, .overlay')
}

/** True when the keystroke belongs to whatever is being written in. */
function typing(event: KeyboardEvent) {
  const element = event.target as HTMLElement | null
  if (!element) return false
  return (
    element.isContentEditable ||
    ['INPUT', 'TEXTAREA', 'SELECT'].includes(element.tagName)
  )
}

watch(target, () => load(), { deep: true })
// Staging changes which side a file lives on, so follow the status — quietly,
// and only when what is on screen could actually have changed. The watcher
// hands out a fresh status object for any write anywhere in the work tree, and
// re-reading the file each time is cheap; throwing the reader's place away is
// not.
watch(
  () => store.status,
  () => load(false)
)

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
          title="The changed lines, in hunks (Tab)"
          @click="diffMode.mode = 'diff'"
        >
          Diff
        </button>
        <button
          class="seg"
          :class="{ on: diffMode.mode === 'file' }"
          title="The whole file, with the changes marked down the side (Tab)"
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

    <div class="pane">
      <div ref="body" class="body">
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
          :text="text"
          :loading="loading"
          :side="target.commit ? null : (target.side ?? 'unstaged')"
          :busy="store.busy"
          @hunk="onHunk"
        />
      </div>
      <ChangeRuler :container="body" :version="shown" />
    </div>
  </section>
</template>

<style scoped>
.viewer {
  display: grid;
  /* The column is stated rather than left implicit. An `auto` column is sized
     to its content, so one very long line of a diff widens the whole column
     past the window instead of scrolling inside it — the file view then paints
     over the panel beside it and the window layout comes apart. */
  grid-template-columns: minmax(0, 1fr);
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
  color: var(--red-soft);
}

.pane {
  display: flex;
  min-width: 0;
  min-height: 0;
}

.body {
  flex: 1;
  min-width: 0;
  overflow: auto;
}
</style>
