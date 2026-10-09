<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  ArrowDownToLine,
  CaseSensitive,
  Check,
  ChevronDown,
  ChevronUp,
  Copy,
  FileBox,
  FolderOpen,
  History,
  Minus,
  Search,
  Undo2,
  Users,
  X
} from 'lucide-vue-next'
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
import { findIn, type FindMarks } from '~/composables/useFind'
import { keyLabel, useShortcuts } from '~/composables/useShortcuts'

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
  if (settle) {
    await toFirstChange()
    // Another file is another set of matches; the search carries on in it from
    // wherever the file was opened at.
    findFrom(null)
  }
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
 * spans lines, and in a `.vue` file a lone line out of the `<script>` block is
 * template text as far as any grammar can tell, because the tag that made it
 * TypeScript is not in what was passed. Reading the file is one call against a
 * file already on disk, and the diff view was the one place that could not tell
 * you what it was looking at.
 *
 * One side is enough: the diff view rebuilds the other from this text and the
 * patch, rather than spending a second call on it.
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
  findFrom(null)
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

// --- finding text in the file
//
// The views draw only the rows on screen, so the search runs over the rows
// themselves and each view marks the matches in whatever it draws. It searches
// the view that is on screen: the whole file in the file view, the lines of the
// patch in the diff.
const finding = ref(false)
const query = ref('')
const matchCase = ref(false)
/** Which match the arrows are on, as an index into `hits`. */
const hit = ref(0)
const findBox = ref<HTMLInputElement | null>(null)
/** The row the current match was on, which a longer search starts from. */
let findRow: number | null = null

/** The rows the view on screen draws, with where each sits down the view. */
const searched = computed<{ text: string; top: number }[]>(() => {
  if (pointer.value) return []
  if (diffMode.mode === 'file') {
    if (diff.value?.binary || text.value === null) return []
    const source = text.value.split('\n')
    // The same last empty piece `markedLines` drops, so a row here is a row there.
    if (source.length && source[source.length - 1] === '') source.pop()
    return source.map((line, at) => ({ text: line, top: at * CODE_ROW }))
  }
  return diffRows(diff.value?.hunks ?? []).rows.map((row) => ({
    text: row.line?.content ?? '',
    top: row.top
  }))
})

const hits = computed(() =>
  finding.value ? findIn(searched.value.map((row) => row.text), query.value, matchCase.value) : []
)

const findMarks = computed<FindMarks | null>(() => {
  if (!finding.value || !query.value) return null
  const at = hits.value[hit.value]
  return { query: query.value, matchCase: matchCase.value, row: at?.row ?? null, nth: at?.nth ?? 0 }
})

/** Where the rows start inside the box: the notes above them push them down. */
function rowsTop(box: HTMLElement) {
  const rows = box.querySelector('.lines, .rows')
  if (!rows) return 0
  return rows.getBoundingClientRect().top - box.getBoundingClientRect().top + box.scrollTop
}

/**
 * Puts the match on `hit` at, or else after, `row` — the top of the view when
 * no row is given. Typing more of a word keeps you on the match you were on
 * rather than sending you back to the top of the file.
 */
function findFrom(row: number | null) {
  const list = hits.value
  if (!list.length) {
    hit.value = 0
    findRow = null
    return
  }
  let from = row
  if (from === null) {
    const box = body.value
    const scrolled = box ? box.scrollTop - rowsTop(box) : 0
    from = searched.value.findIndex((one) => one.top >= scrolled)
  }
  const at = list.findIndex((one) => one.row >= (from ?? 0))
  hit.value = at < 0 ? 0 : at
  findRow = list[hit.value]?.row ?? null
}

/**
 * Scrolls the current match into view.
 *
 * Only when it is not already comfortably on screen, the way an editor does it:
 * stepping through three matches on the same screenful should move the mark,
 * not the page. When it does scroll, the match lands in the middle, clear of
 * the find box and the pinned hunk heading at the top.
 */
async function reveal() {
  const box = body.value
  const at = hits.value[hit.value]
  const row = at ? searched.value[at.row] : undefined
  if (!box || !row) return
  const y = rowsTop(box) + row.top
  const margin = 44
  if (y < box.scrollTop + margin || y + CODE_ROW > box.scrollTop + box.clientHeight - margin) {
    box.scrollTop = Math.max(0, y - box.clientHeight / 2)
    top.value = box.scrollTop
  }
  // Now the row is drawn, the mark is there to bring into view sideways, for
  // a match out at the end of a long line.
  await nextTick()
  box
    .querySelector<HTMLElement>('mark.find-hit.now')
    ?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' })
}

function step(by: number) {
  const count = hits.value.length
  if (!count) return
  hit.value = (hit.value + by + count) % count
  findRow = hits.value[hit.value]?.row ?? null
  void reveal()
}

/**
 * Opens the find box, or puts the caret back in it.
 *
 * Text selected in the file is what is being looked for more often than not,
 * so a short piece of one line of it starts the search.
 */
async function openFind() {
  const picked = window.getSelection?.()?.toString() ?? ''
  if (picked && !picked.includes('\n') && picked.length <= 200) query.value = picked
  finding.value = true
  await nextTick()
  findBox.value?.focus()
  findBox.value?.select()
}

/** Closes it. The words stay, so opening it again picks up where it left off. */
function closeFind() {
  finding.value = false
  findBox.value?.blur()
}

// A reload behind the open file — the watcher saw a save — can leave fewer
// matches than the one the arrows were on.
watch(hits, (list) => {
  if (hit.value >= list.length) hit.value = 0
})

watch([query, matchCase], () => {
  findFrom(findRow)
  void reveal()
})

const isMac = navigator.userAgent.includes('Mac')

/** Keys inside the box: the arrows and Enter walk the matches. */
function onFindKey(event: KeyboardEvent) {
  const mod = isMac ? event.metaKey : event.ctrlKey
  if (event.key === 'ArrowDown' || (event.key === 'Enter' && !event.shiftKey)) {
    event.preventDefault()
    step(1)
  } else if (event.key === 'ArrowUp' || (event.key === 'Enter' && event.shiftKey)) {
    event.preventDefault()
    step(-1)
  } else if (mod && event.key.toLowerCase() === 'g') {
    event.preventDefault()
    step(event.shiftKey ? -1 : 1)
  } else if (mod && event.key.toLowerCase() === 'f') {
    event.preventDefault()
    findBox.value?.select()
  }
}

// The commit list owns these keys everywhere else; while a file is open the
// list is not mounted, and they find text in the file instead.
useShortcuts({
  'viewer.search': () => openFind(),
  'viewer.next': () => (finding.value ? step(1) : openFind()),
  'viewer.previous': () => (finding.value ? step(-1) : openFind())
})

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
    // The find box is the thing on top, so it goes first.
    if (finding.value) closeFind()
    else close()
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
      <span class="path" :title="target.path">
        <span class="dir">{{ target.path.slice(0, target.path.lastIndexOf('/') + 1) }}</span>
        <span class="base-name">{{ target.path.slice(target.path.lastIndexOf('/') + 1) }}</span>
      </span>
      <span v-if="language" class="pill">{{ language }}</span>
      <span v-if="target.commit" class="pill">{{ target.commit.slice(0, 7) }}</span>
      <span v-else class="pill">{{ target.side }}</span>
      <span class="stats">
        <span class="plus">+{{ stats.additions }}</span>
        <span class="minus">−{{ stats.deletions }}</span>
      </span>

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
          class="btn stage"
          :disabled="store.busy"
          @click="git.stage([target.path])"
        >
          <Check :size="14" /> Stage file
        </button>
        <button v-else class="btn stage" :disabled="store.busy" @click="git.unstage([target.path])">
          <Minus :size="14" /> Unstage file
        </button>
        <span class="divider" />
      </template>

      <!-- Always here, whichever view is on screen. It used to appear along
           with the file view and shove everything beside it sideways, so
           switching views moved the buttons under the pointer. Blame is still
           a column of the file, so asking for it from the patch opens that
           view rather than doing nothing. -->
      <button
        class="btn tool"
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

      <button class="btn tool" title="Every commit that touched this file" @click="showHistory">
        <History :size="14" />
      </button>
      <button class="btn tool" title="Copy path" @click="copyText(target.path, 'Path')">
        <Copy :size="14" />
      </button>
      <button class="btn tool" :title="git.revealLabel" @click="git.reveal(target.path)">
        <FolderOpen :size="14" />
      </button>
      <button class="btn tool" title="Close (Esc)" @click="close">
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
          :find="findMarks"
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
          :find="findMarks"
          @hunk="onHunk"
        />
      </div>

      <!-- Summoned with ⌘F and gone with Esc, in the corner a browser and an
           editor both put it, over the code rather than above it so the file
           does not jump down a row when it opens. -->
      <div v-if="finding" class="find">
        <Search :size="13" class="faint" />
        <input
          ref="findBox"
          v-model="query"
          type="text"
          spellcheck="false"
          :placeholder="diffMode.mode === 'file' ? 'Find in file' : 'Find in diff'"
          @keydown="onFindKey"
        />
        <span v-if="query" class="count" :class="{ none: !hits.length }">
          {{ hits.length ? `${hit + 1} of ${hits.length}` : 'no matches' }}
        </span>
        <!-- The buttons leave the caret in the box, so the arrows still step
             through the matches after one of them is clicked. -->
        <button
          class="step"
          :class="{ on: matchCase }"
          :title="matchCase ? 'Matching case' : 'Match case'"
          @mousedown.prevent
          @click="matchCase = !matchCase"
        >
          <CaseSensitive :size="14" />
        </button>
        <button
          class="step"
          :disabled="!hits.length"
          :title="`Previous (↑ or ${keyLabel('mod+shift+g')})`"
          @mousedown.prevent
          @click="step(-1)"
        >
          <ChevronUp :size="13" />
        </button>
        <button
          class="step"
          :disabled="!hits.length"
          :title="`Next (↓ or ${keyLabel('mod+g')})`"
          @mousedown.prevent
          @click="step(1)"
        >
          <ChevronDown :size="13" />
        </button>
        <button class="step" title="Close (Esc)" @mousedown.prevent @click="closeFind">
          <X :size="13" />
        </button>
      </div>
      <ChangeRuler :container="body" :marks="marks" />
    </div>
  </section>
</template>

<style scoped>
/* The viewer is a pane of the shell's grid and takes the page tone from it, so
   it paints nothing behind itself. */
.viewer {
  display: grid;
  /* The column is stated rather than left implicit. An `auto` column is sized
     to its content, so one very long line of a diff widens the whole column
     past the window instead of scrolling inside it — the file view then paints
     over the panel beside it and the window layout comes apart. */
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: auto minmax(0, 1fr);
  min-width: 0;
}

/* The bar over the file is chrome, the same tone as the toolbar and the
   sidebar, so the diff under it reads as the page. */
.bar {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 40px;
  padding: 5px 8px 5px 12px;
  background: var(--canvas);
  border-bottom: 1px solid var(--line);
}

/* The file's name is what you are looking at; the folders it sits in are
   where, and read quieter. */
.path {
  display: flex;
  align-items: baseline;
  min-width: 0;
  max-width: 46%;
  font-size: 12.5px;
}

.dir {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-faint);
}

.base-name {
  flex: none;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
  color: var(--text);
}

/* The language and the side are labels, not counts: square-cornered tags. */
.bar .pill {
  border-radius: var(--radius-sm);
  font-family: var(--mono);
  font-weight: 500;
}

/* Lines added and taken away, read as one small tally. */
.stats {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-family: var(--mono);
  font-size: 11.5px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.grow {
  flex: 1;
}

/* Diff or file: a bordered segmented switch, the same control the file panel
   uses for path and tree. */
.modes {
  display: flex;
  flex: none;
  height: 24px;
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  overflow: hidden;
  background: var(--bg);
}

.seg {
  padding: 0 10px;
  font-size: 11.5px;
  font-weight: 500;
  color: var(--text-dim);
}

.seg + .seg {
  border-left: 1px solid var(--line);
}

.seg:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.seg.on {
  background: var(--bg-active);
  color: var(--accent-soft);
  font-weight: 600;
}

/* Everything on the bar is one size smaller than a toolbar button. */
.bar .btn {
  min-height: 26px;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
}

.bar .tool {
  width: 26px;
  padding: 0;
}

/* A toggle rather than an action: on, it holds the pressed look the segmented
   control uses, so the bar has one idea of what "this is on" looks like. */
.btn.on {
  background: var(--bg-active);
  color: var(--accent-soft);
}

.plus {
  color: var(--green);
}

.minus {
  color: var(--red);
}

/* Discarding is quiet until the pointer is on it, then it says what it is. */
.danger {
  color: var(--text-dim);
}

.bar .danger:hover:not(:disabled) {
  background: var(--danger-bg);
  color: var(--red-soft);
}

/* Staging is the thing this bar is for, so it is the one bordered button. */
.stage {
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--line);
}

.bar .stage:hover:not(:disabled) {
  background: var(--bg-hover);
}

.divider {
  flex: none;
  width: 1px;
  height: 18px;
  margin: 0 3px;
  background: var(--line);
}

.pane {
  position: relative;
  display: flex;
  min-width: 0;
  min-height: 0;
}

/* --- the find box */

/* Floats in the top right corner of the code, clear of the strip beside the
   scrollbar. */
.find {
  position: absolute;
  top: 8px;
  right: 22px;
  z-index: 8;
  display: flex;
  align-items: center;
  gap: 4px;
  width: 340px;
  max-width: calc(100% - 44px);
  height: var(--control-h);
  padding: 0 4px 0 9px;
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  box-shadow: var(--shadow-pop);
}

.find:focus-within {
  border-color: var(--ring);
}

.find input {
  flex: 1;
  min-width: 0;
  border: none;
  background: none;
  padding: 2px 0;
  font-size: 12.5px;
}

.find input:focus {
  outline: none;
  box-shadow: none;
}

.count {
  font-size: 11px;
  color: var(--text-dim);
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}

.count.none {
  color: var(--amber);
}

.step {
  display: grid;
  flex: none;
  place-items: center;
  width: 22px;
  height: 22px;
  border-radius: var(--radius-sm);
  color: var(--text-faint);
}

.step:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
}

.step:disabled {
  opacity: 0.35;
}

.step.on {
  background: var(--bg-active);
  color: var(--accent-soft);
}

/* Every match is lit; the one the arrows are on is lit harder, and outlined so
   it can be told apart on a line tinted green or red. */
.body :deep(mark.find-hit) {
  color: inherit;
  background: color-mix(in srgb, var(--amber) 32%, transparent);
  border-radius: 2px;
}

.body :deep(mark.find-hit.now) {
  background: color-mix(in srgb, var(--amber) 70%, transparent);
  box-shadow: 0 0 0 1px var(--amber);
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
  gap: 6px;
  padding: 64px 24px;
  text-align: center;
}

.lfs .glyph {
  color: var(--text-faint);
}

.lfs h3 {
  margin: 6px 0 0;
  font-size: 14px;
  font-weight: 650;
}

.lfs p {
  margin: 0;
  max-width: 460px;
  font-size: 12.5px;
}

.lfs .oid {
  font-size: 11px;
  word-break: break-all;
}

.lfs .btn-primary {
  margin-top: 8px;
}
</style>
