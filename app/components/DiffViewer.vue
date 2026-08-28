<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from 'vue'
import { ArrowDownToLine, Check, Copy, FileBox, FolderOpen, History, Minus, Undo2, Users, X } from 'lucide-vue-next'
import {
  copyText,
  relativeTime,
  useGit,
  type BlameRun,
  type FileDiff
} from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { labelFor } from '~/composables/useHighlight'
import { diffMode, type DiffMode } from '~/composables/useDiffMode'
import type { PickedLines } from '~/components/DiffView.vue'
import { humanSize, readPointer } from '~/composables/useLfs'
import { stepFile, useFileView, walkOrder, type FileStep } from '~/composables/useFileView'
import {
  CODE_ROW,
  diffRows,
  fileMarks,
  firstChangedLine,
  markedLines,
  patchMarks
} from '~/composables/useCode'

/** The order Tab walks the views in. */
const MODES: DiffMode[] = ['diff', 'file']

const git = useGit()
const menu = useContextMenu()
const store = git.store
const view = useFileView()

const diff = ref<FileDiff | null>(null)
const loading = ref(false)
/** The file itself, for the whole-file view. Only read when that view is on. */
const text = ref<string | null>(null)
const textError = ref<string | null>(null)

/** The scrolling half of the viewer, which the ruler measures. */
const body = ref<HTMLElement | null>(null)

/**
 * Where the box is scrolled to, and how tall it is.
 *
 * Both views draw only the lines these two put on screen, so they are tracked
 * here — in the one element that actually scrolls — rather than each view
 * reaching for an ancestor it does not own. Coalesced to a frame: scroll events
 * arrive faster than frames do, and a second one in the same frame would only
 * throw away the rows the first is still drawing.
 */
const top = ref(0)
const boxHeight = ref(0)
/**
 * How far sideways it is scrolled, and how wide the box is.
 *
 * A hunk heading is as wide as the widest line in the file, so its buttons sat
 * at the end of that width — off the right of the window in any file with a
 * long line in it, reachable only by scrolling away from the code you were
 * about to stage. The heading is drawn the width of the box instead and moved
 * along with the scroll, which needs both of these.
 */
const left = ref(0)
const boxWidth = ref(0)
let queued = false

function onScroll() {
  if (queued) return
  queued = true
  requestAnimationFrame(() => {
    queued = false
    if (!body.value) return
    top.value = body.value.scrollTop
    left.value = body.value.scrollLeft
  })
}

let sizer: ResizeObserver | null = null

const target = computed(() => store.viewer)
const language = computed(() => (target.value ? labelFor(target.value.path) : null))
/**
 * Whether the file being read has been deleted.
 *
 * A deletion has no file on disk, so what both views show is the copy git
 * still has — and saying so is the difference between a page that explains
 * itself and one that looks like the app went wrong.
 */
const gone = computed(() => {
  const current = target.value
  if (!current || current.commit) return false
  const list =
    (current.side ?? 'unstaged') === 'staged' ? store.status?.staged : store.status?.unstaged
  return (list ?? []).some((entry) => entry.path === current.path && entry.kind === 'deleted')
})

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
  await loadBlame()
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
 *
 * Worked out from the diff rather than by looking for the first marked row in
 * the page, which was how it used to be done and no longer can be: the view
 * draws only the rows on screen, and the first change is the one row that is
 * reliably not among them yet.
 */
async function toFirstChange() {
  const box = body.value
  if (!box) return
  // A file just opened is read from wherever its change is, not from wherever
  // the last one happened to be left.
  box.scrollTop = 0
  top.value = 0
  if (diffMode.mode !== 'file') return
  const at = firstChangedLine(diff.value)
  if (at === null) return
  // The rows are placed by the model, so the view has to have been given its
  // height before there is anywhere to scroll to.
  await nextTick()
  // A few lines of what came before, so the change has somewhere to sit.
  box.scrollTop = Math.max(0, (at - 1) * CODE_ROW - 72)
  top.value = box.scrollTop
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

/**
 * Opens or closes the blame column.
 *
 * The column belongs to the file view, so pressing this from the patch takes
 * you there with it open rather than turning on something you cannot see.
 */
function toggleBlame() {
  if (diffMode.mode !== 'file') {
    diffMode.mode = 'file'
    diffMode.blame = true
    return
  }
  diffMode.blame = !diffMode.blame
}

/**
 * The file's own history, in the menu the rest of the app uses for lists.
 *
 * A commit here opens this file as it stood at that commit, which is the
 * question being asked — "what did this look like then" — rather than opening
 * the commit and hunting for the file in it. `--follow` means the list carries
 * on past a rename, and those are the entries you cannot find any other way.
 */
async function showHistory(event: MouseEvent) {
  const current = target.value
  if (!current) return
  const found = await git.fileHistory(current.path, HISTORY_SHOWN + 1)
  if (!found?.length) {
    git.note(`No commits touch ${current.path} yet`)
    return
  }
  const items = found.slice(0, HISTORY_SHOWN).map((one) => ({
    label: one.summary || one.short,
    hint: `${one.short} · ${relativeTime(one.time)}`,
    action: () => {
      store.viewer = { path: current.path, commit: one.oid }
    }
  }))
  if (found.length > HISTORY_SHOWN) {
    items.push({
      label: `…and older still`,
      hint: 'search the commit list',
      action: () => undefined
    })
  }
  menu.show(event, items, current.path)
}

/** How many commits the menu will hold before it stops being a menu. */
const HISTORY_SHOWN = 30

/**
 * The LFS pointer standing in for this file, when that is what is on disk.
 *
 * Three lines of metadata drawn as though they were the file is the whole
 * problem LFS causes a viewer, so when the text is a pointer the panel says
 * what the file is instead of showing what it is not.
 */
const pointer = computed(() => readPointer(text.value))

async function fetchFromLfs() {
  const current = target.value
  if (!current) return
  await git.lfsPull(current.path)
  await load(false)
}

// --- who touched what
//
// Read only while the blame column is on screen, and again when the file or the
// commit under it changes. It is a walk of the file's history, which is not a
// thing to do for a column nobody opened.
const blame = ref<BlameRun[]>([])
const blaming = ref(false)
const blameError = ref<string | null>(null)

async function loadBlame() {
  const current = target.value
  if (!current || diffMode.mode !== 'file' || !diffMode.blame) return
  blaming.value = true
  blameError.value = null
  try {
    const found = await git.blameFile(current.path, current.commit)
    // A slower answer for a file that is no longer open is not this file's.
    if (target.value?.path === current.path) blame.value = found
  } catch (error) {
    blame.value = []
    blameError.value = String(error)
  } finally {
    blaming.value = false
  }
}

watch(() => diffMode.mode, async () => {
  await loadText()
  await loadBlame()
  await toFirstChange()
})

// Turning the column on is the one thing that asks for a walk of the history,
// so it is what pays for it.
watch(() => diffMode.blame, () => loadBlame())

/**
 * Where the changes are, for the strip beside the scrollbar.
 *
 * Worked out from whichever model the view on screen is drawn from, so the two
 * always agree about where a change is — and so the strip still knows about
 * changes that are nowhere near the part of the file being looked at.
 */
const marks = computed(() => {
  if (diffMode.mode === 'file') return fileMarks(markedLines(text.value, diff.value?.hunks ?? []))
  const laid = diffRows(diff.value?.hunks ?? [])
  return patchMarks(laid.rows, laid.height)
})

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
async function onHunk(
  index: number,
  action: 'stage' | 'unstage' | 'discard',
  lines?: PickedLines
) {
  const current = target.value
  if (!current || current.commit) return
  await git.applyHunk(current.path, index, action, lines)
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
    // Round the three in order, backwards with shift: the same key that used
    // to swap two views now walks them, rather than stranding the third.
    const at = MODES.indexOf(diffMode.mode)
    const step = event.shiftKey ? -1 : 1
    diffMode.mode = MODES[(at + step + MODES.length) % MODES.length]!
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
  (status) => {
    // A commit stays open whatever the working tree does; a working file is
    // open because it had changes, and once nothing anywhere has any — the
    // lot discarded, stashed, or dealt with outside the window — the viewer is
    // left showing an empty page over the list you now want. Committing closes
    // it in the panel itself, whether or not it emptied the tree; this is the
    // other ways the changes can go.
    const nothingLeft =
      status && !status.staged.length && !status.unstaged.length && !status.conflicted.length
    if (target.value && !target.value.commit && nothingLeft) {
      close()
      return
    }
    load(false)
  }
)

onMounted(() => {
  load()
  window.addEventListener('keydown', onKey)
  const box = body.value
  if (!box) return
  box.addEventListener('scroll', onScroll, { passive: true })
  boxHeight.value = box.clientHeight
  boxWidth.value = box.clientWidth
  sizer = new ResizeObserver(() => {
    boxHeight.value = box.clientHeight
    boxWidth.value = box.clientWidth
  })
  sizer.observe(box)
})

onBeforeUnmount(() => {
  body.value?.removeEventListener('scroll', onScroll)
  sizer?.disconnect()
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

      <!-- Always here, whichever view is on screen. It used to appear along
           with the file view and shove everything beside it sideways, so
           switching views moved the buttons under the pointer. Blame is still
           a column of the file, so asking for it from the patch opens that
           view rather than doing nothing. -->
      <button
        class="btn"
        :class="{ on: diffMode.mode === 'file' && diffMode.blame }"
        :title="
          diffMode.mode !== 'file'
            ? 'Show who last touched each line — opens the file view'
            : diffMode.blame
              ? 'Hide who last touched each line'
              : 'Show who last touched each line'
        "
        @click="toggleBlame"
      >
        <Users :size="14" />
      </button>

      <button class="btn" title="Every commit that touched this file" @click="showHistory">
        <History :size="14" />
      </button>
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
        <!-- What is on disk is the pointer, not the file. Nothing below can
             say anything useful about it, so this stands in their place. -->
        <div v-if="pointer" class="lfs">
          <FileBox :size="34" class="glyph" />
          <h3>{{ target?.path.split('/').pop() }}</h3>
          <p class="dim">
            Stored with Git LFS — {{ humanSize(pointer.size) }}. What is in the folder is the
            pointer to it, not the file itself.
          </p>
          <p class="faint mono oid">{{ pointer.oid }}</p>
          <button
            v-if="store.lfs?.installed !== false"
            class="btn btn-primary"
            :disabled="store.busy"
            @click="fetchFromLfs"
          >
            <ArrowDownToLine :size="14" /> Fetch it
          </button>
          <p v-else class="faint">
            <span class="mono">git-lfs</span> is not installed on this machine, so nothing here
            can fetch it.
          </p>
        </div>

        <FileView
          v-else-if="diffMode.mode === 'file'"
          :diff="diff"
          :gone="gone"
          :text="text"
          :loading="loading"
          :error="textError"
          :top="top"
          :view="boxHeight"
          :runs="blame"
          :blame="diffMode.blame"
          :blame-loading="blaming"
          :blame-error="blameError"
          @toggle-blame="diffMode.blame = !diffMode.blame"
        />
        <DiffView
          v-else
          :diff="diff"
          :text="text"
          :loading="loading"
          :side="target.commit ? null : (target.side ?? 'unstaged')"
          :busy="store.busy"
          :top="top"
          :view="boxHeight"
          :left="left"
          :width="boxWidth"
          @hunk="onHunk"
        />
      </div>
      <ChangeRuler :container="body" :marks="marks" />
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

/* A toggle rather than an action: on, it holds the pressed look the segmented
   control uses, so the bar has one idea of what "this is on" looks like. */
.btn.on {
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

/* An LFS file that is not here: said in the middle of the pane, because there
   is nothing else the pane could be showing. */
.lfs {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 64px 24px;
  text-align: center;
}

.lfs .glyph {
  color: var(--text-faint);
}

.lfs h3 {
  margin: 4px 0 0;
  font-size: 14px;
}

.lfs p {
  margin: 0;
  max-width: 460px;
  font-size: 12px;
}

.lfs .oid {
  font-size: 11px;
  word-break: break-all;
}

.lfs .btn-primary {
  margin-top: 8px;
}
</style>