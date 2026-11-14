<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  ChevronRight,
  Copy,
  FileDiff,
  FileText,
  Folder,
  FolderOpen,
  History,
  Minus,
  Plus,
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
import { openTab, useTabs } from '~/composables/useTabs'
import {
  ancestors,
  findFiles,
  listingsHere,
  loadDir,
  reloadOpen,
  rootFailed,
  treeSession,
  type DirEntry,
  type FoundFile
} from '~/composables/useFileTree'

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

const rows = computed(() => (filtering.value ? foundRows.value : treeRows.value))

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
function pieces(row: Row) {
  if (!row.hits?.length) return [{ text: row.name, hit: false }]
  const hit = new Set(row.hits)
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

// --- opening things

/** The file on screen, when it is one from the work tree. */
const current = computed(() => {
  const tab = tabs.activeTab()
  return tab && !tab.commit ? tab.path : null
})

async function toggle(row: Row) {
  const open = session.value.open
  if (open[row.path]) {
    delete open[row.path]
    return
  }
  open[row.path] = true
  if (!row.gone && !(row.path in here.value)) await loadDir(row.path)
}

/**
 * Opens a file: a preview tab on a single click, a kept one on a double.
 * From the tree it is the file that is wanted, so File mode; a deleted file
 * has no file left and opens on its diff.
 */
function open(row: Row, preview: boolean) {
  if (row.loading) return
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
  scrollTo(treeRows.value.findIndex((row) => row.path === path))
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

// --- menus

/** Asked first: what the tree is about to throw away. */
const confirming = ref<{ title: string; text: string; act: string; run: () => unknown } | null>(
  null
)

async function history(event: MouseEvent, path: string) {
  const commits = await git.fileHistory(path, 31)
  if (!commits?.length) {
    git.note(`No commits touch ${path} yet`)
    return
  }
  const items: MenuItem[] = commits.slice(0, 30).map((one) => ({
    label: one.summary || one.short,
    hint: `${one.short} · ${relativeTime(one.time)}`,
    action: () => openTab({ path, commit: one.oid, mode: 'diff' })
  }))
  menu.show(event, items, path)
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
  const untracked = (store.status?.unstaged ?? []).some(
    (one) => one.path === row.path && one.kind === 'untracked'
  )
  if (untracked) {
    items.push({
      label: 'Delete file…',
      icon: Trash2,
      danger: true,
      action: () => {
        confirming.value = {
          title: 'Delete this file?',
          text: `Git is not tracking ${row.path}, so it is only on disk. Undo in the history menu brings it back.`,
          act: 'Delete the file',
          run: () => git.deleteUntracked([row.path])
        }
      }
    })
  }
  if (items[items.length - 1]?.separator !== true) items.push({ separator: true, label: '' })
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
    },
    { separator: true, label: '' },
    { label: 'Copy path', icon: Copy, action: () => copyText(row.path, 'Path') },
    { label: git.revealLabel, icon: FolderOpen, disabled: row.gone, action: () => git.reveal(row.path) }
  )
  menu.show(event, items, row.path)
}

function dirMenu(event: MouseEvent, row: Row) {
  menu.show(
    event,
    [
      { label: row.open ? 'Close' : 'Open', icon: Folder, action: () => toggle(row) },
      { separator: true, label: '' },
      { label: 'Copy path', icon: Copy, action: () => copyText(row.path, 'Path') },
      { label: git.revealLabel, icon: FolderOpen, disabled: row.gone, action: () => git.reveal(row.path) }
    ],
    row.path
  )
}

function onMenu(event: MouseEvent, row: Row) {
  if (row.loading) return
  if (row.dir) dirMenu(event, row)
  else fileMenu(event, row)
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
})

function markOf(row: Row) {
  return row.dir ? null : (marks.value.get(row.path) ?? null)
}

function tone(row: Row) {
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
    </div>

    <div ref="list" class="list" role="tree" @scroll.passive="onScroll">
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
              cursor: filtering && at === cursor
            }
          ]"
          :style="{ top: `${at * ROW}px`, paddingLeft: `${8 + row.depth * 14}px` }"
          :data-path="row.path"
          :title="row.folder !== undefined ? row.path : undefined"
          :aria-level="row.depth + 1"
          :aria-expanded="row.dir ? row.open : undefined"
          :aria-selected="!row.dir && row.path === current"
          @click="onClick($event, row)"
          @dblclick="open(row, false)"
          @contextmenu="onMenu($event, row)"
        >
          <ChevronRight v-if="row.dir" :size="12" class="chev" :class="{ down: row.open }" />
          <span v-else class="chev-space" />
          <FileIcon v-if="!row.loading" :path="row.path" :folder="row.dir" :open="row.open" />
          <span v-else class="icon-space" />
          <span class="name">
            <template v-for="(piece, index) in pieces(row)" :key="index">
              <b v-if="piece.hit">{{ piece.text }}</b>
              <template v-else>{{ piece.text }}</template>
            </template>
          </span>
          <span v-if="row.folder" class="folder-of">{{ row.folder }}</span>
          <span v-if="row.dir && row.ignored" class="tag">ignored</span>
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

.list {
  position: relative;
  min-height: 0;
  overflow: auto;
  padding-bottom: 10px;
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
