<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import {
  CaseSensitive,
  ChevronRight,
  EyeOff,
  Folder,
  Funnel,
  GitCompare,
  Package,
  Regex,
  Search,
  TriangleAlert,
  WholeWord,
  X
} from 'lucide-vue-next'
import { copyText, useGit } from '~/composables/useGit'
import CodePreview from '~/components/CodePreview.vue'
import FileIcon from '~/components/FileIcon.vue'
import ResizeHandle from '~/components/ResizeHandle.vue'
import { change } from '~/composables/useFileView'
import { usePanes } from '~/composables/usePanes'
import { keyLabel, keysFor, matchesShortcut } from '~/composables/useShortcuts'
import { openTab, tabId } from '~/composables/useTabs'
import { aimedAt, invoke } from '~/composables/useInvoke'
import { revealInFiles, type DirEntry } from '~/composables/useFileTree'
import { useToasts } from '~/composables/useToasts'
import { editKey, flushing, saveErrorOf, saveStateOf, type SaveState } from '~/composables/useFileEdit'
import { colourLines, runsOf, snippetTick } from '~/composables/snippetColours'
import { pathKind } from '~/composables/pathKind'
import {
  closeSearch,
  findNames,
  findSymbols,
  openSearch,
  queryOf,
  refreshFile,
  runTextSearch,
  search,
  SEARCH_MODES,
  setQueryOf,
  splitLine,
  toggleSource,
  type FoundName,
  type FoundSymbol,
  type SearchMode
} from '~/composables/useSearch'

/**
 * The search window: inside files, file names or symbols, over the app.
 *
 * Shaped like a command palette: one row to type in, with the mode and the
 * options beside the box, and the rest of the height for the results and the
 * preview. No title and no ✕; Esc and a click outside close it. ↑ and ↓
 * move, ↵ opens and closes, ⌘↵ opens and stays, Tab changes mode, ⌥C ⌥W ⌥R
 * flip the options, ← and → fold a file's hits, ⌘C copies path:line. Focus
 * goes back to wherever it was.
 *
 * The preview is an editor (`CodePreview`): a click, or ⌘↓ from the box, puts
 * the caret in it at the match, and from then on the keys are the editor's
 * (↵ is a new line, Tab indents, ⌘F finds in that file) until Esc goes back to
 * the box. What is typed there saves as in a tab, and the file's hits are
 * searched again once it is on disk. An edit that could not be saved opens
 * the file in a tab when the window closes, so it is not left behind unseen.
 *
 * Tests and dependencies are drawn in a colour of their own and listed after
 * the project's files. Opened with a folder selected in the Files tree, the
 * search is inside that folder, shown as a chip in the box; its ✕, or
 * Backspace in an empty box, searches everything again.
 */

const git = useGit()
const store = git.store
const { layout } = usePanes()
const box = ref<HTMLInputElement | null>(null)
const list = ref<HTMLElement | null>(null)
/** The pane the preview is in, to tell a selection made there from one in a box. */
const preview = ref<HTMLElement | null>(null)
const previewer = ref<InstanceType<typeof CodePreview> | null>(null)
/** The repository the window searches, fixed while it is open. */
const repo = aimedAt()
const toasts = useToasts()
const cursor = ref(0)
const names = ref<FoundName[]>([])
const symbols = ref<FoundSymbol[]>([])
/** Files whose hits are folded away under their heading. */
const folded = ref(new Set<string>())
let before: Element | null = null
let timer: ReturnType<typeof setTimeout> | undefined

const isMac = typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac')

const MODE_LABELS: Record<SearchMode, string> = { text: 'In files', files: 'Names', symbols: 'Symbols' }
const MODE_KEYS: Record<SearchMode, string> = {
  text: 'search.text',
  files: 'search.files',
  symbols: 'search.symbols'
}
const PLACEHOLDERS: Record<SearchMode, string> = {
  text: 'Search inside files',
  files: 'File name or folder/name, :42 for a line',
  symbols: 'Class, function or method; capitals match capitals'
}

/** Every row the arrows step through, in order. */
interface Item {
  /** A line that matched; a folded file's heading; a file; a definition. */
  kind: 'hit' | 'file' | 'name' | 'symbol'
  path: string
  line: number | null
  text?: string
  ranges?: [number, number][]
  group?: FoundName['group']
  indices?: number[]
  name?: string
  symbolKind?: string
  /** For a hit: which of its file's hits it is. */
  hitAt?: number
  /** A folder the finder matched: shown in the Files tree, never opened. */
  dir?: boolean
  /** Git ignores it. */
  ignored?: boolean
}

const items = computed<Item[]>(() => {
  if (search.mode === 'text') {
    return search.found.flatMap((file): Item[] =>
      folded.value.has(file.path)
        ? [{ kind: 'file', path: file.path, line: file.hits[0]?.line ?? null }]
        : file.hits.map((hit, hitAt) => ({
            kind: 'hit',
            hitAt,
            path: file.path,
            line: hit.line,
            text: hit.text,
            ranges: hit.ranges
          }))
    )
  }
  if (search.mode === 'symbols') {
    return symbols.value.map((one) => ({
      kind: 'symbol',
      path: one.path,
      line: one.line,
      name: one.name,
      symbolKind: one.kind,
      indices: one.indices
    }))
  }
  const line = splitLine(search.files.query).line
  return names.value.map((one) => ({
    kind: 'name',
    path: one.path,
    line: one.dir ? null : line,
    group: one.group,
    indices: one.indices,
    dir: one.dir,
    ignored: one.ignored
  }))
})

const current = computed(() => items.value[cursor.value] ?? null)

/**
 * How many rows are in the DOM. A search can list two thousand lines; the
 * list shows a page of them and the next page as it is scrolled, or arrowed,
 * near its end. Rendering them all made every keystroke, every hit that came
 * in and every row the mouse crossed redraw the whole list.
 */
const PAGE = 100
const shown = ref(PAGE)
const rendered = computed(() => items.value.slice(0, shown.value))

/** Shows rows up to `at` and a little past it, in whole pages. */
function showUpTo(at: number) {
  if (at < shown.value - 5) return
  shown.value = Math.min(Math.max(items.value.length, PAGE), (Math.floor((at + 5) / PAGE) + 1) * PAGE)
}

// The cursor is always on a row that is drawn, however it got there.
watch(cursor, (at) => showUpTo(at))

// A new search starts from the first page again.
watch(
  () => [search.mode, search.request],
  () => {
    shown.value = PAGE
  }
)

/** The next page once the list is scrolled near its end. */
function onScroll() {
  const el = list.value
  if (!el || shown.value >= items.value.length) return
  if (el.scrollTop + el.clientHeight >= el.scrollHeight - 300) shown.value += PAGE
}

const LETTERS = { added: 'A', modified: 'M', deleted: 'D', renamed: 'R' } as const

/**
 * Every changed file's status letter and colour, by path: read once per
 * status rather than searched for by every row of two thousand.
 */
const statuses = computed(() => {
  const out = new Map<string, { letter: string; kind: string }>()
  const status = store.status
  if (!status) return out
  for (const entry of [...status.staged, ...status.unstaged]) {
    const kind = change(entry.kind)
    out.set(entry.path, { letter: LETTERS[kind], kind })
  }
  for (const path of status.conflicted) out.set(path, { letter: '!', kind: 'clash' })
  return out
})

/** A file's status letter and colour, as the right panel and the tree give it. */
function status(path: string) {
  return statuses.value.get(path) ?? null
}

const base = (path: string) => path.slice(path.lastIndexOf('/') + 1)
const folder = (path: string) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '')

/**
 * The hit lines of each file, coloured in its language: each file's lines are
 * parsed once, together, and cached. Plain until the language has loaded.
 */
const colours = computed(() => {
  void snippetTick.value
  const out = new Map<string, ReturnType<typeof colourLines>>()
  if (search.mode !== 'text') return out
  // Only the files with a row on screen: the rest are coloured when the list
  // gets to them.
  const drawn = new Set(rendered.value.map((item) => item.path))
  for (const file of search.found) {
    if (drawn.has(file.path)) out.set(file.path, colourLines(file.path, file.hits.map((hit) => hit.text)))
  }
  return out
})

/**
 * Each drawn hit line in runs of one colour, its matches marked over the
 * colours, by row. Worked out when the rows change, not on every redraw (the
 * cursor moving is one).
 */
const runs = computed(() => {
  const out = new Map<number, ReturnType<typeof runsOf>>()
  rendered.value.forEach((item, at) => {
    if (item.kind !== 'hit') return
    const tokens = colours.value.get(item.path)?.[item.hitAt ?? -1] ?? null
    out.set(at, runsOf(item.text ?? '', tokens, item.ranges))
  })
  return out
})

/** `text` with the characters at `indices` (counted from `offset`) picked out. */
function picked(text: string, indices: number[] = [], offset = 0) {
  const hits = new Set(indices.map((at) => at - offset))
  const out: { text: string; hit: boolean }[] = []
  for (const [at, char] of [...text].entries()) {
    const on = hits.has(at)
    const last = out[out.length - 1]
    if (last && last.hit === on) last.text += char
    else out.push({ text: char, hit: on })
  }
  return out
}

const nameCut = (path: string) => path.lastIndexOf('/') + 1

/** Index into `items` of the first row of each file or group, for the headings. */
const firstOf = computed(() => {
  const out = new Map<number, string>()
  let previous = ''
  rendered.value.forEach((item, at) => {
    const key = search.mode === 'text' ? item.path : search.mode === 'files' ? (item.group ?? '') : ''
    if (key && key !== previous) out.set(at, key)
    previous = key
  })
  return out
})

const GROUPS: Record<string, string> = {
  recent: 'Recent',
  changed: 'Changed',
  files: 'Files',
  tests: 'Tests',
  deps: 'Dependencies'
}

/** A test or a dependency gets a colour and a tag of its own; a project file neither. */
function kindOf(path: string) {
  const kind = pathKind(path)
  return kind === 'project' ? null : kind
}

const hitCounts = computed(() => new Map(search.found.map((one) => [one.path, one.hits.length])))

function hitsIn(path: string) {
  return hitCounts.value.get(path) ?? 0
}

/** "N matches in M files", over the list. */
const summary = computed(() => {
  if (search.mode === 'symbols') {
    const count = symbols.value.length
    return search.symbols.query.trim() ? `${count} ${count === 1 ? 'symbol' : 'symbols'}` : ''
  }
  if (search.mode === 'files') return ''
  const done = search.done
  if (done?.error) return done.error
  if (!search.text.query) return ''
  const matches = search.found.reduce((sum, file) => sum + file.hits.length, 0)
  const files = search.found.length
  const where = search.text.changedOnly ? 'changed files' : 'files'
  let text = `${matches.toLocaleString()} ${matches === 1 ? 'match' : 'matches'} in ${files} ${files === 1 ? 'file' : where}`
  if (search.running) text += ' · searching…'
  if (done?.capped) {
    text += done.more ? ` · ${done.more.toLocaleString()}+ more not shown` : ' · stopped here, more not searched'
  } else if (done?.more) text += ` · ${done.more.toLocaleString()} more not shown`
  if (done && search.text.changedOnly) text += ' · only changed files'
  if (search.text.scope) text += ` · in ${search.text.scope}/`
  return text
})

/** Every match in the file under the cursor, for the preview to mark. */
const previewHits = computed(() => {
  if (search.mode !== 'text' || !current.value) return []
  const file = search.found.find((one) => one.path === current.value!.path)
  return (file?.hits ?? []).map((hit) => ({ line: hit.line, ranges: hit.ranges }))
})

/** What is in the folder under the cursor, one level down, for its preview. */
const folderEntries = ref<DirEntry[] | null>(null)
let folderAsk = 0

watch(
  () => (current.value?.dir ? current.value.path : null),
  async (path) => {
    const mine = ++folderAsk
    folderEntries.value = null
    if (path === null) return
    const entries = await invoke<DirEntry[]>('list_dir', { path }).catch(() => [])
    if (mine === folderAsk) folderEntries.value = entries ?? []
  }
)

/** The folders of the file under the cursor, for the line over the preview. */
const crumbs = computed(() => (current.value ? current.value.path.split('/') : []))

/** What ⌘F inside the preview starts from: the window's own query, in files. */
const previewQuery = computed(() =>
  search.mode === 'text' && search.text.query
    ? {
        search: search.text.query,
        caseSensitive: search.text.case,
        wholeWord: search.text.word,
        regexp: search.text.regex
      }
    : null
)

/**
 * Where the edits in the previewed file stand, for the line over it: a dot
 * while they are not on disk yet, a warning when they cannot get there. No
 * word for saved, as in the file's header.
 */
const previewSave = computed(() => {
  const path = current.value?.path
  if (!path) return null
  const state = saveStateOf(repo, tabId(path))
  if (state === 'unsaved' || state === 'saving') return { dot: true, text: '', title: 'Not saved yet' }
  if (state === 'conflict') {
    return { dot: false, text: 'Changed on disk', title: 'The file changed on disk under your edits; the bar below asks which to keep' }
  }
  if (state === 'failed') {
    return { dot: false, text: 'Not saved', title: saveErrorOf(repo, tabId(path)) ?? 'The last save failed' }
  }
  return null
})

// --- edits made in the preview

/** Files edited in the preview while the window is open. */
const touched = ref<string[]>([])
/** How each one's edits last stood, to see them land. */
const lastState = new Map<string, SaveState | undefined>()

watch(
  () => (current.value ? saveStateOf(repo, tabId(current.value.path)) : undefined),
  (state) => {
    const path = current.value?.path
    if (path && state && state !== 'saved' && state !== 'readonly' && !touched.value.includes(path)) {
      lastState.set(path, state)
      touched.value = [...touched.value, path]
    }
  }
)

// A file edited in the preview landed on disk: its hits are searched again,
// that file only, and the cursor stays on the nearest one.
watch(
  () => touched.value.map((path) => saveStateOf(repo, tabId(path))),
  (states) => {
    touched.value.forEach((path, at) => {
      const state = states[at]
      const before = lastState.get(path)
      lastState.set(path, state)
      if (state === 'saved' && before !== 'saved') void refreshHits(path)
    })
  }
)

/**
 * After the window closed: an edit made in the preview that could not be
 * saved (the file changed on disk, or the write failed) is opened in a tab,
 * where its bar asks what to do, rather than left waiting unseen.
 */
async function openUnsaved(paths: string[]) {
  // The preview's editor has gone by now, and left its edits to be saved.
  await nextTick()
  await Promise.all(paths.map((path) => flushing.get(editKey(repo, tabId(path)))))
  if (aimedAt() !== repo) return
  const stuck = paths.filter((path) => {
    const state = saveStateOf(repo, tabId(path))
    return state === 'conflict' || state === 'failed'
  })
  if (!stuck.length) return
  for (const path of stuck) openTab({ path, mode: 'file', side: 'unstaged' }, { reveal: 'switch' })
  const names = stuck.map((path) => path.slice(path.lastIndexOf('/') + 1)).join(', ')
  toasts.info(
    `Edits in ${names} are not saved`,
    `The file changed on disk, or the save failed. ${stuck.length === 1 ? 'It is' : 'They are'} open in a tab, where the bar asks what to keep.`
  )
}

async function refreshHits(path: string) {
  if (search.mode !== 'text') return
  const was = current.value
  await refreshFile(path)
  if (!was) return
  if (was.path !== path) {
    // Another file's row: the same row, wherever it is now.
    const same = items.value.findIndex((one) => one.path === was.path && one.line === was.line)
    if (same >= 0) cursor.value = same
    return
  }
  let best = -1
  let gap = Infinity
  items.value.forEach((one, at) => {
    if (one.path !== path) return
    const distance = Math.abs((one.line ?? 0) - (was.line ?? 0))
    if (distance < gap) {
      gap = distance
      best = at
    }
  })
  if (best >= 0) cursor.value = best
  else cursor.value = Math.min(cursor.value, Math.max(0, items.value.length - 1))
}

/** The caret in the preview, on the selected match. */
function intoPreview() {
  const item = current.value
  if (!item) return
  const column = item.kind === 'hit' ? (item.ranges?.[0]?.[0] ?? 0) : 0
  previewer.value?.focusAt(item.line, column)
}

// --- running the search as the box changes

function schedule() {
  clearTimeout(timer)
  if (search.mode === 'text') {
    timer = setTimeout(() => {
      cursor.value = 0
      folded.value = new Set()
      void runTextSearch()
    }, 120)
  } else if (search.mode === 'symbols') {
    timer = setTimeout(async () => {
      const query = search.symbols.query
      const found = await findSymbols(query)
      if (found === null || query !== search.symbols.query) return
      symbols.value = found
      cursor.value = 0
    }, 60)
  } else {
    timer = setTimeout(async () => {
      const query = search.files.query
      const found = await findNames(splitLine(query).query)
      if (found === null || query !== search.files.query) return
      names.value = found
      cursor.value = 0
    }, 60)
  }
}

watch(
  () => [
    search.mode,
    search.text.query,
    search.text.case,
    search.text.word,
    search.text.regex,
    search.text.changedOnly,
    search.text.glob,
    search.text.scope,
    search.text.ignored,
    search.text.deps,
    search.files.query,
    search.files.deps,
    search.files.ignored,
    search.symbols.query
  ],
  schedule
)

// The cursor stays on a real row while results arrive.
watch(
  () => items.value.length,
  (count) => {
    if (cursor.value >= count) cursor.value = Math.max(0, count - 1)
  }
)

// --- opening

function open(item: Item | null, stay: boolean) {
  if (!item) return
  if (item.dir) {
    // A folder: the sidebar comes to the Files tree with it in view.
    revealInFiles(item.path)
    if (!stay) closeSearch()
    return
  }
  // A file that was looked for: the sidebar comes to the Files tree with it.
  openTab(
    { path: item.path, mode: 'file', side: 'unstaged' },
    { line: item.line ?? undefined, reveal: 'switch' }
  )
  if (!stay) closeSearch()
}

async function move(by: number) {
  const count = items.value.length
  if (!count) return
  cursor.value = Math.min(count - 1, Math.max(0, cursor.value + by))
  await nextTick()
  list.value?.querySelector('.on')?.scrollIntoView?.({ block: 'nearest' })
}

/** Folds a file's hits under its heading, or opens them again. */
async function fold(path: string, shut: boolean) {
  const next = new Set(folded.value)
  if (shut) next.add(path)
  else next.delete(path)
  folded.value = next
  // The cursor stays with the file: on its heading, or on its first hit.
  const at = items.value.findIndex((one) => one.path === path)
  if (at >= 0) cursor.value = at
  await nextTick()
  list.value?.querySelector('.on')?.scrollIntoView?.({ block: 'nearest' })
}

function nextMode(by: number) {
  const at = SEARCH_MODES.indexOf(search.mode)
  openSearch(SEARCH_MODES[(at + by + SEARCH_MODES.length) % SEARCH_MODES.length]!)
}

/** ⌥C, ⌥W, ⌥R, by the key's place: on a Mac ⌥ makes them type ç, ∑ and ®. */
const OPTION_KEYS: Record<string, 'case' | 'word' | 'regex'> = {
  KeyC: 'case',
  KeyW: 'word',
  KeyR: 'regex'
}

/** Whether a key was pressed in the preview's editor, or its find bar. */
function inPreview(event: KeyboardEvent) {
  const target = event.target as HTMLElement | null
  return !!target?.closest?.('.cm-editor') && !!preview.value?.contains(target)
}

function onKey(event: KeyboardEvent) {
  const mod = isMac ? event.metaKey : event.ctrlKey
  const inBox = event.target === box.value
  const field = event.target as HTMLInputElement | null
  if (inPreview(event)) {
    // The editor's keys: ↵ is a new line, Tab indents, the arrows move the
    // caret. Esc goes back to the box, once the editor has had it (its find
    // bar, a selection); the next Esc in the box closes the window.
    if (event.key !== 'Escape') return
    event.stopPropagation()
    if (event.defaultPrevented) return
    event.preventDefault()
    box.value?.focus()
    return
  }
  if (event.key === 'ArrowDown' && mod && !event.altKey && !event.shiftKey && inBox) {
    event.preventDefault()
    intoPreview()
    return
  }
  if (matchesShortcut(event, 'search.text')) {
    event.preventDefault()
    openSearch('text')
  } else if (matchesShortcut(event, 'files.recent')) {
    // The same as with the window shut: the names, empty, on the recent files.
    event.preventDefault()
    openSearch('files', true)
  } else if (matchesShortcut(event, 'search.files')) {
    event.preventDefault()
    openSearch('files')
  } else if (matchesShortcut(event, 'search.symbols')) {
    event.preventDefault()
    openSearch('symbols')
  } else if (event.key === 'Tab' && inBox && !mod && !event.altKey) {
    event.preventDefault()
    nextMode(event.shiftKey ? -1 : 1)
  } else if (event.altKey && !event.metaKey && !event.ctrlKey && OPTION_KEYS[event.code]) {
    event.preventDefault()
    if (search.mode !== 'text') return
    const option = OPTION_KEYS[event.code]!
    search.text[option] = !search.text[option]
  } else if (mod && !event.altKey && !event.shiftKey && event.code === 'KeyC' && current.value) {
    // A selection in the box is copied as usual; otherwise the result is.
    if (field?.tagName === 'INPUT' && field.selectionStart !== field.selectionEnd) return
    // So is text selected in the preview.
    const picked = window.getSelection?.()
    if (picked && !picked.isCollapsed && picked.toString() && preview.value?.contains(picked.anchorNode)) return
    event.preventDefault()
    const item = current.value
    copyText(item.line ? `${item.path}:${item.line}` : item.path, 'Location')
  } else if (event.key === 'ArrowDown') {
    event.preventDefault()
    void move(1)
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    void move(-1)
  } else if ((event.key === 'ArrowRight' || event.key === 'ArrowLeft') && field?.tagName === 'INPUT' && !inBox) {
    // The file mask is a field of its own: the arrows move its caret.
    return
  } else if (event.key === 'ArrowRight' && search.mode === 'text' && current.value?.kind === 'file') {
    // In the box → moves the caret, except from the very end of it.
    const end = field?.value.length ?? 0
    if (inBox && ((field?.selectionStart ?? end) < end || (field?.selectionEnd ?? end) < end)) return
    event.preventDefault()
    void fold(current.value.path, false)
  } else if (event.key === 'ArrowLeft' && search.mode === 'text' && current.value?.kind === 'hit') {
    // In the box ← moves the caret, except from the very start of it.
    if (inBox && (field?.selectionStart ?? 0) + (field?.selectionEnd ?? 0) > 0) return
    event.preventDefault()
    void fold(current.value.path, true)
  } else if (
    event.key === 'Backspace' &&
    inBox &&
    search.mode === 'text' &&
    search.text.scope &&
    !field?.value &&
    !mod &&
    !event.altKey &&
    !event.repeat &&
    !event.isComposing
  ) {
    // Backspace in an empty box takes the folder chip away, as in a tag field.
    // Not a held one: holding Backspace to empty the box stops at the chip.
    event.preventDefault()
    search.text.scope = ''
  } else if (event.key === 'Enter') {
    // ↵ on an option or a mode button presses that button.
    if ((event.target as HTMLElement | null)?.tagName === 'BUTTON') return
    event.preventDefault()
    open(current.value, mod)
  }
}

watch(
  () => search.mode,
  async () => {
    await nextTick()
    box.value?.focus()
    box.value?.select()
  }
)

onMounted(async () => {
  before = document.activeElement
  schedule()
  await nextTick()
  box.value?.focus()
  box.value?.select()
})

onBeforeUnmount(() => {
  clearTimeout(timer)
  if (touched.value.length) void openUnsaved([...touched.value])
  if (before instanceof HTMLElement && before.isConnected) before.focus()
})

const query = computed({
  get: () => queryOf(search.mode),
  set: (value: string) => setQueryOf(search.mode, value)
})

const alt = isMac ? '⌥' : 'Alt+'
const hint = computed(
  () =>
    `↵ open · ${keyLabel('mod+enter')} open, keep searching · ${keyLabel('mod+arrowdown')} edit in the preview · Tab mode · ${keyLabel('mod+c')} copy path:line · Esc close`
)
</script>

<template>
  <AppModal title="Search" :width="1180" palette @close="closeSearch">
    <div class="search" @keydown="onKey">
      <!-- One row: the box, the options for text, and the mode. -->
      <div class="top">
        <div class="field">
          <Search :size="14" class="faint" />
          <span
            v-if="search.mode === 'text' && search.text.scope"
            class="scope"
            data-testid="search-scope"
            :title="`Searching inside ${search.text.scope}/ only. Backspace in the empty box searches everything.`"
          >
            <Folder :size="12" />
            <span class="scope-path">{{ search.text.scope }}/</span>
            <button type="button" class="scope-x" aria-label="Search everything" @click="search.text.scope = ''">
              <X :size="11" />
            </button>
          </span>
          <input
            ref="box"
            v-model="query"
            type="text"
            :aria-label="PLACEHOLDERS[search.mode]"
            spellcheck="false"
            :placeholder="PLACEHOLDERS[search.mode]"
            :title="hint"
          />
          <template v-if="search.mode === 'text'">
            <button
              class="opt"
              data-option="case"
              :class="{ on: search.text.case }"
              :aria-pressed="search.text.case"
              :title="`Match case (${alt}C)`"
              @click="search.text.case = !search.text.case"
            >
              <CaseSensitive :size="15" />
            </button>
            <button
              class="opt"
              data-option="word"
              :class="{ on: search.text.word }"
              :aria-pressed="search.text.word"
              :title="`Whole words (${alt}W)`"
              @click="search.text.word = !search.text.word"
            >
              <WholeWord :size="15" />
            </button>
            <button
              class="opt"
              data-option="regex"
              :class="{ on: search.text.regex }"
              :aria-pressed="search.text.regex"
              :title="`Regular expression (${alt}R)`"
              @click="search.text.regex = !search.text.regex"
            >
              <Regex :size="15" />
            </button>
            <button
              class="opt"
              data-option="changed"
              :class="{ on: search.text.changedOnly }"
              :aria-pressed="search.text.changedOnly"
              title="Changed files only"
              @click="search.text.changedOnly = !search.text.changedOnly"
            >
              <GitCompare :size="14" />
            </button>
            <button
              class="opt"
              data-option="ignored"
              :class="{ on: search.text.ignored }"
              :aria-pressed="search.text.ignored"
              title="Include files git ignores (app/etc/env.php, .env), after the project's"
              @click="toggleSource('text', 'ignored')"
            >
              <EyeOff :size="14" />
            </button>
            <button
              class="opt"
              data-option="deps"
              :class="{ on: search.text.deps }"
              :aria-pressed="search.text.deps"
              title="Include dependencies: vendor/ and node_modules/, last"
              @click="toggleSource('text', 'deps')"
            >
              <Package :size="14" />
            </button>
            <span class="mask-field" title="File mask, e.g. *.php or src/**/*.ts">
              <Funnel :size="12" class="faint" />
              <input
                v-model="search.text.glob"
                class="mask"
                type="text"
                aria-label="File mask"
                spellcheck="false"
                placeholder="*.php"
              />
            </span>
          </template>
          <template v-else-if="search.mode === 'files'">
            <button
              class="opt"
              data-option="ignored"
              :class="{ on: search.files.ignored }"
              :aria-pressed="search.files.ignored"
              title="Include files git ignores (app/etc/env.php, .env), among the project's"
              @click="toggleSource('files', 'ignored')"
            >
              <EyeOff :size="14" />
            </button>
            <button
              class="opt"
              data-option="deps"
              :class="{ on: search.files.deps }"
              :aria-pressed="search.files.deps"
              title="Include dependencies: vendor/ and node_modules/, after everything else"
              @click="toggleSource('files', 'deps')"
            >
              <Package :size="14" />
            </button>
          </template>
        </div>
        <div class="modes" role="tablist">
          <button
            v-for="mode in SEARCH_MODES"
            :key="mode"
            class="mode"
            role="tab"
            :data-mode="mode"
            :class="{ on: search.mode === mode }"
            :aria-selected="search.mode === mode"
            :title="`${MODE_LABELS[mode]} (${keyLabel(keysFor(MODE_KEYS[mode]))}, or Tab)`"
            @click="openSearch(mode)"
          >
            {{ MODE_LABELS[mode] }}
          </button>
        </div>
      </div>

      <!-- The list never takes the preview's last 280px, however wide it was
           dragged in a wider window. -->
      <div
        class="panes"
        :style="{
          gridTemplateColumns: `minmax(220px, min(${layout.search}px, calc(100% - 280px))) 1px minmax(0, 1fr)`
        }"
      >
        <div class="left">
          <div v-if="summary" class="summary" :class="{ bad: search.done?.error }">{{ summary }}</div>
          <div ref="list" class="list" role="listbox" @scroll.passive="onScroll">
            <template v-for="(item, at) in rendered" :key="`${item.kind}:${item.path}:${item.line}:${at}`">
              <!-- A file's heading: its hits fold under it with ← and open with →. -->
              <div
                v-if="search.mode === 'text' && (item.kind === 'file' || firstOf.has(at))"
                class="group"
                :class="{ on: item.kind === 'file' && at === cursor, folded: item.kind === 'file' }"
                :data-path="item.path"
                @mousemove="item.kind === 'file' && (cursor = at)"
                @click="fold(item.path, item.kind !== 'file')"
              >
                <ChevronRight :size="12" class="chev" :class="{ down: item.kind !== 'file' }" />
                <span v-if="status(item.path)" class="letter" :class="status(item.path)!.kind">{{
                  status(item.path)!.letter
                }}</span>
                <FileIcon :path="item.path" :size="13" />
                <span class="file" :class="[kindOf(item.path) && `kind-${kindOf(item.path)}`, status(item.path)?.kind]">{{
                  base(item.path)
                }}</span>
                <span class="dir">{{ folder(item.path) }}</span>
                <span v-if="kindOf(item.path)" class="kind-tag" :class="kindOf(item.path)">{{ kindOf(item.path) }}</span>
                <span class="count">{{ hitsIn(item.path) }}</span>
              </div>
              <div v-else-if="firstOf.has(at) && item.group" class="group plain">
                {{ GROUPS[item.group] }}
              </div>
              <div
                v-if="item.kind !== 'file'"
                class="item"
                role="option"
                :class="{ on: at === cursor, hit: item.kind === 'hit' }"
                :aria-selected="at === cursor"
                @mousemove="cursor = at"
                @click="open(item, false)"
              >
                <template v-if="item.kind === 'hit'">
                  <span class="num">{{ item.line }}</span>
                  <span class="code">
                    <template v-for="(run, index) in runs.get(at) ?? []" :key="index">
                      <mark v-if="run.hit"
                        ><span v-for="(part, p) in run.parts" :key="p" :style="part.style ?? undefined">{{
                          part.text
                        }}</span></mark
                      >
                      <template v-else
                        ><span v-for="(part, p) in run.parts" :key="p" :style="part.style ?? undefined">{{
                          part.text
                        }}</span></template
                      >
                    </template>
                  </span>
                </template>
                <template v-else-if="item.kind === 'symbol'">
                  <span class="kind" :class="item.symbolKind">{{ item.symbolKind }}</span>
                  <span class="file symbol">
                    <template v-for="(piece, index) in picked(item.name ?? '', item.indices)" :key="index">
                      <b v-if="piece.hit">{{ piece.text }}</b>
                      <template v-else>{{ piece.text }}</template>
                    </template>
                  </span>
                  <FileIcon :path="item.path" :size="12" />
                  <span class="dir">{{ base(item.path) }}:{{ item.line }} · {{ folder(item.path) }}</span>
                </template>
                <template v-else>
                  <span v-if="!item.dir && status(item.path)" class="letter" :class="status(item.path)!.kind">{{
                    status(item.path)!.letter
                  }}</span>
                  <FileIcon :path="item.path" :folder="item.dir" :size="13" />
                  <span
                    class="file"
                    :class="[
                      kindOf(item.path) && `kind-${kindOf(item.path)}`,
                      { ignored: item.ignored },
                      item.dir ? null : status(item.path)?.kind
                    ]"
                    :data-dir="item.dir || null"
                  >
                    <template
                      v-for="(piece, index) in picked(base(item.path), item.indices, nameCut(item.path))"
                      :key="index"
                    >
                      <b v-if="piece.hit">{{ piece.text }}</b>
                      <template v-else>{{ piece.text }}</template>
                    </template>
                    <template v-if="item.dir">/</template>
                  </span>
                  <span class="dir">
                    <template v-for="(piece, index) in picked(folder(item.path), item.indices)" :key="index">
                      <b v-if="piece.hit">{{ piece.text }}</b>
                      <template v-else>{{ piece.text }}</template>
                    </template>
                  </span>
                  <span v-if="kindOf(item.path)" class="kind-tag" :class="kindOf(item.path)">{{ kindOf(item.path) }}</span>
                  <span v-else-if="item.ignored" class="kind-tag ignored">ignored</span>
                </template>
              </div>
            </template>
            <p
              v-if="!items.length && query && !(search.mode === 'text' && search.running)"
              class="none faint"
            >
              {{ search.done?.error ? '' : 'Nothing found.' }}
            </p>
            <p v-else-if="!items.length && !query && search.mode === 'symbols'" class="none faint">
              Type a class, function or method name. Capitals match capitals: MECC finds
              MakeExactConnectionCommand.
            </p>
          </div>
        </div>
        <ResizeHandle side="search" />
        <div ref="preview" class="side">
          <!-- Where the file is and which line, small, over the preview. -->
          <div v-if="current" class="where" :title="current.path">
            <FileIcon :path="current.path" :folder="current.dir" :size="12" />
            <!-- The folders give way first: the name is what says which file. -->
            <span class="crumbs">
              <span v-if="crumbs.length > 1" class="dirs">
                <template v-for="(part, index) in crumbs.slice(0, -1)" :key="index">
                  <span v-if="index" class="sep">›</span>
                  <span>{{ part }}</span>
                </template>
              </span>
              <span v-if="crumbs.length > 1" class="sep">›</span>
              <span class="last">{{ crumbs[crumbs.length - 1] }}</span>
            </span>
            <span v-if="current.line" class="at">:{{ current.line }}</span>
            <span v-if="previewSave?.dot" class="dirty" :title="previewSave.title" />
            <span v-else-if="previewSave" class="save-warning" :title="previewSave.title"
              ><TriangleAlert :size="11" />{{ previewSave.text }}</span
            >
          </div>
          <!-- A folder is not read as a file: what is in it, one level down. -->
          <div v-if="current?.dir" class="folder-preview">
            <div v-for="entry in folderEntries ?? []" :key="entry.path" class="entry" :class="{ faint: entry.ignored }">
              <FileIcon :path="entry.path" :folder="entry.dir" :size="13" />
              <span>{{ entry.name }}{{ entry.dir ? '/' : '' }}</span>
            </div>
            <p v-if="folderEntries && !folderEntries.length" class="none faint">Empty folder.</p>
          </div>
          <CodePreview
            v-show="!current?.dir"
            ref="previewer"
            :path="current?.dir ? null : (current?.path ?? null)"
            :line="current?.line ?? null"
            :hits="previewHits"
            :query="previewQuery"
          />
        </div>
      </div>

      <p class="keys faint">{{ hint }}</p>
    </div>
  </AppModal>
</template>

<style scoped>
.search {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
  min-height: 0;
}

.top {
  display: flex;
  align-items: center;
  gap: 8px;
}

.field {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 4px;
  height: 34px;
  padding: 0 6px 0 10px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--bg);
}

.field:focus-within {
  border-color: var(--ring);
  box-shadow: var(--focus);
}

/* The inputs inside the frame are bare: the global field style (border,
   background, corners, the focus halo) would draw a second box inside it.
   The frame shows focus for both, with :focus-within. */
.field input,
.field input:focus {
  border: 0;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
  outline: none;
  padding: 0;
}

.field > input {
  flex: 1;
  min-width: 0;
  font-size: 14px;
}

.opt {
  flex: none;
  display: grid;
  place-items: center;
  width: 24px;
  height: 24px;
  color: var(--text-faint);
  border-radius: var(--radius-sm);
}

.opt:hover {
  color: var(--text);
}

.opt.on {
  background: var(--primary-bg);
  color: var(--accent-soft);
}

.mask-field {
  flex: none;
  display: flex;
  align-items: center;
  gap: 3px;
  height: 24px;
  margin-left: 2px;
  padding: 0 6px;
  border-left: 1px solid var(--line-soft);
}

.mask {
  width: 90px;
  font-size: 12px;
  font-family: var(--mono);
}

/* The mode, as a small segmented switch beside the box. */
.modes {
  flex: none;
  display: flex;
  height: 28px;
  padding: 2px;
  gap: 2px;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  background: var(--canvas);
}

.mode {
  padding: 0 10px;
  font-size: 12px;
  color: var(--text-dim);
  border-radius: var(--radius-sm);
}

.mode.on {
  background: var(--bg-active);
  color: var(--text);
  font-weight: 600;
}

.panes {
  flex: 1;
  min-height: 0;
  display: grid;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  overflow: hidden;
}

.left {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
}

.list {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.side {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.summary {
  padding: 4px 8px;
  font-size: 11.5px;
  color: var(--text-dim);
  border-bottom: 1px solid var(--line-soft);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.summary.bad {
  color: var(--danger);
}

.where {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 4px 8px;
  font-size: 11.5px;
  color: var(--text-faint);
  border-bottom: 1px solid var(--line-soft);
  white-space: nowrap;
  min-width: 0;
}

.crumbs {
  display: flex;
  min-width: 0;
  overflow: hidden;
}

.crumbs .dirs {
  flex: 0 100 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.crumbs > .sep {
  flex: none;
}

.crumbs .sep {
  padding: 0 4px;
  opacity: 0.6;
}

.crumbs .last {
  flex: 0 1 auto;
  min-width: 3em;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text);
  font-weight: 600;
}

.at {
  flex: none;
  font-family: var(--mono);
}

/* Edits in the preview not on disk yet: the tab strip's dot. */
.where .dirty {
  flex: none;
  width: 8px;
  height: 8px;
  margin-left: 2px;
  border-radius: 50%;
  background: var(--text-dim);
}

/* Edits that cannot get to disk: the file header's chip. */
.save-warning {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: none;
  padding: 1px 6px;
  font-size: 11px;
  line-height: 16px;
  color: var(--danger);
  border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
  border-radius: var(--radius-sm);
}

.group {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 4px 8px 2px;
  font-size: 12.5px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  cursor: default;
}

.group.on {
  background: var(--bg-active);
}

.group.plain {
  font-size: 11px;
  text-transform: uppercase;
  color: var(--text-faint);
}

.chev {
  flex: none;
  color: var(--text-faint);
  transition: transform 0.1s;
}

.chev.down {
  transform: rotate(90deg);
}

.count {
  flex: none;
  margin-left: auto;
  padding: 0 6px;
  font-size: 10.5px;
  font-weight: 600;
  border-radius: var(--radius-pill);
  color: var(--text-dim);
  background: var(--bg-raised);
}

.item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 8px;
  font-size: 12.5px;
  white-space: nowrap;
  overflow: hidden;
  cursor: default;
}

.item.hit {
  padding-left: 26px;
}

.item.on {
  background: var(--bg-active);
}

.num {
  flex: none;
  min-width: 28px;
  text-align: right;
  font-family: var(--mono);
  font-size: 11.5px;
  color: var(--text-faint);
}

.code {
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--mono);
  font-size: 12px;
}

.code mark {
  background: var(--warning-bg);
  color: inherit;
  border-radius: 2px;
}

.file {
  flex: none;
  max-width: 70%;
  overflow: hidden;
  text-overflow: ellipsis;
}

.file b,
.dir b {
  color: var(--accent-soft);
}

.file.symbol {
  font-family: var(--mono);
  font-size: 12px;
}

.kind {
  flex: none;
  min-width: 52px;
  font-size: 10.5px;
  color: var(--text-faint);
}

.kind.class,
.kind.interface,
.kind.type {
  color: var(--purple);
}

.kind.function,
.kind.method {
  color: var(--info);
}

.dir {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 11.5px;
  font-weight: 400;
  color: var(--text-faint);
}

.letter {
  flex: none;
  font-family: var(--mono);
  font-size: 11px;
  font-weight: 700;
  color: var(--amber);
}

/* Tests and dependencies: a colour of their own, quieter than the project's
   files. A changed file's status colour still wins, below. */
.file.kind-test {
  color: color-mix(in srgb, var(--success) 60%, var(--text-faint));
}

.file.kind-vendor {
  color: color-mix(in srgb, var(--warning) 45%, var(--text-faint));
}

.file.ignored {
  color: var(--text-dim);
}

.kind-tag {
  flex: none;
  font-size: 10px;
  font-weight: 400;
  padding: 0 6px;
  border-radius: var(--radius-pill);
  white-space: nowrap;
  text-transform: none;
}

.kind-tag.test {
  color: color-mix(in srgb, var(--success) 70%, var(--text-faint));
  background: color-mix(in srgb, var(--success) 12%, transparent);
}

.kind-tag.vendor {
  color: color-mix(in srgb, var(--warning) 60%, var(--text-faint));
  background: color-mix(in srgb, var(--warning) 12%, transparent);
}

.kind-tag.ignored {
  color: var(--text-faint);
  background: var(--bg-raised);
}

/* The folder a search is kept inside, as a chip at the start of the box. */
.scope {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  max-width: 40%;
  height: 22px;
  padding: 0 4px 0 6px;
  font-size: 12px;
  color: var(--text-dim);
  background: var(--bg-raised);
  border: 1px solid var(--line-soft);
  border-radius: var(--radius-sm);
}

.scope-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.scope-x {
  display: grid;
  place-items: center;
  width: 16px;
  height: 16px;
  color: var(--text-faint);
  border-radius: var(--radius-sm);
}

.scope-x:hover {
  color: var(--text);
  background: var(--hover);
}

.letter.added,
.file.added {
  color: var(--green-soft);
}

.letter.deleted,
.file.deleted {
  color: var(--red);
}

.letter.renamed,
.file.renamed {
  color: var(--purple);
}

.file.modified {
  color: var(--amber);
}

.none {
  padding: 10px;
  font-size: 12px;
}

/* A folder's preview: its files and folders, one level down. */
.folder-preview {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 6px 0;
}

.folder-preview .entry {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 10px;
  font-size: 12.5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.keys {
  margin: 0;
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
