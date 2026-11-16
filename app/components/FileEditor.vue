<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { EditorState, Prec, Transaction, type ChangeSet, type EditorSelection } from '@codemirror/state'
import { EditorView, keymap, type ViewUpdate } from '@codemirror/view'
import {
  openSearchPanel,
  search as findPanel,
  searchKeymap,
  SearchQuery,
  setSearchQuery
} from '@codemirror/search'
import { TriangleAlert } from 'lucide-vue-next'
import CompareDialog from '~/components/CompareDialog.vue'
import ChangeCard from '~/components/ChangeCard.vue'
import { indentUnit } from '@codemirror/language'
import {
  indentOverrides,
  resolveIndent,
  type EditorconfigIndent,
  type Indent
} from '~/composables/indentation'
import {
  changeAt,
  changesOf,
  revertEdit,
  sameChange,
  stagePick,
  type Change
} from '~/composables/changeCard'
import { copyText, useGit } from '~/composables/useGit'
import type { BlameRun, DiffHunk } from '~/composables/useGit'
import type { FindMarks } from '~/composables/useFind'
import { version } from '~/composables/useHighlight'
import { matchesShortcut } from '~/composables/useShortcuts'
import { consumeLanding, findTab, pinTab, useTabs } from '~/composables/useTabs'
import {
  barLines,
  barsFor,
  basics,
  changeBars,
  editingBasics,
  editorKeys,
  findPosition,
  inlineBlame,
  editorLanguage,
  readOnly,
  rememberFocus,
  searchHits,
  setBars,
  setBlame,
  setFind,
  setHits,
  setLanding,
  setLenses,
  setUnused,
  syntaxColours,
  navigation,
  unusedCode,
  usageLenses,
  type LineHits,
  type UnusedMark,
  type UsageLens
} from '~/composables/useCodeMirror'
import { aimedAt, invokeAt } from '~/composables/useInvoke'
import { useConfig } from '~/composables/useConfig'
import { onCodeFont } from '~/composables/useCodeFont'
import { jumpFrom, nextSymbolRequest, showUsages } from '~/composables/useSymbols'
import {
  acquire,
  follow,
  inCharge,
  isStale,
  release,
  slots,
  workTreeEpoch,
  workTreeMoved,
  type Held,
  type Holder
} from '~/composables/editorBuffers'
import {
  drafts,
  editKey,
  flushing,
  isConflict,
  landed,
  leave,
  onFlush,
  readFile,
  setSaveError,
  setSaveState,
  writeFile,
  type FileRead,
  type Shape
} from '~/composables/useFileEdit'

/**
 * File mode: the file itself, in CodeMirror, saved as you type.
 *
 * Small fixes, not an IDE. It saves 800 ms after the last keystroke, when the
 * tab is left, when the window loses focus and when the app quits; ⌘S saves at
 * once. A save names the file it was based on, so a file an agent rewrote
 * meanwhile is never written over: the bar across the top asks which to keep.
 *
 * Read-only for a binary, an LFS pointer, anything not UTF-8 or over 2 MB, and
 * whatever `locked` says (a commit's copy).
 *
 * Its state outlives it (`editorBuffers.ts`): switching tabs puts it away
 * and coming back takes it up again, parsed, coloured, with its caret, scroll
 * and undo history, without reading the file. A second editor of the same
 * file borrows the state, and this one follows its edits and saves nothing
 * until it is given back.
 *
 * `preview` is the search window's pane: the same editor without a tab of its
 * own, every match marked, lines wrapped, and ⌘F opening CodeMirror's own find
 * bar inside the pane. Over a file open in a tab it borrows the tab's state,
 * so the two are one buffer.
 */

const props = defineProps<{
  tabId: string
  path: string
  /** The working tree against the last commit, for the change bars. */
  hunks: DiffHunk[]
  /** Who last touched each line, for the caret line, when blame is on. */
  runs: BlameRun[] | null
  blame: boolean
  find: FindMarks | null
  /** Text to show read-only when the file cannot be read for editing. */
  fallback: string | null
  locked?: boolean
  /** The search window's preview. */
  preview?: boolean
  /** For the preview: the line the search is on, to bring into view. */
  line?: number | null
  /** For the preview: every match in the file, to mark. */
  hits?: LineHits[]
  /** For the preview's ⌘F: what the search window is looking for. */
  query?: { search: string; caseSensitive: boolean; wholeWord: boolean; regexp: boolean } | null
}>()

const emit = defineEmits<{
  /** The text as it now stands, for the find box. */
  text: [string]
  /** "Show in Diff" on a change's card: the Diff view at this line. */
  bar: [number]
  /** The element that scrolls, for the strip beside it. */
  scroller: [HTMLElement | null]
  /** What reading the file said, for the header. */
  read: [FileRead | null]
  /** How the file is written and whether it can be edited, for the header. */
  facts: [EditorFacts]
}>()

export interface EditorFacts {
  /** Why it cannot be edited (`binary`, `lfs`, `encoding`, `large`, `endings`), or null. */
  readonly: string | null
  eol: 'lf' | 'crlf'
  bom: boolean
  /** How a new line and Tab indent here. */
  indent: Indent | null
}

/** Tells the header what it needs to know about the file. */
function tell() {
  emit('facts', {
    readonly: props.locked ? 'locked' : (reason.value === 'missing' || reason.value === 'unreadable' ? null : reason.value),
    eol: base.shape?.eol ?? 'lf',
    bom: base.shape?.bom ?? false,
    indent: indent.value
  })
}

const tabs = useTabs()
const config = useConfig()
/**
 * The repository this file is in, fixed when the editor opens. A save that
 * lands after the project was switched still goes here.
 */
const repo = aimedAt()
const key = editKey(repo, props.tabId)
const host = ref<HTMLElement | null>(null)
let view: EditorView | null = null
let alive = true

/** The file as it is on disk, which the edits are made on. */
const base = {
  text: '',
  hash: null as string | null,
  /** How the file was written, for writing it again if it is deleted. */
  shape: null as Shape | null
}
/** Why it cannot be edited, when it cannot. */
const reason = ref<string | null>(null)
const loading = ref(true)
/** What is on disk now, when that is not what the edits were made on. */
const conflict = ref<FileRead | null>(null)
/** Deleted on disk while open, with nothing typed since. */
const deleted = ref(false)
const comparing = ref(false)
/** The hash of the disk's copy the picker was opened on. */
let comparedHash = ''
/** The file changed again while it was being compared. */
const movedOn = ref(false)
const failure = ref<string | null>(null)
/** The save under way, so another waits for it rather than racing it. */
let inflight: Promise<void> | null = null
let timer: ReturnType<typeof setTimeout> | undefined

const editable = computed(() => !reason.value && !props.locked)

const { language, colours, access, indenting, bound } = slots
/** Lent to another editor of this file: this one saves nothing until it is back. */
let lent = false
/** The work tree's count of changes this editor has caught up with. */
let seenEpoch = 0

// --- indentation
//
// CodeMirror indents by two spaces unless told otherwise, so a four-space PHP
// file got six on a new line inside a method. The unit is worked out per file
// (`indentation.ts`): an override from the header, `.editorconfig`, the
// file's own whitespace, Settings → Editor, the language's custom.

const indent = ref<Indent | null>(null)
let editorconfig: EditorconfigIndent | null = null

function indentExtensions(now: Indent) {
  return [indentUnit.of(now.tabs ? '\t' : ' '.repeat(now.size)), EditorState.tabSize.of(now.tabWidth)]
}

function workOutIndent(text: string) {
  const settings = config.settings.value
  indent.value = resolveIndent({
    path: props.path,
    text,
    editorconfig,
    settings: { style: settings?.indent_style, size: settings?.indent_size, tabWidth: settings?.tab_width },
    override: indentOverrides[key] ?? null
  })
  view?.dispatch({ effects: indenting.reconfigure(indentExtensions(indent.value)) })
  tell()
}

const REASONS: Record<string, string> = {
  binary: 'Binary file — nothing to read.',
  lfs: 'Stored with Git LFS — this is the pointer, read-only.',
  encoding: 'Not UTF-8 — shown read-only, so a save cannot mangle it.',
  endings: 'Mixed line endings — shown read-only, so a save cannot change lines you did not touch.',
  large: 'Over 2 MB — shown read-only.',
  huge: 'Over 16 MB — too big to show here.',
  missing: 'Not on disk.',
  unreadable: 'Could not be opened for editing — shown read-only.'
}

/** Why a file as read cannot be edited. Past 16 MB not even its text is sent. */
function whyReadOnly(read: FileRead) {
  return read.readonly === 'large' && read.hash.startsWith('size:') ? 'huge' : read.readonly
}

const notice = computed(() => (props.locked ? null : reason.value ? REASONS[reason.value] : null))

function text() {
  return view?.state.doc.toString() ?? ''
}

// The header's "Not saved" chip says why in its tooltip.
watch(failure, (error) => setSaveError(repo, props.tabId, error))

function state(next: Parameters<typeof setSaveState>[2]) {
  setSaveState(repo, props.tabId, next)
}

/** The text as CodeMirror holds it: `\n` only. */
function normal(value: string) {
  return value.replace(/\r\n?/g, '\n')
}

function schedule() {
  clearTimeout(timer)
  timer = setTimeout(() => void save(), 800)
}

/**
 * Writes the text if it differs from the file. Refused because the file
 * changed underneath, it shows the bar instead and writes nothing more until
 * that is answered.
 */
async function save(): Promise<void> {
  clearTimeout(timer)
  // One write at a time: the next names the hash the last one left.
  while (inflight) await inflight
  if (lent || !view || conflict.value || !editable.value || base.hash === null) return
  const now = text()
  if (now === base.text) return
  inflight = write(now)
  try {
    await inflight
  } finally {
    inflight = null
  }
  if (view && !lent && !conflict.value && !failure.value && text() !== base.text) {
    state('unsaved')
    schedule()
  }
}

async function write(now: string) {
  state('saving')
  try {
    const hash = await writeFile(props.path, now, base.hash, repo)
    base.text = now
    base.hash = hash
    failure.value = null
    refreshSymbols()
  } catch (error) {
    if (isConflict(error)) {
      conflict.value = (await readFile(props.path, repo)) ?? missingRead()
    } else {
      failure.value = String(error)
    }
  }
  if (conflict.value) state('conflict')
  else if (failure.value) state('failed')
  else state(text() !== base.text ? 'unsaved' : 'saved')
}

function missingRead(): FileRead {
  return { text: '', hash: '', eol: 'lf', bom: false, readonly: null, missing: true }
}

/** Swaps the text for another, changing only what differs, so the caret stays. */
function replaceText(next: string, record = false) {
  if (!view) return
  const old = view.state.doc.toString()
  if (old === next) return
  let start = 0
  const short = Math.min(old.length, next.length)
  while (start < short && old[start] === next[start]) start++
  let end = 0
  while (
    end < short - start &&
    old[old.length - 1 - end] === next[next.length - 1 - end]
  ) {
    end++
  }
  const top = view.scrollDOM.scrollTop
  view.dispatch({
    changes: { from: start, to: old.length - end, insert: next.slice(start, next.length - end) },
    annotations: record ? [] : [Transaction.addToHistory.of(false)]
  })
  view.scrollDOM.scrollTop = top
}

/** Something on disk moved: is it this file, and what does it mean here? */
async function checkDisk() {
  if (lent || !view || inflight || base.hash === null) return
  seenEpoch = workTreeEpoch()
  const before = base.hash
  const read = await readFile(props.path, repo)
  // A save landed meanwhile: this read may be from before it, and taking it
  // would put the text back to what it was.
  if (!read || lent || !view || inflight || base.hash !== before) return
  const dirty = text() !== base.text
  if (read.missing) {
    if (dirty) {
      conflict.value = read
      state('conflict')
    } else deleted.value = true
    return
  }
  deleted.value = false
  // Our own save, or a write that left it as it was.
  if (read.hash === base.hash) {
    // Back to the file the edits were made on: nothing is in the way of a
    // save any more, so the bar goes and the edits are saved as usual.
    if (conflict.value) {
      conflict.value = null
      comparing.value = false
      state(dirty ? 'unsaved' : 'saved')
      if (dirty) schedule()
    }
    return
  }
  if (!dirty && !conflict.value) {
    takeTheirs(read)
    return
  }
  // Changed again while the two were being compared: the picker shows a file
  // that is not on disk any more, so it starts over from the one that is.
  if (comparing.value && conflict.value && conflict.value.hash !== read.hash) {
    comparing.value = false
    movedOn.value = true
  }
  conflict.value = read
  state('conflict')
}

/**
 * The file as it is on disk replaces the edits. `undoable` is the button:
 * what was typed goes, but ⌘Z brings it back. A quiet reload of a file
 * nobody was editing is not a step in the history.
 */
function takeTheirs(read: FileRead, undoable = false) {
  base.text = normal(read.text)
  base.hash = read.hash
  base.shape = { eol: read.eol, bom: read.bom }
  conflict.value = null
  reason.value = whyReadOnly(read)
  tell()
  view?.dispatch({ effects: access.reconfigure(readOnly(!editable.value)) })
  replaceText(base.text, undoable)
  // Replacing the text is a change like any other and marked it unsaved.
  clearTimeout(timer)
  state(editable.value ? 'saved' : 'readonly')
  refreshSymbols()
}

/** The edits are written over what is on disk now. */
async function keepMine() {
  const theirs = conflict.value
  if (!theirs || !view) return
  base.hash = theirs.missing ? null : theirs.hash
  base.text = normal(theirs.text)
  conflict.value = null
  if (theirs.missing) {
    // Deleted underneath: writing it again is a new file.
    await keepMineAsNew()
    return
  }
  await save()
  // Mine was the same as theirs: nothing to write, and nothing in conflict.
  if (!conflict.value && !failure.value && text() === base.text) state('saved')
}

/** The side-by-side picker settled on this text, over what is on disk now. */
async function compared(result: string) {
  const theirs = conflict.value
  comparing.value = false
  if (!theirs || !view) return
  // The result was put together from the file the picker showed. If the disk
  // has moved on since, saving it would write over changes nobody was shown.
  if (theirs.hash !== comparedHash) {
    movedOn.value = true
    return
  }
  base.text = normal(theirs.text)
  base.hash = theirs.missing ? null : theirs.hash
  conflict.value = null
  replaceText(result, true)
  if (base.hash === null) {
    await keepMineAsNew()
    return
  }
  await save()
  if (!conflict.value && !failure.value && text() === base.text) state('saved')
}

/** Opens the side-by-side picker on the file as it is on disk now. */
function openCompare() {
  if (!conflict.value) return
  comparedHash = conflict.value.hash
  movedOn.value = false
  comparing.value = true
}

async function keepMineAsNew() {
  try {
    // What is written is what is on disk: anything typed while the write is
    // on its way is still unsaved after it.
    const written = text()
    base.hash = await writeFile(props.path, written, null, repo, base.shape)
    base.text = written
    failure.value = null
    if (text() !== written) {
      state('unsaved')
      schedule()
    } else state('saved')
  } catch (error) {
    // Not written: with no hash nothing else would ever save these edits.
    // Back to the bar, asking about the file as it is now.
    const now = (await readFile(props.path, repo)) ?? missingRead()
    conflict.value = now
    if (!now.missing) base.hash = now.hash
    failure.value = String(error)
    state('conflict')
  }
}

// --- moving about the file

/** Puts the caret on a line and brings it to the middle, marked for a moment. */
// --- the card a change bar opens
//
// Clicking a bar used to switch the tab to Diff mode. It opens a card under
// the change instead, showing what was there before, without leaving the
// editor. Read from the same hunks as the bars: the work tree against HEAD.

const git = useGit()
const changes = computed(() => changesOf(props.hunks))
const card = ref<Change | null>(null)
const cardBox = ref<HTMLElement | null>(null)
const pane = ref<HTMLElement | null>(null)
/** Where the card sits in the pane: top and left in px, and its width. */
const cardAt = ref<{ top: number; left: number; width: number } | null>(null)

function openCard(line: number) {
  const found = changeAt(changes.value, line)
  if (!found) return
  card.value = found
  void placeCard()
}

function closeCard() {
  card.value = null
  cardAt.value = null
}

/** The previous or next change, wrapping round; the card and the view follow. */
function moveCard(by: number) {
  const all = changes.value
  if (!card.value || !all.length || !view) return
  const next = all[(card.value.index + by + all.length) % all.length]!
  card.value = next
  const line = Math.min(Math.max(1, next.from), view.state.doc.lines)
  view.dispatch({ effects: EditorView.scrollIntoView(view.state.doc.line(line).from, { y: 'center' }) })
  void placeCard()
}

/** Under the change's last line, or over its first when there is no room below. */
async function placeCard() {
  await new Promise((done) => requestAnimationFrame(() => done(null)))
  const change = card.value
  if (!change || !view || !pane.value) return
  const doc = view.state.doc
  const first = doc.line(Math.min(Math.max(1, change.from), doc.lines))
  const last = doc.line(Math.min(Math.max(1, change.to), doc.lines))
  const paneBox = pane.value.getBoundingClientRect()
  const content = view.contentDOM.getBoundingClientRect()
  const scroller = view.scrollDOM.getBoundingClientRect()
  const below = view.coordsAtPos(last.to)
  const above = view.coordsAtPos(first.from)
  // Scrolled out of what is drawn: there is no line to hang the card on, and
  // left where it was it would sit over code it is not about.
  const edge = view.scrollDOM.getBoundingClientRect()
  const drawn = view.viewport
  const off = (at: number, box: { top: number; bottom: number } | null) =>
    at < drawn.from || at > drawn.to || (!!box && (box.bottom < edge.top || box.top > edge.bottom))
  if (off(last.to, below) && off(first.from, above)) {
    closeCard()
    return
  }
  const height = cardBox.value?.offsetHeight || 160
  const left = content.left - paneBox.left
  const width = Math.max(240, Math.min(content.width, scroller.right - content.left - 12))
  let top = (below?.bottom ?? scroller.top) - paneBox.top + 2
  if (top + height > paneBox.height && above) top = Math.max(0, above.top - paneBox.top - height - 2)
  cardAt.value = { top, left, width }
}

// The hunks are read again after every save. The card follows its change
// there, and goes once the change is not there any more.
watch(changes, (now) => {
  if (!card.value) return
  const same = sameChange(now, card.value)
  if (!same) closeCard()
  else {
    card.value = same
    void placeCard()
  }
})

function revertCard() {
  const change = card.value
  if (!change || !view || !editable.value) return
  // The card's lines are the saved file's. With edits not saved yet they may
  // have moved, and the old text would land on other lines.
  if (text() !== base.text) {
    closeCard()
    void save()
    git.note('Saved your edits first. Open the change again to revert it.')
    return
  }
  const edit = revertEdit(change, view.state.doc)
  if (!edit) return
  // One transaction: one step for ⌘Z, and the usual autosave after it.
  view.dispatch({ changes: edit, userEvent: 'input.revert', scrollIntoView: true })
  closeCard()
}

async function stageCard() {
  const change = card.value
  if (!change || !editable.value) return
  // What is staged is what is on disk, so the edits go there first.
  await save()
  const unstaged = await git.workingFileDiff(props.path, 'unstaged')
  const pick = unstaged ? stagePick(unstaged.hunks, change) : null
  if (!pick) {
    git.note(`Nothing left to stage there: it is staged already`)
    return
  }
  await git.applyHunk(props.path, pick.hunk, 'stage', pick.lines)
  closeCard()
}

function copyCard() {
  if (card.value) void copyText(card.value.removed.join('\n'), 'Old text')
}

function diffCard() {
  const change = card.value
  closeCard()
  if (change) emit('bar', change.from)
}

/** Esc closes it, before anything else hears the key. */
function onCardKey(event: KeyboardEvent) {
  if (event.key !== 'Escape' || !card.value) return
  event.preventDefault()
  event.stopImmediatePropagation()
  closeCard()
}

/** A click anywhere but the card closes it; a bar opens the next one itself. */
function onCardPointer(event: PointerEvent) {
  if (!card.value) return
  const target = event.target as Node | null
  if (target && cardBox.value?.contains(target)) return
  closeCard()
}

let cardSizer: ResizeObserver | null = null

function onCardScroll() {
  if (card.value) void placeCard()
}

watch(card, (open, was) => {
  if (open && !was) {
    window.addEventListener('keydown', onCardKey, true)
    document.addEventListener('pointerdown', onCardPointer, true)
    view?.scrollDOM.addEventListener('scroll', onCardScroll, { passive: true })
    // The window, a panel or the bar over the editor changing its box.
    if (pane.value && typeof ResizeObserver !== 'undefined') {
      cardSizer = new ResizeObserver(() => onCardScroll())
      cardSizer.observe(pane.value)
    }
  } else if (!open && was) {
    cardSizer?.disconnect()
    cardSizer = null
    window.removeEventListener('keydown', onCardKey, true)
    document.removeEventListener('pointerdown', onCardPointer, true)
    view?.scrollDOM.removeEventListener('scroll', onCardScroll)
  }
})

function goToLine(line: number, mark = true) {
  if (!view) return
  const number = Math.min(Math.max(1, Math.round(line)), view.state.doc.lines)
  const at = view.state.doc.line(number).from
  view.dispatch({
    selection: { anchor: at },
    effects: [EditorView.scrollIntoView(at, { y: 'center' }), setLanding.of(mark ? number : null)]
  })
  view.focus()
  if (mark) {
    setTimeout(() => view?.dispatch({ effects: setLanding.of(null) }), 1200)
  }
}

/** The next or previous change after the caret, wrapping round. */
function nextChange(by: number) {
  if (!view) return
  if (card.value) {
    moveCard(by)
    return
  }
  const lines = barLines(view)
  if (!lines.length) return
  const here = view.state.doc.lineAt(view.state.selection.main.head).number
  const target =
    by > 0
      ? (lines.find((line) => line > here) ?? lines[0]!)
      : ([...lines].reverse().find((line) => line < here) ?? lines[lines.length - 1]!)
  goToLine(target, false)
}

/** Where the first change is, for a file just opened from a list of changes. */
function toFirstChange() {
  if (!view) return
  const first = barLines(view)[0]
  if (first === undefined) return
  const at = view.state.doc.line(Math.min(first, view.state.doc.lines)).from
  view.dispatch({ effects: EditorView.scrollIntoView(at, { y: 'start', yMargin: 72 }) })
}

function focus() {
  view?.focus()
}

// --- names: ⌘-click, unused code, usage counts

/** The languages the backend reads names from (symbols.rs). */
const NAMED = /\.(php|js|mjs|cjs|jsx|ts|mts|cts|tsx|vue|rs|py|go)$/i

function named() {
  return NAMED.test(props.path) && !/\.(blade\.php|d\.ts)$/i.test(props.path)
}

/** ⌘J: what ⌘-click does, at the caret. */
function jumpAtCaret() {
  if (!view || !named()) return
  const head = view.state.selection.main.head
  const box = view.coordsAtPos(head)
  void jumpFrom(props.path, head, text(), box ? { x: box.left, y: box.bottom - 18 } : null)
}

let symbolSeq = 0
let symbolTimer: ReturnType<typeof setTimeout> | undefined

/** Asks again what nothing uses and how often the rest is used. */
function refreshSymbols(delay = 300) {
  clearTimeout(symbolTimer)
  symbolTimer = setTimeout(() => void loadSymbols(), delay)
}

async function loadSymbols() {
  // A dependency is not counted, and nothing in it is dimmed: what uses it is
  // mostly outside this repository.
  if (lent || !view || !named() || props.locked || props.preview || reason.value === 'dependency') return
  const mine = ++symbolSeq
  const asked = text()
  const counts = config.settings.value?.usage_counts !== false
  const [marks, lenses] = await Promise.all([
    invokeAt<UnusedMark[]>(repo, 'unused_in', { path: props.path, text: asked, request: nextSymbolRequest() }).catch(() => []),
    counts
      ? invokeAt<UsageLens[]>(repo, 'usage_counts', { path: props.path, text: asked }).catch(() => [])
      : Promise.resolve([] as UsageLens[])
  ])
  // Typed over since: the offsets are for a text that is gone. The next save
  // asks again.
  if (mine !== symbolSeq || lent || !view || text() !== asked) return
  view.dispatch({ effects: [setUnused.of(marks ?? []), setLenses.of(lenses ?? [])] })
}

defineExpose({ goToLine, nextChange, focus, focusAt, save, text, jumpAtCaret, openCard, closeCard })

// --- building the view

function onUpdate(update: ViewUpdate) {
  if (lent) return
  if (update.docChanged) {
    // The editor this was borrowed from shows the edit too.
    follow(repo, props.tabId, holder, update.changes)
    // Typing in a preview tab keeps it. Not typing in the search window's
    // preview: that opens no tab.
    const typed = update.transactions.some((tr) => tr.annotation(Transaction.userEvent) !== undefined)
    if (typed && !props.preview) {
      const tab = tabs.state.tabs.find((one) => one.id === props.tabId)
      if (tab?.preview) pinTab(tab.id)
    }
    const now = update.state.doc.toString()
    emit('text', now)
    // The card stays while you type elsewhere, and follows its lines. Revert
    // and Stage look at the saved text before they act, so a card that has
    // gone stale cannot land on the wrong lines.
    if (card.value) void placeCard()
    if (editable.value && !conflict.value && base.hash !== null) {
      const dirty = now !== base.text
      // Back to what is on disk: there is nothing left that could not be saved.
      if (!dirty) failure.value = null
      // A write on its way says "Saving…" until it lands, and settles the
      // state itself then.
      if (!inflight) state(dirty ? 'unsaved' : 'saved')
      if (dirty) schedule()
    }
  }
  if (update.selectionSet && !props.preview) {
    const tab = tabs.state.tabs.find((one) => one.id === props.tabId)
    if (tab) {
      const head = update.state.selection.main.head
      tab.caret = head
      tab.line = update.state.doc.lineAt(head).number
    }
  }
}

function showBars() {
  if (!view) return
  view.dispatch({ effects: setBars.of(barsFor(text(), props.hunks)) })
}

function showBlame() {
  view?.dispatch({ effects: setBlame.of(props.blame ? props.runs : null) })
}

let lastFind = ''

function showFind() {
  if (!view) return
  const find = props.find
  const effects = [setFind.of(find)]
  const key = find ? `${find.query}|${find.matchCase}|${find.row}|${find.nth}` : ''
  if (key !== lastFind) {
    lastFind = key
    const at = findPosition(view, find)
    if (at) {
      const fits = view.visibleRanges.some((range) => at.from >= range.from && at.to <= range.to)
      if (!fits) effects.push(EditorView.scrollIntoView(at.from, { y: 'center' }) as never)
    }
  }
  view.dispatch({ effects })
}

// The code changed size: the caret's line stays where it was on screen.
const dropFontKeeper = onCodeFont(() => {
  if (!view) return
  const head = view.state.selection.main.head
  const offset = view.lineBlockAt(head).top - view.scrollDOM.scrollTop
  return () => {
    if (!view) return
    view.requestMeasure()
    view.dispatch({ effects: EditorView.scrollIntoView(head, { y: 'start', yMargin: Math.max(0, offset) }) })
  }
})

let unlisten: UnlistenFn | undefined
let dropFlush: (() => void) | undefined
let checkTimer: ReturnType<typeof setTimeout> | undefined

function onBlur() {
  void save()
}

/**
 * What calls back into this component: swapped in whenever a state comes
 * from another editor, so nothing in it points at one that has gone.
 */
function own() {
  return [
    changeBars((line) => openCard(line)),
    navigation((pos, event) => {
      if (named()) void jumpFrom(props.path, pos, text(), { x: event.clientX, y: event.clientY })
    }),
    usageLenses((lens, event) =>
      void showUsages(lens.name, props.path, lens.kind, { x: event.clientX, y: event.clientY }, lens.line)
    ),
    EditorView.updateListener.of(onUpdate),
    EditorView.domEventHandlers({
      keydown(event) {
        if (matchesShortcut(event, 'file.save')) {
          event.preventDefault()
          void save()
          return true
        }
        return false
      },
      focus(_event, editor) {
        rememberFocus(editor)
        return false
      },
      blur() {
        void save()
        return false
      }
    }),
    ...(props.preview ? previewOnly() : [])
  ]
}

/** What the search window's preview has that a tab's editor does not. */
function previewOnly() {
  return [
    searchHits,
    EditorView.lineWrapping,
    findPanel({ top: true }),
    Prec.high(
      keymap.of([{ key: 'Mod-f', run: findHere }, ...searchKeymap.filter((one) => one.key !== 'Mod-f')])
    ),
    findPanelLook
  ]
}

/** ⌘F in the preview: CodeMirror's find bar, inside the pane, on the window's query. */
function findHere(editor: EditorView) {
  openSearchPanel(editor)
  const query = props.query
  if (query?.search) {
    editor.dispatch({ effects: setSearchQuery.of(new SearchQuery({ ...query })) })
    editor.dom.querySelector<HTMLInputElement>('.cm-search input[name="search"]')?.select()
  }
  return true
}

/** The find bar in the app's colours rather than CodeMirror's. */
const findPanelLook = EditorView.theme({
  '.cm-panels': { backgroundColor: 'var(--bg-raised)', color: 'var(--text)' },
  '.cm-panels-top': { borderBottom: '1px solid var(--line-soft)' },
  '.cm-search': { fontSize: '12px', padding: '4px 8px', display: 'flex', flexWrap: 'wrap', gap: '4px', alignItems: 'center' },
  '.cm-search input, .cm-search button': { fontSize: '12px', margin: '0' },
  '.cm-search .cm-textfield': {
    background: 'var(--bg)',
    color: 'var(--text)',
    border: '1px solid var(--line)',
    borderRadius: 'var(--radius-sm)',
    padding: '2px 6px'
  },
  '.cm-search .cm-button': {
    background: 'var(--bg)',
    backgroundImage: 'none',
    color: 'var(--text)',
    border: '1px solid var(--line)',
    borderRadius: 'var(--radius-sm)',
    padding: '1px 8px'
  },
  '.cm-search label': { color: 'var(--text-dim)' },
  '.cm-search [name="close"]': { color: 'var(--text-dim)' }
})

/** The preview's marks: every match in the file, the one the search is on brighter. */
function showHits() {
  if (!view || !props.preview) return
  view.dispatch({ effects: setHits.of({ hits: props.hits ?? [], now: props.line ?? null }) })
}

/** The preview's line in the middle of the pane, picked out; the caret stays where it is. */
function center() {
  if (!view || !props.preview || !props.line) return
  const number = Math.min(Math.max(1, props.line), view.state.doc.lines)
  const at = view.state.doc.line(number).from
  view.dispatch({ effects: [EditorView.scrollIntoView(at, { y: 'center' }), setLanding.of(number)] })
}

/** Puts the caret on the search's line, at its first match, and the focus in the editor. */
function focusAt(line: number | null, column = 0) {
  if (!view) return
  if (line) {
    const number = Math.min(Math.max(1, line), view.state.doc.lines)
    const at = view.state.doc.line(number)
    const pos = at.from + Math.min(column, at.length)
    view.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: 'center' }) })
  }
  view.focus()
}

// --- kept warm, and handed between editors (editorBuffers.ts)

/** What another editor needs to carry on from here. Null when it should read the file itself. */
function snapshot(): Held | null {
  if (!view || lent || props.locked || deleted.value) return null
  if (reason.value === 'missing' || reason.value === 'unreadable') return null
  const held: Held = {
    path: props.path,
    state: view.state,
    // The preview's place is the search's, not the tab's: the tab goes back
    // to where it was.
    scroll: props.preview ? null : view.scrollSnapshot(),
    base: { ...base },
    reason: reason.value,
    editorconfig,
    colours: version.value,
    epoch: seenEpoch,
    settle: null
  }
  // A save on its way: the hash it leaves is the one the next save names.
  const writing = inflight
  if (writing) {
    held.settle = writing.then(
      () => {
        held.base = { ...base }
      },
      () => undefined
    )
  }
  return held
}

/** Where this editor was when it lent its state, to go back to. */
let lentScroll: ReturnType<EditorView['scrollSnapshot']> | null = null

const holder: Holder = {
  sticky: !!props.preview,
  peek: () => snapshot(),
  lend() {
    const held = snapshot()
    lentScroll = view?.scrollSnapshot() ?? null
    clearTimeout(timer)
    clearTimeout(checkTimer)
    clearTimeout(symbolTimer)
    closeCard()
    lent = true
    view?.dispatch({ effects: access.reconfigure(readOnly(true)) })
    return held
  },
  reclaim(held) {
    lent = false
    if (!view) return
    if (!held || held.path !== props.path) {
      view.dispatch({ effects: access.reconfigure(readOnly(!editable.value)) })
      return
    }
    // The caret where it was here, carried through the edits made meanwhile.
    const selection = view.state.doc.eq(held.state.doc) ? view.state.selection : null
    void adopt({ ...held, scroll: lentScroll }, true, selection)
    lentScroll = null
  },
  follow(changes: ChangeSet) {
    // Only onto the text the change was made on: a state this editor did not
    // lend (the other one read the file itself) is not followed.
    if (!view || !lent || changes.length !== view.state.doc.length) return
    view.dispatch({ changes, annotations: [Transaction.addToHistory.of(false)] })
  }
}

/**
 * Carries on from a state another editor left: no read, no parse, no
 * colouring. Checks the disk only when something there moved meanwhile.
 * `back` is this editor taking back what it lent, which saves what was typed
 * in the other one straight away.
 */
async function adopt(held: Held, back = false, selection: EditorSelection | null = null) {
  // Only waited for when there is something to wait for, so the usual way
  // back draws on the first frame.
  if (held.settle) await held.settle
  const pending = flushing.get(key)
  if (pending) await pending
  if (!alive || !host.value) return
  base.text = held.base.text
  base.hash = held.base.hash
  base.shape = held.base.shape
  reason.value = held.reason
  editorconfig = held.editorconfig
  const doc = held.state.doc.toString()
  // A follower leaves the drafts and saves to the editor in charge.
  const draft = lent ? undefined : drafts.get(key)
  const wrote = lent ? undefined : landed.get(key)
  if (!lent) landed.delete(key)
  if (draft && draft.path === props.path) {
    // Left behind and refused: the bar asks about it again on the save below.
    drafts.delete(key)
    base.text = draft.baseText
    base.hash = draft.baseHash
    base.shape = draft.shape ?? base.shape
  } else if (wrote && wrote.text === doc) {
    // Saved after it was put away.
    base.text = wrote.text
    base.hash = wrote.hash
  }
  loading.value = false
  const effects = [bound.reconfigure(own()), access.reconfigure(readOnly(lent || !editable.value))]
  if (held.colours !== version.value) effects.push(colours.reconfigure(syntaxColours()))
  const next = held.state.update({ effects, selection: selection ?? undefined }).state
  // The preview goes to the search's line, not to where the tab was.
  const scroll = props.preview ? null : held.scroll
  if (view) {
    view.setState(next)
    if (scroll) view.dispatch({ effects: scroll })
  } else {
    view = new EditorView({ parent: host.value, state: next, scrollTo: scroll ?? undefined })
  }
  // Put away by the preview: the tab's own caret and place.
  const tab = props.preview ? null : tabs.state.tabs.find((one) => one.id === props.tabId)
  if (!scroll && tab && (tab.scrollTop !== undefined || tab.caret !== undefined)) {
    view.dispatch({ selection: { anchor: Math.min(tab.caret ?? 0, view.state.doc.length) } })
    const top = tab.scrollTop ?? 0
    requestAnimationFrame(() => {
      if (view) view.scrollDOM.scrollTop = top
    })
  }
  seenEpoch = held.epoch
  workOutIndent(doc)
  emit('scroller', view.scrollDOM)
  emit('text', doc)
  showFind()
  // Following the search window's preview, which saves: nothing to do here
  // until it hands the file over.
  if (lent) return
  if (draft && draft.path === props.path) {
    if (draft.text !== doc) replaceText(draft.text, true)
    state('unsaved')
    void save()
  } else if (editable.value && text() !== base.text) {
    state('unsaved')
    if (back) void save()
    else schedule()
  } else state(editable.value ? 'saved' : 'readonly')
  const landing = tabs.state.landing
  if (props.preview) {
    showHits()
    center()
  } else if (landing && landing.tab === props.tabId) {
    goToLine(landing.line)
    consumeLanding(landing.seq)
  }
  if (isStale(held)) void checkDisk()
}

async function start() {
  const held = props.locked ? null : acquire(repo, props.tabId, holder)
  // Opened under the search window's preview of the same file: it follows.
  if (!props.locked && !inCharge(repo, props.tabId, holder)) lent = true
  if (held && held.path === props.path) {
    await adopt(held)
    return
  }
  // Asked alongside the read: what .editorconfig says about indenting it.
  const configAsk = invokeAt<EditorconfigIndent | null>(repo, 'indent_config', { path: props.path }).catch(
    () => null
  )
  const pending = flushing.get(key)
  if (pending) await pending
  // Gone while that save ran: the draft stays for the editor that comes next.
  if (!alive) return
  let doc = ''
  const draft = drafts.get(key)
  if (draft && draft.path === props.path && !props.locked && !lent) {
    drafts.delete(key)
    base.text = draft.baseText
    base.hash = draft.baseHash
    base.shape = draft.shape ?? null
    doc = draft.text
    emit('read', null)
  } else if (props.locked) {
    doc = props.fallback ?? ''
  } else {
    const read = await readFile(props.path, repo)
    if (!alive) return
    emit('read', read)
    if (read && !read.missing) {
      base.text = normal(read.text)
      base.hash = read.hash
      base.shape = { eol: read.eol, bom: read.bom }
      reason.value = whyReadOnly(read)
      doc = base.text
    } else {
      reason.value = read?.missing ? 'missing' : 'unreadable'
      doc = props.fallback ?? ''
    }
  }
  seenEpoch = workTreeEpoch()
  editorconfig = await configAsk
  if (!alive || !host.value) return
  loading.value = false
  workOutIndent(doc)

  view = new EditorView({
    parent: host.value,
    state: EditorState.create({
      doc,
      extensions: [
        basics(),
        editingBasics(),
        editorKeys([]),
        inlineBlame,
        unusedCode,
        language.of([]),
        colours.of(syntaxColours()),
        access.of(readOnly(lent || !editable.value)),
        indenting.of(indentExtensions(indent.value!)),
        bound.of(own())
      ]
    })
  })
  emit('scroller', view.scrollDOM)
  emit('text', doc)
  showBars()
  showBlame()
  showFind()
  if (lent) {
    // Following another editor of the file, which says how it stands.
  } else if (draft && draft.path === props.path) {
    state('unsaved')
    void save()
  } else state(editable.value ? 'saved' : 'readonly')

  // Where to start: a jump that brought us here, then where the tab was
  // left, then the first change.
  const tab = tabs.state.tabs.find((one) => one.id === props.tabId)
  const landing = tabs.state.landing
  if (props.preview) {
    showHits()
    center()
  } else if (landing && landing.tab === props.tabId) {
    goToLine(landing.line)
    consumeLanding(landing.seq)
  } else if (tab?.scrollTop !== undefined || tab?.caret !== undefined) {
    const caret = Math.min(tab.caret ?? 0, view.state.doc.length)
    view.dispatch({ selection: { anchor: caret } })
    const top = tab.scrollTop ?? 0
    requestAnimationFrame(() => {
      if (view) view.scrollDOM.scrollTop = top
    })
  } else toFirstChange()

  void editorLanguage(props.path).then((support) => {
    if (support && view) view.dispatch({ effects: language.reconfigure(support) })
  })
  refreshSymbols(0)
}

onMounted(async () => {
  window.addEventListener('blur', onBlur)
  dropFlush = onFlush(() => save())
  void start()
  if (props.locked) return
  const off = await listen<{ work_tree: boolean }>('repo-changed', ({ payload }) => {
    if (!payload.work_tree) return
    workTreeMoved()
    if (lent) return
    clearTimeout(checkTimer)
    checkTimer = setTimeout(() => void checkDisk(), 120)
  }).catch(() => undefined)
  if (!alive) off?.()
  else unlisten = off
})

onBeforeUnmount(() => {
  alive = false
  dropFontKeeper()
  window.removeEventListener('blur', onBlur)
  window.removeEventListener('keydown', onCardKey, true)
  document.removeEventListener('pointerdown', onCardPointer, true)
  view?.scrollDOM.removeEventListener('scroll', onCardScroll)
  cardSizer?.disconnect()
  dropFlush?.()
  unlisten?.()
  clearTimeout(timer)
  clearTimeout(checkTimer)
  clearTimeout(symbolTimer)
  if (!view) {
    release(repo, props.tabId, holder, null)
    return
  }
  // In this file's project, which may no longer be the one on screen.
  const tab = findTab(repo, props.tabId)
  if (tab && !lent) {
    tab.scrollTop = view.scrollDOM.scrollTop
    tab.caret = view.state.selection.main.head
  }
  // Kept warm for the tab, or handed back to the editor it was borrowed
  // from, which then owns the edits and saves them itself.
  const wasLent = lent
  const held = tab ? snapshot() : null
  const handed = release(repo, props.tabId, holder, held) === 'handed'
  if (wasLent || handed) {
    rememberFocus(null)
    view.destroy()
    view = null
    emit('scroller', null)
    return
  }
  // Leaving with edits not on disk: they go on saving without the editor,
  // and wait here if the save is refused.
  // A save still under way finishes first, and the draft is made from the
  // hash it leaves, so the two never race into a false conflict.
  const now = text()
  // A file deleted underneath has no hash; its edits are still kept, as a
  // draft whose save is refused until the editor comes back and asks.
  const canSave = editable.value && (base.hash !== null || !!conflict.value)
  if (canSave && (now !== base.text || conflict.value || inflight)) {
    const path = props.path
    const stuck = !!conflict.value
    leave(
      repo,
      props.tabId,
      () =>
        now !== base.text || stuck || conflict.value
          ? {
              repo,
              tab: props.tabId,
              path,
              text: now,
              baseHash: base.hash ?? '',
              baseText: base.text,
              shape: base.shape
            }
          : null,
      inflight
    )
  }
  rememberFocus(null)
  view.destroy()
  view = null
  emit('scroller', null)
})

// Shown from the text the viewer read: it may land after the editor did.
watch(
  () => props.fallback,
  (value) => {
    if (view && value !== null && (props.locked || reason.value === 'unreadable')) {
      replaceText(value)
      showBars()
    }
  }
)
watch(() => props.hunks, showBars)
watch(
  () => [
    indentOverrides[key],
    config.settings.value?.indent_style,
    config.settings.value?.indent_size,
    config.settings.value?.tab_width
  ],
  () => {
    if (view) workOutIndent(base.text || text())
  },
  { deep: true }
)
watch(() => config.settings.value?.usage_counts, () => refreshSymbols(0))
watch(() => [props.runs, props.blame], showBlame)
watch(() => props.find, showFind, { deep: true })
// More matches arriving while a search runs are marked where the view is: it
// does not go back to the line under whoever is reading.
watch(() => props.hits, showHits)
watch(
  () => props.line,
  () => {
    showHits()
    center()
  }
)
watch(version, () => view?.dispatch({ effects: colours.reconfigure(syntaxColours()) }))
watch(
  () => tabs.state.landing,
  (landing) => {
    if (landing && landing.tab === props.tabId && view && !props.preview) {
      goToLine(landing.line)
      consumeLanding(landing.seq)
    }
  }
)
</script>

<template>
  <div ref="pane" class="editor-pane">
    <div v-if="conflict" class="disk-bar" role="alert">
      <TriangleAlert :size="14" />
      <span class="what">
        {{
          conflict.missing
            ? 'Deleted on disk while you were editing it.'
            : movedOn
              ? 'Changed on disk again while you were comparing. Compare shows it as it is now.'
              : 'Changed on disk while you were editing it.'
        }}
      </span>
      <button class="btn tiny" @click="keepMine">Keep mine</button>
      <button v-if="!conflict.missing" class="btn tiny" @click="takeTheirs(conflict, true)">
        Take theirs
      </button>
      <button v-if="!conflict.missing" class="btn tiny" @click="openCompare">Compare</button>
    </div>
    <p v-else-if="deleted" class="note">Deleted on disk. What is shown is what it held.</p>
    <p v-else-if="failure" class="note bad">Could not save: {{ failure }}</p>
    <p v-if="notice" class="note">{{ notice }}</p>
    <p v-if="loading" class="note">Loading file…</p>
    <div ref="host" class="host" :class="{ hidden: reason === 'binary' }" />

    <div
      v-if="card"
      ref="cardBox"
      class="card-slot"
      :style="cardAt ? { top: `${cardAt.top}px`, left: `${cardAt.left}px`, width: `${cardAt.width}px` } : { visibility: 'hidden' }"
    >
      <ChangeCard
        :path="props.path"
        :change="card"
        :count="changes.length"
        :editable="editable"
        @move="moveCard"
        @revert="revertCard"
        @stage="stageCard"
        @copy="copyCard"
        @diff="diffCard"
        @close="closeCard"
      />
    </div>

    <CompareDialog
      v-if="comparing && conflict"
      :path="props.path"
      :mine="text()"
      :theirs="conflict.text"
      @done="compared"
      @close="comparing = false"
    />
  </div>
</template>

<style scoped>
.editor-pane {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.host {
  flex: 1;
  min-height: 0;
}

/* The card over the editor, placed under the change it is about. */
.card-slot {
  position: absolute;
  z-index: 20;
}

.host.hidden {
  display: none;
}

.host :deep(.cm-editor) {
  height: 100%;
}

.disk-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  background: var(--warning-bg);
  border-bottom: 1px solid var(--warning-line);
  color: var(--text);
  font-size: 12.5px;
}

.disk-bar .what {
  flex: 1;
}

.note {
  margin: 0;
  padding: 6px 12px;
  font-size: 12px;
  color: var(--text-dim);
  border-bottom: 1px solid var(--line-soft);
}

.note.bad {
  color: var(--danger);
}
</style>
