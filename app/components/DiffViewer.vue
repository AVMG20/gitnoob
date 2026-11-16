<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from 'vue'
import { onCodeFont } from '~/composables/useCodeFont'
import {
  ArrowDownToLine,
  CaseSensitive,
  Check,
  ChevronDown,
  ChevronUp,
  Ellipsis,
  FileBox,
  FilePlus,
  FileText,
  FolderTree,
  History,
  Minus,
  Search,
  Trash2,
  TriangleAlert,
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
import { useContextMenu, type MenuItem } from '~/composables/useContextMenu'
import { labelFor } from '~/composables/useHighlight'
import { diffMode, type DiffMode } from '~/composables/useDiffMode'
import type { PickedLines } from '~/components/DiffView.vue'
import { humanSize, readPointer } from '~/composables/useLfs'
import {
  deletingFiles,
  stepFile,
  useFileView,
  walkOrder,
  type FileStep
} from '~/composables/useFileView'
import { headerState } from '~/composables/fileHeader'
import { framework, openWiringForFile } from '~/composables/useFramework'
import { revealInFiles } from '~/composables/useFileTree'
import {
  codeRow,
  diffRows,
  fileMarks,
  firstChangedLine,
  markedLines,
  patchMarks
} from '~/composables/useCode'
import { findIn, type FindMarks } from '~/composables/useFind'
import { keyLabel, useShortcuts } from '~/composables/useShortcuts'
import { closeTab, goToLine, openTab, showGraph, tabId, useTabs } from '~/composables/useTabs'
import { aimedAt, invoke } from '~/composables/useInvoke'
import {
  closeTabsSafely,
  editKey,
  readFile,
  saveErrorOf,
  saveStateOf
} from '~/composables/useFileEdit'
import FileEditor, { type EditorFacts } from '~/components/FileEditor.vue'
import { indentLabel, indentOverrides } from '~/composables/indentation'
import ImageView from '~/components/ImageView.vue'
import ImageDiff from '~/components/ImageDiff.vue'
import {
  bytesLabel,
  formatOf,
  isImagePath,
  isSvgPath,
  type ImageInfo,
  type ImageSide
} from '~/composables/useImage'
import FileIcon from '~/components/FileIcon.vue'
import MidTruncate from '~/components/MidTruncate.vue'

/** The order Tab walks the views in. */
const MODES: DiffMode[] = ['diff', 'file']

const git = useGit()
const menu = useContextMenu()
const store = git.store
const view = useFileView()
const tabs = useTabs()

const diff = ref<FileDiff | null>(null)
const loading = ref(false)
/** The file itself, for the whole-file view. Only read when that view is on. */
const text = ref<string | null>(null)
const textError = ref<string | null>(null)

/**
 * The working tree against the last commit, for the editor's change bars.
 * The diff above is one side of the index; the bars mark everything since
 * the commit, staged or not.
 */
const headDiff = ref<FileDiff | null>(null)
/** The text in the editor as it now stands, which the find box searches. */
const editorText = ref<string | null>(null)
/** The element the editor scrolls in, for the strip beside it. */
const editorScroller = ref<HTMLElement | null>(null)
const editor = ref<{
  goToLine: (line: number, mark?: boolean) => void
  nextChange: (by: number) => void
  focus: () => void
  jumpAtCaret: () => void
} | null>(null)
/**
 * Nothing on disk and nothing in the status: deleted and committed since the
 * tab was opened, or a tab reopened by back after the file went.
 */
const missing = ref(false)

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

/** The tab on screen, for its save state and its id. */
const currentId = computed(() => (target.value ? tabId(target.value.path, target.value.commit) : ''))

/**
 * Whether File mode is the editor: a file in the work tree that is there.
 * A commit's copy and a deleted file keep the read-only view, with its
 * blame column.
 */
const editing = computed(
  () =>
    diffMode.mode === 'file' &&
    !!target.value &&
    !target.value.commit &&
    !gone.value &&
    !missing.value &&
    !showImage.value
)

// --- images
//
// An image is drawn, not read as text: `ImageView` in File mode, `ImageDiff`
// (before and after) in Diff mode. An SVG has a Source switch that puts its
// text in the editor like any other file.

const isImage = computed(() => !!target.value && isImagePath(target.value.path))
const isSvg = computed(() => !!target.value && isSvgPath(target.value.path))
/** An SVG shown as its text, for this tab, this session. */
const svgSource = ref(false)
const showImage = computed(() => isImage.value && !(isSvg.value && svgSource.value))
/** What the image on screen is: its size in pixels and bytes, its type. */
const imageInfo = ref<ImageInfo | null>(null)
/** The pointer text, when the image on disk is an LFS pointer. */
const imageLfs = ref<string | null>(null)
/** Goes up on every reload, so the before/after is read again after a write. */
const imageVersion = ref(0)

// The text is only read for an SVG shown as its source.
watch(svgSource, () => void load(false))

/** Which copy File mode shows: the commit's, or the work tree's (the last
    copy git has, for one deleted from disk). */
const imageSide = computed<ImageSide>(() => {
  const current = target.value
  if (current?.commit) return { source: 'commit', commit: current.commit }
  if (gone.value) return { source: (current?.side ?? 'unstaged') === 'staged' ? 'head' : 'index' }
  return { source: 'work' }
})

/** The two sides Diff mode compares, matching the side of the index on show. */
const imageSides = computed<{ before: ImageSide; after: ImageSide }>(() => {
  const current = target.value
  if (current?.commit) {
    return { before: { source: 'parent', commit: current.commit }, after: { source: 'commit', commit: current.commit } }
  }
  return (current?.side ?? 'unstaged') === 'staged'
    ? { before: { source: 'head' }, after: { source: 'index' } }
    : { before: { source: 'index' }, after: { source: 'work' } }
})

/**
 * A chip for the save states that need doing something about. Saving, saved
 * and unsaved are already on the tab (its dot) and in the panel (the M), so
 * the bar says nothing about them.
 */
const saveLabel = computed(() => {
  if (!editing.value) return null
  switch (saveStateOf(aimedAt(), currentId.value)) {
    case 'conflict':
      return { text: 'Changed on disk', title: 'The file changed on disk under your edits; the bar below asks which to keep' }
    case 'failed':
      return { text: 'Not saved', title: saveErrorOf(aimedAt(), currentId.value) ?? 'The last save failed' }
    default:
      return null
  }
})
const language = computed(() => (target.value ? labelFor(target.value.path) : null))

// --- the bar over the file
//
// Which badge and which buttons it has is decided in one place, `headerState`,
// from the file, the status and the tab. This only gathers what it needs.

/** How the editor found the file: read-only or not, its endings, a BOM. */
const facts = ref<EditorFacts | null>(null)
/** Git ignores the file. Only asked for one the status does not list. */
const ignored = ref(false)
/** Commit subjects, for the hash's tooltip on a commit tab. */
const subjects = ref<Record<string, string>>({})

const head = computed(() => {
  const current = target.value
  if (!current) return null
  const status = store.status
  const staged = status?.staged.find((entry) => entry.path === current.path) ?? null
  const unstaged = status?.unstaged.find((entry) => entry.path === current.path) ?? null
  return headerState(
    {
      path: current.path,
      readonly: facts.value?.readonly ?? null,
      ignored: ignored.value,
      missing: missing.value
    },
    {
      staged: staged ? { kind: staged.kind ?? 'modified' } : null,
      unstaged: unstaged ? { kind: unstaged.kind ?? 'modified' } : null,
      conflicted: !!status?.conflicted.includes(current.path)
    },
    { commit: current.commit, subject: current.commit ? subjects.value[current.commit] : null }
  )
})

/** The folders above the file, each one a way to it in the Files tree. */
const crumbs = computed(() => {
  const path = target.value?.path ?? ''
  const parts = path.split('/').slice(0, -1)
  return parts.map((name, at) => ({ name, path: parts.slice(0, at + 1).join('/') }))
})
const baseName = computed(() => target.value?.path.split('/').pop() ?? '')

/** Where the caret is, as Ln and Col, from the tab and the editor's text. */
const caret = computed(() => {
  const tab = tabs.state.tabs.find((one) => one.id === currentId.value)
  const now = editorText.value ?? ''
  const at = Math.min(tab?.caret ?? 0, now.length)
  // `lastIndexOf` reads a start of -1 as 0, and would find a newline that
  // opens the file: at the very start there is nothing before the caret.
  const start = at === 0 ? 0 : now.lastIndexOf('\n', at - 1) + 1
  return { line: tab?.line ?? 1, col: at - start + 1 }
})

/** A narrow window keeps the bar to one row by folding the quieter tools away. */
const bar = ref<HTMLElement | null>(null)
const narrow = ref(false)
let barSizer: ResizeObserver | null = null
watch(bar, (element) => {
  barSizer?.disconnect()
  if (!element || typeof ResizeObserver === 'undefined') return
  barSizer = new ResizeObserver(() => (narrow.value = element.clientWidth < 760))
  barSizer.observe(element)
})

watch(
  () => [target.value?.path, target.value?.commit],
  () => {
    facts.value = null
    ignored.value = false
    svgSource.value = false
    imageInfo.value = null
    imageLfs.value = null
    void loadSubject()
  }
)

/** The subject of the commit a commit tab shows, read once per commit. */
async function loadSubject() {
  const oid = target.value?.commit
  if (!oid || subjects.value[oid] !== undefined) return
  const known = store.detail?.oid === oid ? store.detail.summary : null
  if (known !== null) {
    subjects.value = { ...subjects.value, [oid]: known }
    return
  }
  const found = await git.commitDetail(oid)
  if (found) subjects.value = { ...subjects.value, [oid]: found.summary }
}

/** Asks git whether a file the status does not list is one it ignores. */
async function loadIgnored(current: NonNullable<typeof target.value>) {
  if (current.commit) return
  const listed = [...(store.status?.staged ?? []), ...(store.status?.unstaged ?? [])].some(
    (entry) => entry.path === current.path
  )
  if (listed) {
    ignored.value = false
    return
  }
  const answer = await invoke<boolean>('path_ignored', { path: current.path }).catch(() => false)
  if (target.value === current) ignored.value = !!answer
}

// A file with nothing to compare is read in File mode; there is no Diff to show.
watch(
  () => [head.value?.modes, head.value?.kind, !!store.status] as const,
  ([modes, kind, known]) => {
    // Everything committed or thrown away closes a diff tab instead (below).
    const status = store.status
    const empty = !!status && !status.staged.length && !status.unstaged.length && !status.conflicted.length
    if (known && !empty && modes === false && kind !== 'missing' && diffMode.mode === 'diff') {
      diffMode.mode = 'file'
    }
  }
)

/** The path from the top of the repository, or on disk. */
function absolutePath(path: string) {
  const root = store.repo?.path ?? ''
  return root ? `${root.replace(/[\\/]$/, '')}/${path}` : path
}

const INDENT_FROM: Record<string, string> = {
  override: 'picked here for this file',
  editorconfig: 'from .editorconfig',
  file: 'read from the file',
  settings: 'from Settings → Editor',
  language: 'the usual for this language',
  default: 'from Settings → Editor'
}

const indentTitle = computed(() => {
  const now = facts.value?.indent
  return now ? `Indenting with ${indentLabel(now).toLowerCase()}, ${INDENT_FROM[now.from]}. Click to change it for this file.` : ''
})

/** Spaces or tabs for this file, this session; or back to what was worked out. */
function showIndentMenu(event: MouseEvent) {
  const key = editKey(aimedAt(), currentId.value)
  const now = facts.value?.indent
  const pick = (tabs: boolean, size: number) => () => {
    indentOverrides[key] = { tabs, size }
  }
  const items: MenuItem[] = [
    ...[2, 4, 8].map((size) => ({
      label: `Spaces: ${size}`,
      hint: now && !now.tabs && now.size === size ? 'now' : undefined,
      action: pick(false, size)
    })),
    { label: 'Tabs', hint: now?.tabs ? 'now' : undefined, action: pick(true, 0) }
  ]
  if (indentOverrides[key]) {
    items.push(
      { label: '', separator: true },
      { label: 'Back to what the file says', action: () => delete indentOverrides[key] }
    )
  }
  menu.show(event, items, 'Indentation for this file')
}

/** The ⋯ menu: everything the bar has no room or no reason to show. */
function showMore(event: MouseEvent) {
  const current = target.value
  if (!current) return
  const items = []
  if (narrow.value) {
    items.push(
      { label: 'History', action: () => void showHistory(event) },
      {
        label: diffMode.blame && diffMode.mode === 'file' ? 'Hide blame' : 'Show blame',
        action: toggleBlame
      },
      { label: 'Reveal in Files', action: () => revealInFiles(current.path) }
    )
  }
  items.push(
    { label: 'Copy path', action: () => copyText(absolutePath(current.path), 'Path') },
    { label: 'Copy relative path', action: () => copyText(current.path, 'Path') }
  )
  // The framework's account of the class in this file (Magento's magequery).
  if (!current.commit && framework.info?.wiring && /\.php$/i.test(current.path)) {
    items.push({ label: `${framework.info.name} wiring`, action: () => void openWiringForFile(current.path) })
  }
  if (!current.commit) {
    items.push(
      { label: git.revealLabel, action: () => void git.reveal(current.path) },
      {
        label: 'Open in default app',
        action: () =>
          void invoke('open_default', { path: current.path }).catch((error) => git.note(String(error)))
      }
    )
  }
  menu.show(event, items, current.path)
}

/** The git buttons the state asks for. */
function act(action: string) {
  const current = target.value
  if (!current || store.busy) return
  if (action === 'discard') void git.discard([current.path])
  else if (action === 'stage' || action === 'add') void git.stage([current.path])
  else if (action === 'unstage') void git.unstage([current.path])
  else if (action === 'delete') deletingFiles.value = [current.path]
  else if (action === 'open-current') openTab({ path: current.path, mode: 'file' })
}
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
  imageVersion.value++
  void loadIgnored(current)
  await loadText()
  await loadHead(current)
  await loadBlame()
  loading.value = false
  if (settle) {
    // A tab come back to is read from where it was left; a file just opened
    // from wherever its change is.
    const kept = tabs.activeTab()?.scrollTop
    if (editing.value) {
      // The editor keeps its own place.
    } else if (kept !== undefined && body.value) {
      await nextTick()
      body.value.scrollTop = kept
      top.value = body.value.scrollTop
    } else await toFirstChange()
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
  if (!box || editing.value) return
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
  box.scrollTop = Math.max(0, (at - 1) * codeRow() - 72)
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
  if (showImage.value) {
    // Drawn from its bytes by the image views; there is no text to read.
    text.value = null
    missing.value = false
    return
  }
  try {
    text.value = await git.fileText(current.path, current.commit, current.side ?? 'unstaged')
    if (target.value === current) missing.value = false
  } catch (error) {
    text.value = null
    textError.value = String(error)
    // Gone from the disk and from git's index and HEAD: there is nothing to
    // show, and that is the page rather than an error.
    if (!current.commit && target.value === current) missing.value = await isMissing(current.path)
  }
}

/** Whether a working-tree path has nothing on disk and nothing in the status. */
async function isMissing(path: string) {
  const listed = [...(store.status?.staged ?? []), ...(store.status?.unstaged ?? [])].some(
    (entry) => entry.path === path
  )
  if (listed) return false
  const read = await readFile(path)
  return !!read?.missing
}

/** The bars' diff, for a working-tree file shown in the editor. */
async function loadHead(current: NonNullable<typeof target.value>) {
  if (current.commit || diffMode.mode !== 'file' || showImage.value) {
    headDiff.value = null
    return
  }
  try {
    const found = await invoke<FileDiff>('head_file_diff', { path: current.path })
    if (target.value === current) headDiff.value = found
  } catch {
    if (target.value === current) headDiff.value = null
  }
}

/**
 * "Show in Diff" on a change bar's card: the same change in Diff mode, at its
 * hunk, on whichever side of the index it is on.
 */
async function toBar(line: number) {
  const current = target.value
  if (!current || current.commit) return
  const unstaged = (store.status?.unstaged ?? []).some((entry) => entry.path === current.path)
  const side = unstaged ? 'unstaged' : 'staged'
  pendingLine = line
  if (current.side !== side) store.viewer = { path: current.path, side }
  diffMode.mode = 'diff'
}

/** A line the diff should scroll to once it is drawn, from a bar click. */
let pendingLine: number | null = null

async function toPendingHunk() {
  const line = pendingLine
  pendingLine = null
  const box = body.value
  if (line === null || !box || diffMode.mode !== 'diff') return
  const hunks = diff.value?.hunks ?? []
  const covering = hunks.findIndex((hunk) => {
    const found = /\+(\d+)(?:,(\d+))?/.exec(hunk.header)
    const start = Number(found?.[1] ?? 0)
    const count = found?.[2] === undefined ? 1 : Number(found[2])
    return line >= start && line <= start + Math.max(count, 1)
  })
  const index = covering < 0 ? 0 : covering
  await nextTick()
  const head = diffRows(hunks).rows.find((row) => row.kind === 'head' && row.hunk === index)
  if (!head) return
  box.scrollTop = Math.max(0, rowsTop(box) + head.top - 8)
  top.value = box.scrollTop
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
      openTab({ path: current.path, commit: one.oid, mode: 'diff' })
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
const pointer = computed(() => readPointer(text.value) ?? readPointer(imageLfs.value))

async function fetchFromLfs() {
  const current = target.value
  if (!current) return
  await git.lfsPull(current.path)
  imageLfs.value = null
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
  const current = target.value
  if (current) await loadHead(current)
  await loadBlame()
  if (pendingLine !== null) await toPendingHunk()
  else await toFirstChange()
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
  if (editing.value) {
    return fileMarks(markedLines(editorText.value ?? text.value, headDiff.value?.hunks ?? []))
  }
  if (diffMode.mode === 'file') return fileMarks(markedLines(text.value, diff.value?.hunks ?? []))
  const laid = diffRows(diff.value?.hunks ?? [])
  return patchMarks(laid.rows, laid.height)
})

/** Closes this file's tab; the last one closed brings the graph back. */
function close() {
  const tab = tabs.activeTab()
  if (tab) void closeTabsSafely([tab.id], () => closeTab(tab.id))
}

/** Flips between the diff and the whole file. */
function flipMode(by = 1) {
  // Nothing changed, nothing to compare: Tab and the key leave the file be.
  if (head.value && !head.value.modes) return
  const at = MODES.indexOf(diffMode.mode)
  diffMode.mode = MODES[(at + by + MODES.length) % MODES.length]!
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
    const shown = editing.value ? (editorText.value ?? text.value) : text.value
    if (diff.value?.binary || shown === null) return []
    const source = shown.split('\n')
    // The same last empty piece `markedLines` drops, so a row here is a row there.
    if (source.length && source[source.length - 1] === '') source.pop()
    return source.map((line, at) => ({ text: line, top: at * codeRow() }))
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
    const box = editing.value ? editorScroller.value : body.value
    const scrolled = box ? box.scrollTop - (editing.value ? 0 : rowsTop(box)) : 0
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
  // The editor brings its own current match into view.
  if (editing.value) return
  const at = hits.value[hit.value]
  const row = at ? searched.value[at.row] : undefined
  if (!box || !row) return
  const y = rowsTop(box) + row.top
  const margin = 44
  if (y < box.scrollTop + margin || y + codeRow() > box.scrollTop + box.clientHeight - margin) {
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
  'viewer.previous': () => (finding.value ? step(-1) : openFind()),
  'file.mode': () => flipMode(),
  'file.line': () => openLine(),
  'file.nextChange': () => changeStep(1),
  'file.previousChange': () => changeStep(-1),
  'file.jump': () => editor.value?.jumpAtCaret()
})

// --- go to line

const asking = ref(false)
const lineInput = ref('')
const lineBox = ref<HTMLInputElement | null>(null)

async function openLine() {
  lineInput.value = ''
  asking.value = true
  await nextTick()
  lineBox.value?.focus()
}

function closeLine() {
  asking.value = false
  if (editing.value) editor.value?.focus()
}

/** A jump, so back returns to where the caret was. Diff mode goes to the file. */
function submitLine() {
  const line = Number.parseInt(lineInput.value, 10)
  asking.value = false
  if (!Number.isFinite(line) || line < 1) return
  if (diffMode.mode !== 'file') diffMode.mode = 'file'
  goToLine(line)
}

function onLineKey(event: KeyboardEvent) {
  if (event.key === 'Enter') {
    event.preventDefault()
    submitLine()
  } else if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    closeLine()
  }
}

/** Where a jump lands when the read-only views are on screen. */
async function landOutsideEditor(line: number) {
  const box = body.value
  if (!box || editing.value) return
  if (diffMode.mode !== 'file') return
  await nextTick()
  box.scrollTop = Math.max(0, (line - 1) * codeRow() - box.clientHeight / 2)
  top.value = box.scrollTop
}

watch(
  () => tabs.state.landing,
  (landing) => {
    if (landing && landing.tab === currentId.value) void landOutsideEditor(landing.line)
  }
)

// --- the next change, in the file or the patch

function changeStep(by: number) {
  if (editing.value) {
    editor.value?.nextChange(by)
    return
  }
  const box = body.value
  if (!box) return
  if (diffMode.mode === 'diff') {
    const heads = diffRows(diff.value?.hunks ?? []).rows.filter((row) => row.kind === 'head')
    const offset = rowsTop(box)
    const now = box.scrollTop - offset
    const next =
      by > 0
        ? heads.find((row) => row.top > now + 2)
        : [...heads].reverse().find((row) => row.top < now - 2)
    if (next) {
      box.scrollTop = Math.max(0, offset + next.top)
      top.value = box.scrollTop
    }
    return
  }
  const lines = markedLines(text.value, diff.value?.hunks ?? [])
  const starts = lines
    .filter((line, at) => (line.mark || line.removed.length) && !lines[at - 1]?.mark)
    .map((line) => line.number)
  const now = Math.floor(box.scrollTop / codeRow()) + 1
  const next =
    by > 0 ? starts.find((line) => line > now + 1) : [...starts].reverse().find((line) => line < now - 1)
  if (next !== undefined) {
    box.scrollTop = Math.max(0, (next - 1) * codeRow() - 72)
    top.value = box.scrollTop
  }
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

/**
 * Stage, unstage or discard one hunk, then reload so the view is honest —
 * without losing the reader's place.
 *
 * It used to reload as though the file had just been opened, which put the
 * page back at the top: in a long file every hunk staged was a scroll back
 * down to the next one. Now the hunk that takes the acted-on one's place — the
 * next, once this one has gone to the other side — lands where its heading
 * was. When nothing is left on this side the file is done with, and the
 * viewer moves on the way staging a whole file does.
 */
async function onHunk(
  index: number,
  action: 'stage' | 'unstage' | 'discard',
  lines?: PickedLines
) {
  const current = target.value
  if (!current || current.commit) return
  const side = current.side ?? 'unstaged'
  const box = body.value
  // Where the heading sat in the box, kept on screen: a heading scrolled
  // past (the pinned copy was the one pressed) puts the next one at the top.
  const before = diffRows(diff.value?.hunks ?? []).rows.find(
    (row) => row.kind === 'head' && row.hunk === index
  )
  const offset = box && before ? Math.max(0, rowsTop(box) + before.top - box.scrollTop) : 0
  // Read before the status moves, the way the file buttons do it.
  const next = side === 'unstaged' ? git.nextToRead(current.path) : null

  const said = await git.applyHunk(current.path, index, action, lines)
  if (target.value !== current) return
  await load(false)
  if (target.value !== current) return

  const hunks = diff.value?.hunks ?? []
  if (said !== null && !hunks.length) {
    // Nothing left on this side. Staging or discarding goes on to the next
    // file to read; unstaging follows the file to where its changes now are.
    store.viewer =
      side === 'unstaged' ? (next ? { path: next, side: 'unstaged' } : null) : git.sideOf(current.path)
    return
  }
  if (!box) return
  await nextTick()
  const laid = diffRows(hunks).rows.filter((row) => row.kind === 'head')
  const landing = laid[Math.min(index, laid.length - 1)]
  if (!landing) return
  box.scrollTop = Math.max(0, rowsTop(box) + landing.top - offset)
  top.value = box.scrollTop
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    // The find box is the thing on top, so it goes first. Otherwise Escape is
    // the way home: the graph comes forward and the file stays in its tab.
    // A dialog in front of the viewer takes the key, and takes it alone.
    if (covered()) return
    if (finding.value) closeFind()
    else if (!typing(event)) showGraph()
    return
  }
  if (typing(event) || covered() || event.altKey || event.ctrlKey || event.metaKey) return
  // Tab flips between the patch and the file, which is the one thing anyone
  // does twice while reading a change. Left alone wherever it still means
  // "next field", and wherever a modifier makes it mean something else.
  if (event.key === 'Tab') {
    event.preventDefault()
    flipMode(event.shiftKey ? -1 : 1)
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

/** The tab on screen, so where it was scrolled to can be kept on leaving it. */
let shown: string | null = null

function keepScroll() {
  const box = body.value
  // The editor keeps its own place, on its own way out.
  if (!shown || !box || editing.value) return
  const left = tabs.state.tabs.find((tab) => tab.id === shown)
  if (left) left.scrollTop = box.scrollTop
}

watch(
  target,
  (now) => {
    keepScroll()
    shown = now ? tabId(now.path, now.commit) : null
    load()
  },
  { deep: true }
)
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
    // A tab reading the whole file is about the file, not its changes, and
    // stays open.
    if (target.value && !target.value.commit && nothingLeft && diffMode.mode === 'diff') {
      close()
      return
    }
    load(false)
  }
)

onMounted(() => {
  shown = target.value ? tabId(target.value.path, target.value.commit) : null
  load()
  void loadSubject()
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

// The code changed size: the line at the top stays at the top. The editor
// keeps its own place.
const dropFontKeeper = onCodeFont(() => {
  const box = body.value
  if (!box || editing.value) return
  const top = box.scrollTop
  const row = codeRow()
  return () => {
    box.scrollTop = (top * codeRow()) / row
  }
})

onBeforeUnmount(() => {
  dropFontKeeper()
  keepScroll()
  body.value?.removeEventListener('scroll', onScroll)
  sizer?.disconnect()
  barSizer?.disconnect()
})

onUnmounted(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <section v-if="target" class="viewer">
    <header ref="bar" class="bar" :data-state="head?.kind">
      <span class="type" :title="language ?? undefined"><FileIcon :path="target.path" :size="15" /></span>
      <!-- Where the file is: each folder a way to it in the Files tree. The
           folders give way first and the name never does, the way MidTruncate
           cuts a long name in the middle rather than at the end. -->
      <span class="path" :title="target.path">
        <span class="dir">
          <template v-for="crumb in crumbs" :key="crumb.path">
            <button
              class="crumb"
              :title="`Show ${crumb.path} in Files`"
              @click="revealInFiles(crumb.path)"
            >
              {{ crumb.name }}</button
            ><span class="sep">/</span>
          </template>
        </span>
        <MidTruncate class="base-name" :text="baseName" :tail="8" />
      </span>
      <span
        v-for="badge in head?.badges ?? []"
        :key="badge.text"
        class="badge"
        :class="badge.tone"
        :title="badge.title"
        >{{ badge.text }}</span
      >
      <span v-if="head?.commit" class="pill hash" :title="head.commit.subject || target.commit!">{{
        head.commit.short
      }}</span>
      <span v-else-if="head?.kind === 'modified'" class="pill">{{ target.side }}</span>
      <span v-if="head?.counts && !isImage" class="stats">
        <span class="plus">+{{ stats.additions }}</span>
        <span class="minus">−{{ stats.deletions }}</span>
      </span>
      <span v-if="saveLabel" class="save-warning" :title="saveLabel.title"
        ><TriangleAlert :size="11" />{{ saveLabel.text }}</span
      >

      <span class="grow" />

      <!-- How the file is written, in the editor: where the caret is (click to
           go to a line), its line endings, its encoding, its language. -->
      <span v-if="showImage && diffMode.mode === 'file' && imageInfo" class="facts" data-fact="image">
        <span class="fact" title="Width × height in pixels">{{ imageInfo.width }} × {{ imageInfo.height }}</span>
        <span class="fact">{{ bytesLabel(imageInfo.size) }}</span>
        <span class="fact">{{ formatOf(imageInfo.mime) }}</span>
      </span>
      <button
        v-if="isSvg"
        class="btn tool svg-source"
        :class="{ on: svgSource }"
        data-act="svg-source"
        :title="svgSource ? 'Show the image' : 'Show the SVG as text, to edit it'"
        @click="svgSource = !svgSource"
      >
        {{ svgSource ? 'Image' : 'Source' }}
      </button>
            <span v-if="editing && facts" class="facts">
        <button class="fact" title="Go to line (⌘G)" @click="openLine">
          Ln {{ caret.line }}, Col {{ caret.col }}
        </button>
        <template v-if="!narrow">
          <span class="fact" :title="facts.eol === 'crlf' ? 'Windows line endings' : 'Unix line endings'">{{
            facts.eol === 'crlf' ? 'CRLF' : 'LF'
          }}</span>
          <span class="fact">{{ facts.bom ? 'UTF-8 BOM' : 'UTF-8' }}</span>
          <span v-if="language" class="fact">{{ language }}</span>
        </template>
        <button
          v-if="facts.indent"
          class="fact"
          data-fact="indent"
          :title="indentTitle"
          @click="showIndentMenu"
        >
          {{ indentLabel(facts.indent) }}
        </button>
      </span>

      <!-- Two ways to read the same change: the patch, or the file with the
           change marked in it. A file with no change has only the one. -->
      <span class="modes" :class="{ off: !head?.modes }" :title="head?.modes ? undefined : 'No changes'">
        <button
          class="seg"
          :class="{ on: diffMode.mode === 'diff' && head?.modes }"
          :disabled="!head?.modes"
          :title="head?.modes ? 'The changed lines, in hunks (Tab)' : 'No changes'"
          @click="diffMode.mode = 'diff'"
        >
          Diff
        </button>
        <button
          class="seg"
          :class="{ on: diffMode.mode === 'file' }"
          :disabled="!head?.modes"
          :title="head?.modes ? 'The whole file, with the changes marked down the side (Tab)' : 'No changes'"
          @click="diffMode.mode = 'file'"
        >
          File
        </button>
      </span>

      <!-- The git buttons, as the state has them: Discard left of Stage, the
           destructive one never where the hand already is. -->
      <template v-if="head?.actions.length">
        <template v-for="action in head.actions" :key="action">
          <button
            v-if="action === 'discard'"
            class="btn danger"
            :disabled="store.busy"
            title="Throw away the unstaged changes to this file"
            @click="act('discard')"
          >
            <Undo2 :size="14" /> Discard
          </button>
          <button v-else-if="action === 'stage'" class="btn stage" :disabled="store.busy" @click="act('stage')">
            <Check :size="14" /> Stage file
          </button>
          <button v-else-if="action === 'unstage'" class="btn stage" :disabled="store.busy" @click="act('unstage')">
            <Minus :size="14" /> Unstage file
          </button>
          <button
            v-else-if="action === 'delete'"
            class="btn danger"
            :disabled="store.busy"
            title="Delete it from disk; git has no copy"
            @click="act('delete')"
          >
            <Trash2 :size="14" /> Delete file
          </button>
          <button
            v-else-if="action === 'add'"
            class="btn stage"
            :disabled="store.busy"
            title="Start tracking it: stage the whole file"
            @click="act('add')"
          >
            <FilePlus :size="14" /> Add file
          </button>
          <button
            v-else-if="action === 'open-current'"
            class="btn"
            title="The file as it is in the work tree now"
            @click="act('open-current')"
          >
            <FileText :size="14" /> Open current version
          </button>
        </template>
        <span class="divider" />
      </template>

      <!-- The same box ⌘F opens, for whoever does not know the key. A press
           leaves the selection alone so selected text still starts it. -->
      <button
        class="btn tool"
        :class="{ on: finding }"
        :title="finding ? 'Close the find box (Esc)' : `Find in the ${diffMode.mode} (${keyLabel('mod+f')})`"
        @mousedown.prevent
        @click="finding ? closeFind() : openFind()"
      >
        <Search :size="14" />
      </button>

      <!-- Always in the same place whichever view is on screen; asking for it
           from the patch opens the file view with the column. -->
      <template v-if="!narrow">
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
        <button
          v-if="!target.commit"
          class="btn tool"
          title="Show it in the Files tree"
          @click="revealInFiles(target.path)"
        >
          <FolderTree :size="14" />
        </button>
      </template>
      <button class="btn tool more" title="More" @click="showMore">
        <Ellipsis :size="14" />
      </button>
      <button class="btn tool" title="Close this tab" @click="close">
        <X :size="16" />
      </button>
    </header>

    <div class="pane">
      <div
        ref="body"
        class="body"
        :class="{ editing: editing || showImage }"
        :data-code-view="showImage || pointer ? undefined : ''"
      >
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

        <!-- Deleted and gone from git's index and HEAD too: there is no copy
             to show, and the page says so rather than showing an error. -->
        <div v-else-if="missing" class="lfs missing">
          <h3>{{ target?.path.split('/').pop() }}</h3>
          <p class="dim">
            This file is not in the work tree any more, and nothing uncommitted is left of it. Its
            history still has every version that was committed.
          </p>
          <span class="row-buttons">
            <button class="btn" @click="showHistory">
              <History :size="14" /> Its history
            </button>
            <button class="btn" @click="close">Close the tab</button>
          </span>
        </div>

        <ImageDiff
          v-else-if="showImage && target && diffMode.mode === 'diff'"
          :key="`diff:${currentId}`"
          :path="target.path"
          :before="imageSides.before"
          :after="imageSides.after"
          :version="imageVersion"
        />
        <ImageView
          v-else-if="showImage && target"
          :key="`image:${currentId}`"
          :path="target.path"
          :side="imageSide"
          @info="(value: ImageInfo | null) => (imageInfo = value)"
          @lfs="(value: string) => (imageLfs = value)"
        />

        <FileEditor
          v-else-if="editing && target"
          :key="currentId"
          ref="editor"
          :tab-id="currentId"
          :path="target.path"
          :hunks="headDiff?.hunks ?? []"
          :runs="blame"
          :blame="diffMode.blame"
          :find="findMarks"
          :fallback="text"
          @text="(value: string) => (editorText = value)"
          @scroller="(element: HTMLElement | null) => (editorScroller = element)"
          @bar="toBar"
          @facts="(value: EditorFacts) => (facts = value)"
        />

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
          :title="'Previous (↑ or ⇧↵)'"
          @mousedown.prevent
          @click="step(-1)"
        >
          <ChevronUp :size="13" />
        </button>
        <button
          class="step"
          :disabled="!hits.length"
          :title="'Next (↓ or ↵)'"
          @mousedown.prevent
          @click="step(1)"
        >
          <ChevronDown :size="13" />
        </button>
        <button class="step" title="Close (Esc)" @mousedown.prevent @click="closeFind">
          <X :size="13" />
        </button>
      </div>
      <!-- ⌘G: a line to go to, in the same corner as the find box. -->
      <div v-if="asking" class="find goto">
        <span class="faint">Line</span>
        <input
          ref="lineBox"
          v-model="lineInput"
          type="text"
          inputmode="numeric"
          spellcheck="false"
          placeholder="Go to line"
          @keydown="onLineKey"
          @blur="asking = false"
        />
      </div>
      <ChangeRuler :container="editing ? editorScroller : body" :marks="marks" />
    </div>
  </section>
</template>

<style scoped>
/* The editor scrolls itself; the box only holds it. */
.body.editing {
  overflow: hidden;
}

.save-warning {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: none;
  padding: 1px 6px;
  font-size: 11px;
  line-height: 16px;
  white-space: nowrap;
  color: var(--danger);
  border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
  border-radius: var(--radius-sm);
}

.row-buttons {
  display: flex;
  gap: 8px;
}

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
  /* A bar with more on it than the pane is wide is cut at the pane's edge
     rather than drawn over the panel beside it. */
  min-width: 0;
  overflow: hidden;
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

/* A folder in the path is a way to it in the Files tree. */
.crumb {
  display: inline;
  padding: 0;
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
  cursor: pointer;
}

.crumb:hover {
  color: var(--text);
  text-decoration: underline;
}

.sep {
  padding: 0 1px;
  opacity: 0.6;
}

.type {
  display: inline-flex;
  flex: none;
}

/* What kind of file this is to git: new, ignored, read-only. */
.badge {
  flex: none;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  font-size: 11px;
  line-height: 16px;
  white-space: nowrap;
  color: var(--text-dim);
  background: var(--bg-raised);
  border: 1px solid var(--line);
}

.badge.new {
  color: var(--green-soft);
  border-color: color-mix(in srgb, var(--green-soft) 40%, transparent);
}

.badge.deleted,
.badge.conflict {
  color: var(--amber);
  border-color: color-mix(in srgb, var(--amber) 40%, transparent);
}

.badge.readonly,
.badge.ignored {
  color: var(--text-faint);
}

/* Where the caret is and how the file is written, quiet until wanted. */
.facts {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  margin-right: 4px;
  font-size: 11.5px;
  color: var(--text-faint);
  white-space: nowrap;
}

.fact {
  padding: 0;
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
}

button.fact {
  cursor: pointer;
  font-variant-numeric: tabular-nums;
}

button.fact:hover {
  color: var(--text);
}

.modes.off {
  opacity: 0.5;
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
