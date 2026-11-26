<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { h, type FunctionalComponent } from 'vue'
import {
  ChevronRight,
  ChevronsDown,
  ChevronsDownUp,
  ChevronsUp,
  ClipboardPaste,
  Copy,
  CopyPlus,
  Eye,
  EyeOff,
  FileDiff,
  FilePlus,
  Files,
  FileText,
  FileX,
  Folder,
  FolderClock,
  FolderOpen,
  FolderPlus,
  History,
  Minus,
  Palette,
  Pencil,
  Plus,
  Scissors,
  Search,
  Trash2,
  Undo2,
  Users
} from 'lucide-vue-next'
import FileIcon from '~/components/FileIcon.vue'
import { copyText, relativeTime, useGit } from '~/composables/useGit'
import { change } from '~/composables/useFileView'
import { diffMode } from '~/composables/useDiffMode'
import { useContextMenu, type MenuItem } from '~/composables/useContextMenu'
import { useDragDrop } from '~/composables/useDragDrop'
import { useToasts } from '~/composables/useToasts'
import { openTab, useTabs } from '~/composables/useTabs'
import {
  ancestors,
  findFiles,
  listingsHere,
  loadDir,
  reloadOpen,
  rootFailed,
  treeFolder,
  treeSession,
  treeView,
  type DirEntry,
  type FoundFile
} from '~/composables/useFileTree'
import {
  canPaste,
  carriesFiles,
  clip,
  copyEntries,
  createEntry,
  deleteEntries,
  droppedItems,
  duplicateEntry,
  importDropped,
  joinPath,
  moveEntries,
  nameOf,
  parentOf,
  pasteInto,
  renameEntry,
  treeClipboard,
  within
} from '~/composables/useFileOps'
import { ignoreChoices } from '~/composables/ignoreRules'
import { speedMatch, type SpeedMatch } from '~/composables/speedSearch'
import { openSearch } from '~/composables/useSearch'
import { keyLabel, keysFor } from '~/composables/useShortcuts'
import { FOLDER_COLORS, folderColor, setFolderColor } from '~/composables/useFolderColors'

/**
 * The repository as folders and files, in the sidebar beside the branches.
 *
 * Read one folder at a time, when it is opened, so a repository of any size
 * shows at once. Coloured from the status the right panel already has rather
 * than by walking anything: a changed file in its status colour with a dot for
 * where its changes are, and every folder above it in the same colour, so the
 * places with work in them stand out with everything folded.
 */

const git = useGit()
const store = git.store
const tabs = useTabs()
const menu = useContextMenu()
const drag = useDragDrop()

const session = computed(() => treeSession())
const here = computed(() => listingsHere())

/** What the status says about one path. */
interface Mark {
  kind: string
  staged: boolean
  unstaged: boolean
  /** What the unstaged change is, when there is one: a file staged as new
      and edited since is `added` over all and `modified` here. */
  unstagedKind?: string
}

const marks = computed(() => {
  const out = new Map<string, Mark>()
  const status = store.status
  if (!status) return out
  const put = (path: string, kind: string, side: 'staged' | 'unstaged') => {
    const mark: Mark = out.get(path) ?? { kind: change(kind), staged: false, unstaged: false }
    mark[side] = true
    if (side === 'unstaged') mark.unstagedKind = change(kind)
    out.set(path, mark)
  }
  for (const entry of status.staged) put(entry.path, entry.kind, 'staged')
  for (const entry of status.unstaged) put(entry.path, entry.kind, 'unstaged')
  for (const path of status.conflicted) {
    out.set(path, { kind: 'clash', staged: false, unstaged: true })
  }
  return out
})

/** Each folder with a change somewhere below it, and the colour it takes. */
const folders = computed(() => {
  const out = new Map<string, string>()
  for (const [path, mark] of marks.value) {
    for (const dir of ancestors(path)) {
      const was = out.get(dir)
      if (!was || was === mark.kind) out.set(dir, mark.kind)
      else if (mark.kind === 'clash' || was === 'clash') out.set(dir, 'clash')
      else out.set(dir, 'modified')
    }
  }
  return out
})

/** How many changed files are somewhere under each folder. */
const counts = computed(() => {
  const out = new Map<string, number>()
  for (const path of marks.value.keys()) {
    for (const dir of ancestors(path)) out.set(dir, (out.get(dir) ?? 0) + 1)
  }
  return out
})

interface Row {
  key: string
  path: string
  name: string
  depth: number
  dir: boolean
  ignored: boolean
  link: boolean
  open: boolean
  /** Deleted: in the status, no longer on disk. */
  gone: boolean
  /** A placeholder while a folder is being read. */
  loading?: boolean
  /** A filter result: the folder it is in, and the characters that matched. */
  folder?: string
  hits?: number[]
  /** The row a new file or folder is being named in. */
  draft?: boolean
}

type Child = DirEntry & { gone?: boolean }

/**
 * The deleted files the status still lists, by the folder each would be in,
 * and the folders above them that may be gone too. Worked out once per status
 * rather than once per open folder.
 */
const deletedIn = computed(() => {
  const out = new Map<string, Child[]>()
  const seen = new Set<string>()
  for (const [path, mark] of marks.value) {
    if (mark.kind !== 'deleted') continue
    const parts = path.split('/')
    for (let at = 0; at < parts.length; at++) {
      const dir = parts.slice(0, at).join('/')
      const entry = parts.slice(0, at + 1).join('/')
      if (seen.has(entry)) continue
      seen.add(entry)
      const list = out.get(dir) ?? []
      list.push({ name: parts[at]!, path: entry, dir: at < parts.length - 1, ignored: false, link: false, gone: true })
      out.set(dir, list)
    }
  }
  return out
})

/** A folder's entries, with the deleted files the status still lists added back. */
function children(dir: string): Child[] {
  const listed: Child[] = here.value[dir] ?? []
  const deleted = deletedIn.value.get(dir)
  if (!deleted) return listed
  const names = new Set(listed.map((one) => one.name))
  const extra = deleted.filter((one) => !names.has(one.name))
  if (!extra.length) return listed
  return [...listed, ...extra].sort(
    (a, b) => Number(b.dir) - Number(a.dir) || a.name.toLowerCase().localeCompare(b.name.toLowerCase())
  )
}

const treeRows = computed<Row[]>(() => {
  const out: Row[] = []
  const open = session.value.open
  const visit = (dir: string, depth: number, gone: boolean) => {
    if (!gone && !(dir in here.value)) {
      out.push({ key: `${dir}/…`, path: dir, name: 'Reading…', depth, dir: false, ignored: false, link: false, open: false, gone: false, loading: true })
      return
    }
    for (const entry of children(dir)) {
      if (entry.ignored && !treeView.showIgnored) continue
      const isOpen = entry.dir && !!open[entry.path]
      out.push({
        key: entry.path,
        path: entry.path,
        name: entry.name,
        depth,
        dir: entry.dir,
        ignored: entry.ignored,
        link: entry.link,
        open: isOpen,
        gone: !!entry.gone
      })
      if (isOpen) visit(entry.path, depth + 1, !!entry.gone)
    }
  }
  visit('', 0, false)
  return out
})

// --- the filter

const found = ref<FoundFile[]>([])
const cursor = ref(0)
let filterTimer: ReturnType<typeof setTimeout> | undefined

const filtering = computed(() => !!session.value.filter.trim())

const foundRows = computed<Row[]>(() =>
  found.value.map((one) => {
    const cut = one.path.lastIndexOf('/')
    return {
      key: `found:${one.path}`,
      path: one.path,
      name: one.path.slice(cut + 1),
      folder: cut < 0 ? '' : one.path.slice(0, cut),
      hits: one.indices.filter((at) => at > cut).map((at) => at - cut - 1),
      depth: 0,
      dir: false,
      ignored: false,
      link: false,
      open: false,
      gone: false
    }
  })
)

const DRAFT = '\u0000draft'

/** A name being typed: a new file or folder in `parent`, or a rename of `path`. */
interface Editing {
  kind: 'file' | 'dir' | 'rename'
  parent: string
  path: string
  name: string
  dir: boolean
}

const editing = ref<Editing | null>(null)

/** The rows on screen, with the one a new name is being typed in. */
const rows = computed<Row[]>(() => {
  const base = filtering.value ? foundRows.value : treeRows.value
  const edit = editing.value
  if (!edit || edit.kind === 'rename' || filtering.value) return base
  const at = edit.parent ? base.findIndex((row) => row.dir && row.path === edit.parent) : -1
  if (edit.parent && at < 0) return base
  const draft: Row = {
    key: DRAFT,
    path: edit.parent,
    name: '',
    depth: at < 0 ? 0 : base[at]!.depth + 1,
    dir: edit.kind === 'dir',
    ignored: false,
    link: false,
    open: false,
    gone: false,
    draft: true
  }
  return [...base.slice(0, at + 1), draft, ...base.slice(at + 1)]
})

async function runFilter() {
  const query = session.value.filter.trim()
  if (!query) {
    found.value = []
    return
  }
  const answer = await findFiles(query, 200)
  // Overtaken by a newer search, or by an empty box.
  if (answer === null || session.value.filter.trim() !== query) return
  found.value = answer
  cursor.value = 0
}

watch(
  () => session.value.filter,
  () => {
    clearTimeout(filterTimer)
    filterTimer = setTimeout(() => void runFilter(), 80)
  }
)

/** A name with the characters the filter matched picked out. */
function pieces(row: Row, at = -1) {
  const speed = speedHits.value.get(at)?.indices
  const hits = speed ?? row.hits
  if (!hits?.length) return [{ text: row.name, hit: false }]
  const hit = new Set(hits)
  const out: { text: string; hit: boolean }[] = []
  for (const [at, char] of [...row.name].entries()) {
    const on = hit.has(at)
    const last = out[out.length - 1]
    if (last && last.hit === on) last.text += char
    else out.push({ text: char, hit: on })
  }
  return out
}

function onFilterKey(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.preventDefault()
    session.value.filter = ''
  } else if (event.key === 'ArrowDown' && filtering.value) {
    event.preventDefault()
    cursor.value = Math.min(cursor.value + 1, Math.max(0, found.value.length - 1))
    scrollTo(cursor.value)
  } else if (event.key === 'ArrowUp' && filtering.value) {
    event.preventDefault()
    cursor.value = Math.max(0, cursor.value - 1)
    scrollTo(cursor.value)
  } else if (event.key === 'Enter' && filtering.value) {
    event.preventDefault()
    const row = foundRows.value[cursor.value]
    if (row) open(row, false)
  }
}

// --- the list, drawn a screenful at a time

const ROW = 26
const list = ref<HTMLElement | null>(null)
const top = ref(0)
const height = ref(0)
let sizer: ResizeObserver | null = null

const visible = computed(() => {
  // Before the box has a size (and in tests, where it never does) a
  // screenful is assumed rather than nothing drawn.
  const view = height.value || 800
  const first = Math.max(0, Math.floor(top.value / ROW) - 10)
  const last = Math.min(rows.value.length, Math.ceil((top.value + view) / ROW) + 10)
  return rows.value.slice(first, last).map((row, offset) => ({ row, at: first + offset }))
})

function onScroll() {
  const box = list.value
  if (!box) return
  top.value = box.scrollTop
  if (!filtering.value) session.value.scroll = box.scrollTop
}

/** Brings a row into view, in the middle when it was off screen. */
function scrollTo(index: number) {
  const box = list.value
  if (!box || index < 0) return
  const y = index * ROW
  const view = box.clientHeight || height.value
  if (y < box.scrollTop || y + ROW > box.scrollTop + view) {
    box.scrollTop = Math.max(0, y - view / 2)
    top.value = box.scrollTop
  }
}

/** Keeps a row on screen with as little scrolling as it takes. */
function keepInView(index: number) {
  const box = list.value
  if (!box || index < 0) return
  const y = index * ROW
  const view = box.clientHeight || height.value
  if (y < box.scrollTop) box.scrollTop = y
  else if (view && y + ROW > box.scrollTop + view) box.scrollTop = y + ROW - view
  top.value = box.scrollTop
}

// --- the keyboard
//
// The tree takes the focus like a list in an IDE: a selected row of its own
// (ringed, and not the open file's highlight), the arrows to walk and fold,
// ↵ to open or fold, Space for a preview tab. Typing a name with the tree
// focused is a speed search over the rows on screen: the letters are marked
// where they match and the selection goes to the best match, without asking
// the backend anything. The filter box over the tree is something else: it
// asks the backend to narrow the whole tree.

/** The row the keyboard is on, by its key, so it survives folding above it. */
const selected = ref<string | null>(null)
const focused = ref(false)

const selectedAt = computed(() => {
  const key = selected.value
  if (key === null) return -1
  return rows.value.findIndex((row) => row.key === key)
})

function select(index: number) {
  const row = rows.value[index]
  if (!row) return
  selected.value = row.key
  if (filtering.value) cursor.value = index
  keepInView(index)
}

/** Where the keyboard starts: where it was, else the open file, else the top. */
function startAt() {
  if (selectedAt.value >= 0) return selectedAt.value
  const open = rows.value.findIndex((row) => row.path === current.value && !row.dir)
  return open >= 0 ? open : 0
}

const query = ref('')
/** The last character typed matched nothing: the overlay says so in red. */
const missed = ref(false)
let idleTimer: ReturnType<typeof setTimeout> | undefined

/** Every row the speed search matches, by index. */
const speedHits = computed(() => {
  const out = new Map<number, SpeedMatch>()
  if (!query.value) return out
  rows.value.forEach((row, at) => {
    if (row.loading) return
    const match = speedMatch(row.name, query.value)
    if (match) out.set(at, match)
  })
  return out
})

function endQuery() {
  clearTimeout(idleTimer)
  query.value = ''
  missed.value = false
}

function touchQuery() {
  clearTimeout(idleTimer)
  idleTimer = setTimeout(endQuery, 5000)
}

/** The query changed: to the best match at or after the selection, wrapping. */
function jumpToMatch(next: string) {
  const from = Math.max(0, startAt())
  let best: { at: number; tier: number; distance: number } | null = null
  const count = rows.value.length
  rows.value.forEach((row, at) => {
    if (row.loading) return
    const match = speedMatch(row.name, next)
    if (!match) return
    const distance = (at - from + count) % count
    if (!best || match.tier > best.tier || (match.tier === best.tier && distance < best.distance)) {
      best = { at, tier: match.tier, distance }
    }
  })
  const found = best as { at: number } | null
  if (!found) {
    // Nothing has it: the letter stays, in red, and the selection stays put.
    query.value = next
    missed.value = true
    touchQuery()
    return
  }
  query.value = next
  missed.value = false
  select(found.at)
  touchQuery()
}

/** ↑ and ↓ during a speed search: the matches only, round and round. */
function stepMatch(by: 1 | -1) {
  const hits = [...speedHits.value.keys()]
  if (!hits.length) return
  const from = selectedAt.value
  const ahead = by > 0 ? hits.find((at) => at > from) ?? hits[0]! : [...hits].reverse().find((at) => at < from) ?? hits[hits.length - 1]!
  select(ahead)
  touchQuery()
}

/** The folder something lands in when this row is the target: itself for a
    folder, the one it is in for a file. */
function folderFor(row: Row | null | undefined): string {
  if (!row) return ''
  if (row.draft) return row.path
  return row.dir && !row.gone ? row.path : parentOf(row.path)
}

/** What the keys that change files do, on the selected row. True if one did. */
function editKey(event: KeyboardEvent): boolean {
  if (query.value || event.altKey) return false
  const mod = event.metaKey || event.ctrlKey
  const row = rows.value[selectedAt.value]
  const usable = !!row && !row.loading && !row.draft && !row.gone
  if (event.key === 'F2' && !mod) {
    if (usable) void startRename(row!)
    return true
  }
  if (event.key === 'Delete' || (mod && event.key === 'Backspace')) {
    if (usable) askDelete([row!])
    return true
  }
  if (!mod || event.shiftKey) return false
  switch (event.key.toLowerCase()) {
    case 'c':
      if (usable) clip('copy', [row!.path])
      return true
    case 'x':
      if (usable) clip('cut', [row!.path])
      return true
    case 'v':
      void pasteInto(folderFor(row))
      return true
  }
  return false
}

function onTreeKey(event: KeyboardEvent) {
  // Keys in the filter box, or anything else that takes text, are its own.
  if (event.target !== list.value) return
  if (editKey(event)) {
    event.preventDefault()
    event.stopPropagation()
    return
  }
  if (event.metaKey || event.ctrlKey || event.altKey) return
  const count = rows.value.length
  if (!count) return
  const at = startAt()
  const row = rows.value[at]
  let handled = true
  switch (event.key) {
    case 'ArrowDown':
      if (query.value) stepMatch(1)
      else select(selectedAt.value < 0 ? at : Math.min(count - 1, at + 1))
      break
    case 'ArrowUp':
      if (query.value) stepMatch(-1)
      else select(selectedAt.value < 0 ? at : Math.max(0, at - 1))
      break
    case 'ArrowRight':
      endQuery()
      if (row?.dir && !row.open) void toggle(row)
      else if (row?.dir) select(at + 1 < count && rows.value[at + 1]!.depth > row.depth ? at + 1 : at)
      else select(at)
      break
    case 'ArrowLeft': {
      endQuery()
      if (row?.dir && row.open) {
        void toggle(row)
        select(at)
        break
      }
      const parent = row ? parentOf(row.path) : ''
      const up = parent ? rows.value.findIndex((one) => one.dir && one.path === parent) : -1
      select(up >= 0 ? up : at)
      break
    }
    case 'Home':
      endQuery()
      select(0)
      break
    case 'End':
      endQuery()
      select(count - 1)
      break
    case 'PageDown':
    case 'PageUp': {
      endQuery()
      const page = Math.max(1, Math.floor((list.value?.clientHeight || height.value || 800) / ROW) - 1)
      select(Math.min(count - 1, Math.max(0, at + (event.key === 'PageDown' ? page : -page))))
      break
    }
    case 'Enter':
      endQuery()
      if (row) {
        select(at)
        if (row.dir) void toggle(row)
        else open(row, false)
      }
      break
    case 'Escape':
      // The first Esc ends the speed search; the next is the window's.
      if (!query.value) {
        handled = false
        break
      }
      endQuery()
      break
    case 'Backspace':
      if (!query.value) {
        handled = false
        break
      }
      if (query.value.length > 1) jumpToMatch(query.value.slice(0, -1))
      else endQuery()
      break
    case '*':
      // Opens everything in the folder, as in the IDEs; inside a speed search
      // it is only another character.
      if (query.value) {
        jumpToMatch(query.value + '*')
        break
      }
      if (row) select(at)
      if (row?.dir && !row.loading) void expandAll(row)
      break
    case ' ':
      if (query.value) {
        jumpToMatch(query.value + ' ')
        break
      }
      if (row) {
        select(at)
        if (row.dir) void toggle(row)
        else open(row, true)
      }
      break
    default:
      if (event.key.length === 1) jumpToMatch(query.value + event.key)
      else handled = false
  }
  if (handled) {
    event.preventDefault()
    // Nothing else hears it: not the viewer's ↑ ↓ and Esc, not a shortcut.
    event.stopPropagation()
  }
}

function onTreeFocus() {
  focused.value = true
  if (selectedAt.value < 0 && rows.value.length) selected.value = rows.value[startAt()]!.key
}

function onTreeBlur() {
  focused.value = false
  endQuery()
}

// The folder ⌘⇧F searches inside: the selected row, while the tree has the
// focus and the row is a folder.
watch(
  () => {
    const row = focused.value ? rows.value[selectedAt.value] : undefined
    return row?.dir && !row.loading ? row.path : null
  },
  (dir) => {
    treeFolder.dir = dir
  },
  { immediate: true }
)

onBeforeUnmount(() => {
  treeFolder.dir = null
})

// --- opening things

/** The file on screen, when it is one from the work tree. */
const current = computed(() => {
  const tab = tabs.activeTab()
  return tab && !tab.commit ? tab.path : null
})

/**
 * Opens or shuts a folder. Shutting one shuts everything open inside it, so
 * opening it again shows it folded. Opening one that holds a single path of
 * folders down to a single file opens the whole path, as `Setup/Patch/Data`
 * with one file at the bottom would otherwise take three clicks to reach.
 */
async function toggle(row: Row) {
  const open = session.value.open
  if (open[row.path]) {
    shut(row.path)
    return
  }
  open[row.path] = true
  if (row.gone) return
  if (!(row.path in here.value)) await loadDir(row.path)
  await followChain(row.path)
}

/** Shuts a folder and every folder open inside it. */
function shut(path: string) {
  const open = session.value.open
  delete open[path]
  const inside = `${path}/`
  for (const dir of Object.keys(open)) {
    if (dir.startsWith(inside)) delete open[dir]
  }
}

/** How deep a single path of folders is followed before giving up. */
const CHAIN_LIMIT = 32

/**
 * Opens the folders below a just-opened one when each holds exactly one thing
 * and the last of them holds exactly one file. Anything else (two entries
 * anywhere on the way, an empty folder, a link) leaves it as it was.
 */
async function followChain(top: string) {
  const open = session.value.open
  const chain: string[] = []
  let dir = top
  for (let step = 0; ; step++) {
    if (step >= CHAIN_LIMIT) return
    if (!(dir in here.value)) await loadDir(dir)
    const inside = children(dir)
    if (inside.length !== 1) return
    const only = inside[0]!
    if (!only.dir) break
    if (only.link || only.gone) return
    chain.push(only.path)
    dir = only.path
  }
  // Shut again while the folders below were being read: it stays shut.
  if (!open[top]) return
  for (const one of chain) open[one] = true
}

/** The most folders "Open all inside" opens, so a vendor tree cannot stall it. */
const EXPAND_LIMIT = 500

/**
 * Opens a folder and every folder in it, all the way down. Ignored folders
 * inside are left shut unless the folder itself is ignored, and links are
 * never followed, so it cannot run in circles.
 */
async function expandAll(row: Row) {
  const open = session.value.open
  const keepIgnored = row.ignored
  const gone = new Set<string>(row.gone ? [row.path] : [])
  let budget = EXPAND_LIMIT
  let level = [row.path]
  let cut = false
  while (level.length) {
    for (const dir of level) open[dir] = true
    budget -= level.length
    await Promise.all(
      level.filter((dir) => !gone.has(dir) && !(dir in here.value)).map((dir) => loadDir(dir))
    )
    const next: string[] = []
    for (const dir of level) {
      if (!open[dir]) continue
      for (const one of children(dir)) {
        if (!one.dir || one.link || (one.ignored && !keepIgnored)) continue
        if (one.gone) gone.add(one.path)
        next.push(one.path)
      }
    }
    if (next.length > budget) cut = true
    level = next.slice(0, Math.max(0, budget))
  }
  if (cut) git.note(`Opened the first ${EXPAND_LIMIT} folders in ${row.path}`)
}

/**
 * Opens a file: a preview tab on a single click, a kept one on a double.
 * From the tree it is the file that is wanted, so File mode; a deleted file
 * has no file left and opens on its diff.
 */
function open(row: Row, preview: boolean) {
  if (row.loading || isEditing(row)) return
  if (row.dir) {
    if (!preview) return
    void toggle(row)
    return
  }
  const mark = marks.value.get(row.path)
  if (row.gone) {
    openTab({ path: row.path, mode: 'diff', side: sideOf(mark) }, { preview })
    return
  }
  openTab({ path: row.path, mode: 'file', side: sideOf(mark) }, { preview })
}

/** The side a file's changes are on: a file with only staged ones reads from
    the index, so its bar and its tally are about what is really there. */
function sideOf(mark: Mark | undefined): 'staged' | 'unstaged' {
  return mark && mark.staged && !mark.unstaged ? 'staged' : 'unstaged'
}

/**
 * A click on a row. The second click of a double click on a folder is not a
 * second toggle: that opened it and shut it again.
 */
function onClick(event: MouseEvent, row: Row) {
  if (isEditing(row)) return
  endQuery()
  selected.value = row.key
  list.value?.focus({ preventScroll: true })
  if (row.dir && event.detail > 1) return
  open(row, true)
}

/** Opens every folder down to a file and puts it on screen. */
async function reveal(path: string) {
  const open = session.value.open
  const dirs = ancestors(path)
  for (const dir of dirs) open[dir] = true
  await Promise.all(dirs.filter((dir) => !(dir in here.value)).map((dir) => loadDir(dir)))
  if (filtering.value) return
  await nextTick()
  scrollTo(rows.value.findIndex((row) => row.path === path && !row.draft))
}

watch(current, (path) => {
  if (path) void reveal(path)
})

watch(
  () => session.value.reveal?.seq,
  () => {
    const asked = session.value.reveal
    if (asked) void reveal(asked.path)
  }
)

// --- naming: new files and folders, renames

/** Whether this row is the one a name is being typed in. */
function isEditing(row: Row) {
  const edit = editing.value
  if (!edit) return false
  if (row.draft) return true
  return edit.kind === 'rename' && row.path === edit.path
}

/** Puts the name box on screen and the caret in it, the name picked out. */
async function focusEdit() {
  await nextTick()
  const at = rows.value.findIndex((row) => isEditing(row))
  if (at >= 0) scrollTo(at)
  await nextTick()
  const box = list.value?.querySelector<HTMLInputElement>('input.edit')
  if (!box) return
  box.focus({ preventScroll: true })
  // A rename picks out the name and leaves the extension, as Finder does.
  const dot = box.value.lastIndexOf('.')
  if (editing.value?.kind === 'rename' && !editing.value.dir && dot > 0) box.setSelectionRange(0, dot)
  else box.select()
}

/** Starts naming a new file or folder inside `parent` (`''` is the root). */
async function startCreate(parent: string, dir: boolean) {
  endQuery()
  if (filtering.value) session.value.filter = ''
  if (parent) {
    const open = session.value.open
    const dirs = [...ancestors(parent), parent]
    for (const one of dirs) open[one] = true
    await Promise.all(dirs.filter((one) => !(one in here.value)).map((one) => loadDir(one)))
  }
  editing.value = { kind: dir ? 'dir' : 'file', parent, path: '', name: '', dir }
  await focusEdit()
}

async function startRename(row: Row) {
  if (row.loading || row.draft || row.gone) return
  endQuery()
  editing.value = {
    kind: 'rename',
    parent: parentOf(row.path),
    path: row.path,
    name: nameOf(row.path),
    dir: row.dir
  }
  await focusEdit()
}

// The row being renamed, or the folder a name is typed in, gone from under
// the box (a checkout, a reload): there is nothing left to name.
watch(rows, (now) => {
  if (editing.value && !settling && !now.some((row) => isEditing(row))) editing.value = null
})

/** Set while a name is being acted on, so the blur that follows Enter does
    not act on it twice. */
let settling = false

async function commitEdit() {
  const edit = editing.value
  if (!edit || settling) return
  const name = edit.name.trim().replace(/^\/+|\/+$/g, '')
  if (!name || (edit.kind === 'rename' && name === nameOf(edit.path))) {
    cancelEdit()
    return
  }
  settling = true
  editing.value = null
  try {
    const done =
      edit.kind === 'rename'
        ? await renameEntry(edit.path, joinPath(edit.parent, name))
        : await createEntry(edit.parent, name, edit.kind === 'dir')
    if (done === null) {
      // Refused (taken, not a name): the box comes back with what was typed.
      editing.value = { ...edit, name }
      await focusEdit()
      return
    }
    await nextTick()
    const at = rows.value.findIndex((row) => row.path === done && !row.draft)
    if (at >= 0) select(at)
    if (edit.kind !== 'file') list.value?.focus({ preventScroll: true })
  } finally {
    settling = false
  }
}

function cancelEdit() {
  editing.value = null
  list.value?.focus({ preventScroll: true })
}

function onEditKey(event: KeyboardEvent) {
  // The tree's own keys (speed search, arrows) are not for the name box.
  event.stopPropagation()
  if (event.key === 'Enter') {
    event.preventDefault()
    void commitEdit()
  } else if (event.key === 'Escape') {
    event.preventDefault()
    cancelEdit()
  }
}

// --- dragging, inside the tree and in from outside

/** The folder a drag would drop into, `''` for the root; null for none. */
const dropAt = ref<string | null>(null)
let springTimer: ReturnType<typeof setTimeout> | undefined

function onDragStart(event: DragEvent, row: Row) {
  if (row.loading || row.draft || row.gone || isEditing(row)) {
    event.preventDefault()
    return
  }
  drag.begin(event, { kind: 'tree', paths: [row.path] })
  // ⌥ while dropping copies rather than moves.
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'copyMove'
}

function onDragEnd() {
  drag.end()
  dropAt.value = null
  clearTimeout(springTimer)
}

/** Whether what is dragged may land in `dest`. */
function droppable(event: DragEvent, dest: string) {
  const payload = drag.state.payload
  if (payload?.kind === 'tree') {
    if (payload.paths.some((path) => within(path, dest) && path !== dest)) return false
    if (payload.paths.some((path) => path === dest)) return false
    // Moving a file into the folder it is in is nothing; copying it is a duplicate.
    return event.altKey || payload.paths.some((path) => parentOf(path) !== dest)
  }
  return !payload && carriesFiles(event.dataTransfer)
}

function onDragOver(event: DragEvent, row: Row | null) {
  // A row answers for itself, yes or no: a refusal must not fall through to
  // the list underneath, which would take it as a drop on the root.
  if (row) event.stopPropagation()
  const dest = folderFor(row)
  if (row?.loading || editing.value || !droppable(event, dest)) {
    dropAt.value = null
    clearTimeout(springTimer)
    return
  }
  event.preventDefault()
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = drag.state.payload && !event.altKey ? 'move' : 'copy'
  }
  if (dropAt.value === dest) return
  dropAt.value = dest
  // Held over a shut folder, it opens, so a drop can go deeper.
  clearTimeout(springTimer)
  if (row?.dir && !row.open && !row.gone && !row.draft) {
    springTimer = setTimeout(() => {
      if (dropAt.value === row.path) void toggle(row)
    }, 700)
  }
}

function onDragLeave(event: DragEvent) {
  const box = list.value
  const entering = event.relatedTarget as Node | null
  if (box && entering && box.contains(entering)) return
  dropAt.value = null
  clearTimeout(springTimer)
}

async function onDrop(event: DragEvent, row: Row | null) {
  if (row) event.stopPropagation()
  const dest = folderFor(row)
  dropAt.value = null
  clearTimeout(springTimer)
  if (row?.loading || editing.value || !droppable(event, dest)) {
    drag.end()
    return
  }
  const ours = drag.take(['tree'])
  if (ours) {
    event.preventDefault()
    event.stopPropagation()
    if (event.altKey) await copyEntries(ours.paths, dest)
    else await moveEntries(ours.paths, dest)
    return
  }
  if (!carriesFiles(event.dataTransfer)) return
  event.preventDefault()
  event.stopPropagation()
  // Read now: the list is emptied once this handler returns.
  const items = droppedItems(event.dataTransfer)
  if (items.length) await importDropped(items, dest)
}

function isCut(row: Row) {
  return (
    treeClipboard.mode === 'cut' &&
    treeClipboard.repo === store.repo?.path &&
    treeClipboard.paths.includes(row.path)
  )
}

// --- menus

/** Asked first: what the tree is about to throw away. */
const confirming = ref<{ title: string; text: string; act: string; run: () => unknown } | null>(
  null
)

/** The row whose menu is open, ringed so it is clear what the menu is about. */
const menuRow = ref<string | null>(null)

watch(
  () => menu.state.open,
  (open) => {
    if (!open) menuRow.value = null
  }
)

const TRASH = typeof navigator !== 'undefined' && navigator.userAgent.includes('Windows') ? 'Recycle Bin' : 'Trash'

function plural(count: number, word: string) {
  return `${count} ${word}${count === 1 ? '' : 's'}`
}

async function history(event: MouseEvent, path: string, folder = false) {
  const commits = await git.fileHistory(path, 31, folder)
  if (!commits?.length) {
    git.note(`No commits touch ${path} yet`)
    return
  }
  const items: MenuItem[] = commits.slice(0, 30).map((one) => ({
    label: one.summary || one.short,
    hint: `${one.short} · ${relativeTime(one.time)}`,
    // A file opens on what that commit did to it; a folder's commit is
    // shown in the graph, with every file it touched.
    action: () => (folder ? git.revealCommit(one.oid) : openTab({ path, commit: one.oid, mode: 'diff' }))
  }))
  menu.show(event, items, folder ? `${path}/` : path)
}

function askDelete(targets: Row[]) {
  const paths = targets.map((row) => row.path)
  const one = targets.length === 1 ? targets[0]! : null
  const what = one ? (one.dir ? `${one.path}/ and everything in it` : one.path) : `${paths.length} items`
  confirming.value = {
    title: one ? `Delete ${one.name}?` : `Delete ${paths.length} items?`,
    text: `${what} ${one ? 'goes' : 'go'} to the ${TRASH}. What git tracks shows up as deleted, so discarding brings it back; undo in the history menu brings back the rest too, except what git ignores, which only the ${TRASH} keeps.`,
    act: `Move to ${TRASH}`,
    run: () => deleteEntries(paths)
  }
}

/** New, cut, copy, paste, duplicate, rename, delete: the same for any row. */
function editItems(row: Row): MenuItem[] {
  const here = folderFor(row)
  const waiting = canPaste() ? treeClipboard.paths : []
  const pasteLabel = !waiting.length
    ? 'Paste'
    : waiting.length === 1
      ? `Paste ${nameOf(waiting[0]!)}`
      : `Paste ${waiting.length} items`
  return [
    { label: 'New file…', icon: FilePlus, action: () => startCreate(here, false) },
    { label: 'New folder…', icon: FolderPlus, action: () => startCreate(here, true) },
    { separator: true, label: '' },
    { label: 'Cut', icon: Scissors, hint: keyLabel('mod+x'), disabled: row.gone, action: () => clip('cut', [row.path]) },
    { label: 'Copy', icon: Files, hint: keyLabel('mod+c'), disabled: row.gone, action: () => clip('copy', [row.path]) },
    {
      label: pasteLabel,
      icon: ClipboardPaste,
      hint: keyLabel('mod+v'),
      disabled: !waiting.length,
      action: () => pasteInto(here)
    },
    { label: 'Duplicate', icon: CopyPlus, disabled: row.gone, action: () => duplicateEntry(row.path) },
    { label: 'Rename…', icon: Pencil, hint: 'F2', disabled: row.gone, action: () => startRename(row) },
    {
      label: 'Delete…',
      icon: Trash2,
      hint: keyLabel('mod+backspace'),
      danger: true,
      disabled: row.gone,
      action: () => askDelete([row])
    }
  ]
}

async function ignore(pattern: string, local: boolean, row: Row, tracked: boolean) {
  const said = await git.addToGitignore(pattern, local)
  if (said === null) return
  git.note(said)
  if (tracked) {
    useToasts().offer(said, [{ label: 'Stop tracking', run: () => untrack(row) }],
      `Git still tracks ${row.path}, and a rule only keeps out what it does not track yet.`)
  } else useToasts().info(said)
}

async function untrack(row: Row) {
  const said = await git.untrackPaths([row.path])
  if (said !== null) useToasts().info(said)
}

/** Add to .gitignore, and Stop tracking when git tracks it. */
function gitItems(row: Row, tracked: boolean): MenuItem[] {
  const items: MenuItem[] = []
  if (!row.ignored && !row.gone) {
    const choices = ignoreChoices(row.path, row.dir)
    const children: MenuItem[] = choices.map((one) => ({
      label: one.label,
      hint: one.pattern,
      action: () => ignore(one.pattern, false, row, tracked)
    }))
    children.push(
      { separator: true, label: '' },
      {
        label: 'Only on this machine',
        hint: '.git/info/exclude',
        action: () => ignore(choices[0]!.pattern, true, row, tracked)
      }
    )
    items.push({ label: 'Add to .gitignore', icon: EyeOff, children })
  }
  if (tracked && !row.gone) {
    items.push({ label: 'Stop tracking', icon: FileX, hint: 'keep on disk', action: () => untrack(row) })
  }
  return items
}

function fileMenu(event: MouseEvent, row: Row) {
  const mark = marks.value.get(row.path)
  const conflicted = mark?.kind === 'clash'
  const items: MenuItem[] = [
    { label: 'Open', icon: FileText, action: () => open(row, false) },
    {
      label: 'Open diff',
      icon: FileDiff,
      disabled: !mark,
      hint: mark ? undefined : 'no changes',
      action: () =>
        openTab({ path: row.path, mode: 'diff', side: mark?.unstaged ? 'unstaged' : 'staged' })
    },
    { separator: true, label: '' }
  ]
  if (mark?.unstaged && !conflicted) {
    items.push({ label: 'Stage', icon: Plus, action: () => git.stage([row.path]) })
  }
  if (mark?.staged) {
    items.push({ label: 'Unstage', icon: Minus, action: () => git.unstage([row.path]) })
  }
  if (mark?.unstaged && !conflicted && mark.unstagedKind !== 'added') {
    items.push({
      label: 'Discard changes…',
      icon: Undo2,
      danger: true,
      action: () => {
        confirming.value = {
          title: 'Discard these changes?',
          text: `The unstaged changes to ${row.path} go back to what is staged or committed. Anything staged stays staged.`,
          act: 'Discard',
          run: () => git.discard([row.path])
        }
      }
    })
  }
  if (items[items.length - 1]?.separator !== true) items.push({ separator: true, label: '' })
  const untracked = (store.status?.unstaged ?? []).some(
    (one) => one.path === row.path && one.kind === 'untracked'
  )
  items.push(...editItems(row), { separator: true, label: '' })
  items.push(
    { label: 'File history', icon: History, action: () => history(event, row.path) },
    {
      label: 'Blame',
      icon: Users,
      disabled: row.gone,
      action: () => {
        openTab({ path: row.path, mode: 'file', side: sideOf(mark) })
        diffMode.blame = true
      }
    }
  )
  const gitOnes = gitItems(row, !untracked && !row.ignored)
  if (gitOnes.length) items.push({ separator: true, label: '' }, ...gitOnes)
  items.push(
    { separator: true, label: '' },
    { label: 'Copy path', icon: Copy, action: () => copyText(row.path, 'Path') },
    { label: git.revealLabel, icon: FolderOpen, disabled: row.gone, action: () => git.reveal(row.path) }
  )
  menu.show(event, items, row.path)
}

/** A round swatch of a colour, where a menu item has its icon. */
function swatch(hex: string | null): FunctionalComponent {
  return () =>
    h('span', {
      class: 'icon',
      style: {
        display: 'inline-block',
        width: '12px',
        height: '12px',
        margin: '1px',
        borderRadius: '50%',
        boxSizing: 'border-box',
        background: hex ?? 'transparent',
        border: hex ? 'none' : '1.5px solid currentColor'
      }
    })
}

/** What the status lists inside a folder, by what can be done with it. */
function changesUnder(dir: string) {
  const status = store.status
  const conflicted = new Set(status?.conflicted ?? [])
  const inside = (path: string) => within(dir, path) && !conflicted.has(path)
  const unique = (list: string[]) => [...new Set(list)]
  const unstaged = (status?.unstaged ?? []).filter((one) => inside(one.path))
  return {
    stage: unique(unstaged.map((one) => one.path)),
    unstage: unique((status?.staged ?? []).filter((one) => inside(one.path)).map((one) => one.path)),
    discard: unique(unstaged.filter((one) => change(one.kind) !== 'added').map((one) => one.path))
  }
}

function dirMenu(event: MouseEvent, row: Row) {
  const repo = store.repo?.path
  const now = folderColor(repo, row.path)?.id ?? null
  const colors: MenuItem[] = FOLDER_COLORS.map((one) => ({
    label: one.label,
    icon: swatch(one.hex),
    hint: one.id === now ? '✓' : undefined,
    action: () => setFolderColor(repo, row.path, one.id)
  }))
  colors.push(
    { separator: true, label: '' },
    {
      label: 'No colour',
      icon: swatch(null),
      disabled: now === null,
      action: () => setFolderColor(repo, row.path, null)
    }
  )
  const items: MenuItem[] = [
    { label: row.open ? 'Close' : 'Open', icon: Folder, action: () => toggle(row) },
    { label: 'Open all inside', icon: ChevronsDown, hint: '*', action: () => expandAll(row) },
    {
      label: 'Close all inside',
      icon: ChevronsUp,
      disabled: !row.open,
      action: () => shut(row.path)
    },
    {
      label: 'Search in folder…',
      icon: Search,
      hint: keyLabel(keysFor('search.text')),
      action: () => openSearch('text', false, row.path)
    },
    { separator: true, label: '' }
  ]
  const changes = changesUnder(row.path)
  if (changes.stage.length) {
    items.push({
      label: `Stage ${plural(changes.stage.length, 'file')}`,
      icon: Plus,
      action: () => git.stage(changes.stage)
    })
  }
  if (changes.unstage.length) {
    items.push({
      label: `Unstage ${plural(changes.unstage.length, 'file')}`,
      icon: Minus,
      action: () => git.unstage(changes.unstage)
    })
  }
  if (changes.discard.length) {
    items.push({
      label: 'Discard changes…',
      icon: Undo2,
      danger: true,
      action: () => {
        confirming.value = {
          title: `Discard the changes in ${row.name}?`,
          text: `The unstaged changes to ${plural(changes.discard.length, 'file')} in ${row.path}/ go back to what is staged or committed. New files stay, and anything staged stays staged. Undo in the history menu brings them back.`,
          act: 'Discard',
          run: () => git.discard(changes.discard)
        }
      }
    })
  }
  if (items[items.length - 1]?.separator !== true) items.push({ separator: true, label: '' })
  items.push(
    ...editItems(row),
    { separator: true, label: '' },
    { label: 'Folder history', icon: FolderClock, action: () => history(event, row.path, true) },
    { label: 'Colour', icon: Palette, children: colors }
  )
  // Whether git tracks anything in the folder is not known here; the backend
  // says so when it does not.
  const gitOnes = gitItems(row, !row.ignored)
  if (gitOnes.length) items.push({ separator: true, label: '' }, ...gitOnes)
  items.push(
    { separator: true, label: '' },
    { label: 'Copy path', icon: Copy, action: () => copyText(row.path, 'Path') },
    { label: git.revealLabel, icon: FolderOpen, disabled: row.gone, action: () => git.reveal(row.path) }
  )
  menu.show(event, items, `${row.path}/`)
}

function onMenu(event: MouseEvent, row: Row) {
  if (row.loading || row.draft || isEditing(row)) {
    // Not the root's menu either.
    event.preventDefault()
    event.stopPropagation()
    return
  }
  if (row.dir) dirMenu(event, row)
  else fileMenu(event, row)
  menuRow.value = row.key
}

/** Shuts every folder. */
function closeAll() {
  for (const dir of Object.keys(session.value.open)) delete session.value.open[dir]
}

/** The menu of the empty space under the rows: the root folder's. */
function onListMenu(event: MouseEvent) {
  if (editing.value) return
  const waiting = canPaste() ? treeClipboard.paths : []
  menu.show(
    event,
    [
      { label: 'New file…', icon: FilePlus, action: () => startCreate('', false) },
      { label: 'New folder…', icon: FolderPlus, action: () => startCreate('', true) },
      {
        label: waiting.length === 1 ? `Paste ${nameOf(waiting[0]!)}` : waiting.length ? `Paste ${waiting.length} items` : 'Paste',
        icon: ClipboardPaste,
        hint: keyLabel('mod+v'),
        disabled: !waiting.length,
        action: () => pasteInto('')
      },
      { separator: true, label: '' },
      {
        label: 'Show ignored files',
        icon: treeView.showIgnored ? Eye : EyeOff,
        hint: treeView.showIgnored ? '✓' : undefined,
        action: () => (treeView.showIgnored = !treeView.showIgnored)
      },
      { separator: true, label: '' },
      { label: 'Close all folders', icon: ChevronsDownUp, action: closeAll }
    ],
    store.repo?.name ?? 'Files'
  )
}

async function confirm() {
  const asked = confirming.value
  confirming.value = null
  await asked?.run()
}

// --- keeping up with the disk

let unlisten: UnlistenFn | undefined
let gone = false
let reloadTimer: ReturnType<typeof setTimeout> | undefined

/** Re-reads the open folders once a burst of changes has settled. */
function soon() {
  clearTimeout(reloadTimer)
  reloadTimer = setTimeout(() => void reloadOpen(), 150)
}

// Another project: its own folders, read if they have not been.
watch(
  () => store.repo?.path,
  () => void start()
)

async function start() {
  if (!store.repo) return
  await loadDir('')
  const open = Object.keys(session.value.open)
  await Promise.all(open.filter((dir) => !(dir in here.value)).map((dir) => loadDir(dir)))
  await nextTick()
  if (list.value) {
    list.value.scrollTop = session.value.scroll
    top.value = list.value.scrollTop
  }
  if (session.value.filter.trim()) void runFilter()
  // A file asked for before the tree was on screen ("Reveal in Files" with
  // the branches showing) comes before the one that happens to be open.
  const asked = session.value.reveal
  if (asked) void reveal(asked.path)
  else if (current.value) void reveal(current.value)
}

const failed = computed(() => !!store.repo && !!rootFailed[store.repo.path])

onMounted(async () => {
  const box = list.value
  if (box) {
    height.value = box.clientHeight
    sizer = new ResizeObserver(() => {
      height.value = box.clientHeight
    })
    sizer.observe(box)
  }
  void start()
  const off = await listen<{ work_tree: boolean }>('repo-changed', ({ payload }) => {
    if (payload.work_tree) soon()
  }).catch(() => undefined)
  if (gone) off?.()
  else unlisten = off
})

onBeforeUnmount(() => {
  gone = true
  sizer?.disconnect()
  unlisten?.()
  clearTimeout(filterTimer)
  clearTimeout(reloadTimer)
  clearTimeout(idleTimer)
  clearTimeout(springTimer)
})

/** The colour a folder was given, as styles for its row. */
function tint(row: Row) {
  if (!row.dir || row.loading) return undefined
  const color = folderColor(store.repo?.path, row.path)
  return color ? { '--folder-color': color.hex } : undefined
}

function markOf(row: Row) {
  return row.dir || row.draft ? null : (marks.value.get(row.path) ?? null)
}

function countOf(row: Row) {
  return row.dir && !row.draft ? (counts.value.get(row.path) ?? 0) : 0
}

function tone(row: Row) {
  if (row.draft) return ''
  if (row.dir) return folders.value.get(row.path) ?? ''
  return marks.value.get(row.path)?.kind ?? ''
}

function dot(mark: Mark | null) {
  if (!mark) return ''
  if (mark.staged && mark.unstaged) return 'both'
  return mark.staged ? 'staged' : 'unstaged'
}

const DOT_TITLE: Record<string, string> = {
  staged: 'Staged',
  unstaged: 'Changed, not staged',
  both: 'Partly staged'
}
</script>

<template>
  <div class="tree-pane">
    <div class="filter">
      <Search :size="13" class="faint" />
      <input
        v-model="session.filter"
        type="search"
        placeholder="Filter files"
        spellcheck="false"
        @keydown="onFilterKey"
      />
      <button
        type="button"
        class="toggle"
        :class="{ on: !treeView.showIgnored }"
        :aria-pressed="treeView.showIgnored"
        :title="treeView.showIgnored ? 'Hide ignored files' : 'Show ignored files'"
        aria-label="Show ignored files"
        @click="treeView.showIgnored = !treeView.showIgnored"
      >
        <Eye v-if="treeView.showIgnored" :size="13" />
        <EyeOff v-else :size="13" />
      </button>
    </div>

    <div class="list-wrap">
    <!-- The speed search: what has been typed, over the rows, moving none. -->
    <div v-if="query" class="speed" :class="{ missed }" role="status" aria-live="polite">
      <Search :size="11" />
      <span class="typed">{{ query }}</span>
    </div>
    <div
      ref="list"
      class="list"
      :class="{ 'drop-root': dropAt === '' }"
      role="tree"
      tabindex="0"
      aria-label="Files"
      :aria-activedescendant="selected !== null ? `tree-row-${selectedAt}` : undefined"
      @scroll.passive="onScroll"
      @keydown="onTreeKey"
      @focus="onTreeFocus"
      @blur="onTreeBlur"
      @contextmenu="onListMenu"
      @dragover="onDragOver($event, null)"
      @dragleave="onDragLeave"
      @drop="onDrop($event, null)"
    >
      <div class="space" :style="{ height: `${rows.length * ROW}px` }">
        <div
          v-for="{ row, at } in visible"
          :key="row.key"
          class="row"
          role="treeitem"
          :class="[
            tone(row),
            {
              dir: row.dir,
              ignored: row.ignored,
              gone: row.gone,
              loading: row.loading,
              on: !row.dir && row.path === current,
              cursor: filtering && at === cursor,
              selected: at === selectedAt,
              matched: speedHits.has(at),
              tinted: !!tint(row),
              draft: row.draft,
              editing: isEditing(row),
              'menu-on': row.key === menuRow,
              drop: row.dir && !row.loading && !row.draft && row.path === dropAt,
              cut: isCut(row)
            }
          ]"
          :style="{ top: `${at * ROW}px`, paddingLeft: `${8 + row.depth * 14}px`, ...tint(row) }"
          :id="`tree-row-${at}`"
          :data-path="row.path"
          :title="row.folder !== undefined ? row.path : undefined"
          :aria-level="row.depth + 1"
          :aria-expanded="row.dir ? row.open : undefined"
          :aria-selected="!row.dir && row.path === current"
          @click="onClick($event, row)"
          :draggable="!row.loading && !row.draft && !row.gone && !isEditing(row)"
          @dblclick="open(row, false)"
          @contextmenu="onMenu($event, row)"
          @dragstart="onDragStart($event, row)"
          @dragend="onDragEnd"
          @dragover="onDragOver($event, row)"
          @drop="onDrop($event, row)"
        >
          <ChevronRight v-if="row.dir" :size="12" class="chev" :class="{ down: row.open }" />
          <span v-else class="chev-space" />
          <FileIcon
            v-if="!row.loading"
            :path="row.draft ? editing?.name || 'untitled' : row.path"
            :folder="row.dir"
            :open="row.open"
          />
          <span v-else class="icon-space" />
          <input
            v-if="editing && isEditing(row)"
            v-model="editing.name"
            class="edit"
            spellcheck="false"
            autocomplete="off"
            :aria-label="editing.kind === 'rename' ? `New name for ${editing.path}` : editing.kind === 'dir' ? 'Folder name' : 'File name'"
            :placeholder="editing.kind === 'dir' ? 'Folder name' : editing.kind === 'file' ? 'File name, or path/to/file' : ''"
            @keydown="onEditKey"
            @blur="commitEdit"
            @click.stop
            @dblclick.stop
            @mousedown.stop
          />
          <span v-else class="name">
            <template v-for="(piece, index) in pieces(row, at)" :key="index">
              <mark v-if="piece.hit && query" class="speed-hit">{{ piece.text }}</mark>
              <b v-else-if="piece.hit">{{ piece.text }}</b>
              <template v-else>{{ piece.text }}</template>
            </template>
          </span>
          <span v-if="row.folder" class="folder-of">{{ row.folder }}</span>
          <span v-if="row.dir && row.ignored" class="tag">ignored</span>
          <span
            v-if="countOf(row)"
            class="count"
            :title="`${countOf(row)} changed ${countOf(row) === 1 ? 'file' : 'files'} inside`"
          >{{ countOf(row) }}</span>
          <span v-if="row.link" class="tag">link</span>
          <span
            v-if="markOf(row)"
            class="dot"
            :class="dot(markOf(row))"
            role="img"
            :aria-label="DOT_TITLE[dot(markOf(row))]"
            :title="DOT_TITLE[dot(markOf(row))]"
          />
        </div>
      </div>
      <p v-if="filtering && !found.length" class="empty">No files match.</p>
      <p v-else-if="!filtering && failed" class="empty">
        Could not read this folder.
        <button class="again" @click="loadDir('')">Try again</button>
      </p>
      <p v-else-if="!filtering && !rows.length" class="empty">No files here yet.</p>
    </div>
    </div>

    <AppModal
      v-if="confirming"
      :title="confirming.title"
      :width="420"
      @close="confirming = null"
    >
      <p class="ask">{{ confirming.text }}</p>
      <template #footer>
        <button class="btn btn-ghost" @click="confirming = null">Cancel</button>
        <button class="btn btn-danger" @click="confirm">{{ confirming.act }}</button>
      </template>
    </AppModal>
  </div>
</template>

<style scoped>
.tree-pane {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: auto minmax(0, 1fr);
  min-height: 0;
}

/* The same field as the branch filter, so switching lists moves nothing. */
.filter {
  display: flex;
  align-items: center;
  gap: 6px;
  height: var(--control-h);
  margin: 8px 8px 4px;
  padding: 0 8px;
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  color: var(--text-faint);
}

.filter:focus-within {
  border-color: var(--ring);
  box-shadow: var(--focus);
}

.filter input {
  flex: 1;
  min-width: 0;
  border: none;
  background: none;
  padding: 0;
  font-size: 12.5px;
}

/* The ignored-files switch at the end of the filter box. */
.toggle {
  flex: none;
  display: grid;
  place-items: center;
  width: 20px;
  height: 20px;
  margin-right: -4px;
  border-radius: var(--radius-sm);
  color: var(--text-faint);
}

.toggle:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.toggle.on {
  color: var(--accent-soft);
  background: var(--bg-active);
}

.list-wrap {
  position: relative;
  display: grid;
  grid-template-rows: minmax(0, 1fr);
  min-height: 0;
}

.list {
  position: relative;
  min-height: 0;
  overflow: auto;
  padding-bottom: 10px;
  outline: none;
}

/* The row the keyboard is on: a ring, while the tree has the focus, so it is
   not taken for the open file's highlight. */
.list:focus .row.selected,
.row.menu-on {
  box-shadow: inset 0 0 0 1px var(--ring);
}

/* Where a drag would land: the folder, or the whole list for the root. */
.row.drop,
.list.drop-root {
  background: color-mix(in oklab, var(--accent) 16%, transparent);
}

.list.drop-root {
  box-shadow: inset 0 0 0 1px var(--ring);
}

/* Cut, waiting for a paste. */
.row.cut {
  opacity: 0.5;
}

/* The name box, in a row's place, the size of the name it stands for. */
.edit {
  flex: 1;
  min-width: 0;
  height: 20px;
  padding: 0 4px;
  margin-left: -4px;
  font: inherit;
  font-size: 12.5px;
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--ring);
  border-radius: var(--radius-sm);
  box-shadow: var(--focus);
  user-select: text;
}

/* The speed search's query, small, over the top of the rows. */
.speed {
  position: absolute;
  z-index: 2;
  top: 2px;
  left: 8px;
  display: flex;
  align-items: center;
  gap: 4px;
  max-width: calc(100% - 16px);
  height: 20px;
  padding: 0 6px;
  font-size: 11.5px;
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--ring);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-pop);
  pointer-events: none;
}

.speed .typed {
  overflow: hidden;
  white-space: pre;
  text-overflow: ellipsis;
}

.speed.missed {
  color: var(--danger);
  border-color: var(--danger);
}

/* The search window's mark, on the letters the speed search matched. */
.name mark.speed-hit {
  background: var(--warning-bg);
  color: inherit;
  border-radius: 2px;
}

.space {
  position: relative;
}

.row {
  position: absolute;
  left: 6px;
  right: 6px;
  display: flex;
  align-items: center;
  gap: 5px;
  /* The height and the inset of a row in the branch list, so switching
     between the two lists moves nothing. */
  height: 26px;
  padding-right: 8px;
  font-size: 12.5px;
  color: var(--text);
  border-radius: var(--radius-sm);
  white-space: nowrap;
  cursor: default;
  user-select: none;
}

.row:hover {
  background: var(--bg-hover);
}

/* A folder given a colour: its icon in it, and a wash of it behind the row,
   mixed with the theme's text so it reads on light and dark alike. */
.row.tinted {
  background: color-mix(in oklab, var(--folder-color) 12%, transparent);
}

.row.tinted:hover {
  background: color-mix(in oklab, var(--folder-color) 20%, transparent);
}

.row.tinted :deep(.file-icon) {
  color: color-mix(in oklab, var(--folder-color) 80%, var(--fg));
}

.row.on,
.row.cursor {
  background: var(--bg-active);
}

.row.on .name {
  font-weight: 600;
}

.chev {
  flex: none;
  color: var(--text-faint);
  transition: transform 0.1s;
}

.chev.down {
  transform: rotate(90deg);
}

.chev-space {
  flex: none;
  width: 12px;
}

/* Where an icon would be, on the row that stands in for a folder being read. */
.icon-space {
  flex: none;
  width: 14px;
}

.again {
  color: var(--accent-soft);
  text-decoration: underline;
}

.name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.name b {
  font-weight: 700;
  color: var(--accent-soft);
}

.folder-of {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 11.5px;
  color: var(--text-faint);
}

.tag {
  flex: none;
  margin-left: auto;
  font-size: 10.5px;
  color: var(--text-faint);
}

/* The same colours the right panel gives a change. */
.row.modified {
  color: var(--amber);
}

.row.added {
  color: var(--green-soft);
}

.row.deleted {
  color: var(--red);
}

.row.renamed {
  color: var(--purple);
}

/* Amber for the mark and the folder, the softer one for the name, as the
   right panel has it. */
.row.clash {
  color: var(--amber);
}

.row.clash:not(.dir) .name {
  color: var(--amber-soft);
}

.row.gone .name {
  text-decoration: line-through;
}

.row.ignored {
  opacity: 0.5;
}

.row.loading {
  color: var(--text-faint);
  font-style: italic;
}

/* Filled for staged, a ring for unstaged, half and half for both. */
.dot {
  flex: none;
  width: 7px;
  height: 7px;
  margin-left: auto;
  border-radius: 50%;
  border: 1.5px solid currentColor;
}

.tag + .dot {
  margin-left: 4px;
}

/* How many changed files a folder holds, in its colour. */
.count {
  flex: none;
  margin-left: auto;
  min-width: 16px;
  padding: 0 4px;
  font-size: 10.5px;
  line-height: 15px;
  text-align: center;
  font-variant-numeric: tabular-nums;
  border-radius: 8px;
  background: color-mix(in oklab, currentColor 14%, transparent);
}

.tag + .count {
  margin-left: 4px;
}

.dot.staged {
  background: currentColor;
}

.dot.both {
  background: linear-gradient(90deg, currentColor 50%, transparent 50%);
}

.empty {
  padding: 8px 14px;
  font-size: 12px;
  color: var(--text-faint);
}

.ask {
  margin: 0;
  overflow-wrap: anywhere;
}
</style>
