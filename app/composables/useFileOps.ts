import { nextTick, reactive } from 'vue'
import { invoke } from './useInvoke'
import { useGit } from './useGit'
import { useToasts } from './useToasts'
import { closeTabsSafely, flushAll } from './useFileEdit'
import { activeTab, closeWhere, openTab, useTabs } from './useTabs'
import type { DiffMode } from './useDiffMode'
import { ancestors, reloadOpen, treeSession } from './useFileTree'

/**
 * What the Files tree does to the files themselves: new, rename, move, copy,
 * delete, paste and files dropped in from outside.
 *
 * The backend does the work (`fileops.rs`). What is here is the window's side
 * of it: open files are saved and their tabs let go of before anything under
 * them moves, and come back under the new name after; the folders that were
 * open stay open where they went.
 */

export function joinPath(dir: string, name: string) {
  const clean = dir.replace(/^\/+|\/+$/g, '')
  return clean ? `${clean}/${name}` : name
}

export function parentOf(path: string) {
  const cut = path.lastIndexOf('/')
  return cut < 0 ? '' : path.slice(0, cut)
}

export function nameOf(path: string) {
  return path.slice(path.lastIndexOf('/') + 1)
}

/** Whether `test` is `path` or inside it. */
export function within(path: string, test: string) {
  return path === '' || test === path || test.startsWith(`${path}/`)
}

/** The paths that are not inside another one of them: moving `a` moves `a/b`. */
export function topmost(paths: string[]) {
  const unique = [...new Set(paths)]
  return unique.filter((one) => !unique.some((other) => other !== one && within(other, one)))
}

/** Where `path` is after `from` became `to`, or null when it was not inside. */
function carried(path: string, from: string, to: string) {
  if (path === from) return to
  if (path.startsWith(`${from}/`)) return to + path.slice(from.length)
  return null
}

/** Files cut or copied in the tree, waiting for a paste. */
export const treeClipboard = reactive({
  mode: null as 'cut' | 'copy' | null,
  paths: [] as string[],
  repo: null as string | null
})

export function clip(mode: 'cut' | 'copy', paths: string[]) {
  treeClipboard.mode = mode
  treeClipboard.paths = topmost(paths)
  treeClipboard.repo = useGit().store.repo?.path ?? null
  const what = paths.length === 1 ? paths[0]! : `${paths.length} items`
  useGit().note(`${mode === 'cut' ? 'Cut' : 'Copied'} ${what}`)
}

export function clearClip() {
  treeClipboard.mode = null
  treeClipboard.paths = []
  treeClipboard.repo = null
}

/** Whether there is something to paste in the project on screen. */
export function canPaste() {
  return !!treeClipboard.mode && treeClipboard.repo === (useGit().store.repo?.path ?? null)
}

interface LetGo {
  tabs: { path: string; mode: DiffMode }[]
  active: string | null
}

/**
 * Saves every open file and closes the tabs of those at or under `paths`, so
 * nothing is written to the old name after it has gone. Null when a tab held
 * edits that could not be saved and the answer was to keep them.
 */
async function letGo(paths: string[]): Promise<LetGo | null> {
  await flushAll()
  const { state } = useTabs()
  const hit = state.tabs.filter((tab) => !tab.commit && paths.some((path) => within(path, tab.path)))
  if (!hit.length) return { tabs: [], active: null }
  const active = activeTab()
  const ids = hit.map((tab) => tab.id)
  const ok = await closeTabsSafely(ids, () => closeWhere((tab) => ids.includes(tab.id)))
  if (!ok) return null
  await nextTick()
  return {
    tabs: hit.map((tab) => ({ path: tab.path, mode: tab.mode })),
    active: active && ids.includes(active.id) ? active.path : null
  }
}

/** Opens the tabs `letGo` closed again, at the names they have now. */
function reopen(closed: LetGo, from: string[], to: string[]) {
  const where = (path: string) => {
    for (const [at, one] of from.entries()) {
      const now = carried(path, one, to[at]!)
      if (now !== null) return now
    }
    return null
  }
  for (const tab of closed.tabs) {
    if (tab.path === closed.active) continue
    const now = where(tab.path)
    if (now) openTab({ path: now, mode: tab.mode }, { history: false })
  }
  const active = closed.tabs.find((tab) => tab.path === closed.active)
  if (active) {
    const now = where(active.path)
    if (now) openTab({ path: now, mode: active.mode }, { history: false })
  }
}

/** Folders open at or under `from` stay open under `to`. */
function carryOpen(from: string, to: string) {
  const open = treeSession().open
  for (const dir of Object.keys(open)) {
    const now = carried(dir, from, to)
    if (now === null) continue
    delete open[dir]
    open[now] = true
  }
  for (const dir of ancestors(to)) open[dir] = true
}

function forgetOpen(path: string) {
  const open = treeSession().open
  for (const dir of Object.keys(open)) if (within(path, dir)) delete open[dir]
}

/** Makes an empty file or a folder in `parent`. A file opens in the editor. */
export async function createEntry(parent: string, name: string, dir: boolean) {
  const made = await useGit().createPath(joinPath(parent, name), dir)
  if (made === null) return null
  const open = treeSession().open
  for (const one of ancestors(made)) open[one] = true
  if (dir) open[made] = true
  await reloadOpen()
  if (!dir) openTab({ path: made, mode: 'file' })
  return made
}

/** Renames, or moves, one path. */
export async function renameEntry(from: string, to: string) {
  if (from === to) return to
  const closed = await letGo([from])
  if (!closed) return null
  const now = await useGit().renamePath(from, to)
  if (now === null) {
    reopen(closed, [from], [from])
    return null
  }
  carryOpen(from, now)
  await reloadOpen()
  reopen(closed, [from], [now])
  return now
}

/** Moves paths into a folder. Those already in it stay where they are. */
export async function moveEntries(paths: string[], dest: string) {
  const moving = topmost(paths).filter((path) => parentOf(path) !== dest)
  if (!moving.length) return []
  if (moving.some((path) => within(path, dest))) {
    useGit().note(`A folder cannot be moved into itself`, 'error')
    return null
  }
  const closed = await letGo(moving)
  if (!closed) return null
  const git = useGit()
  const said = await git.movePaths(moving, dest)
  // What did not move is still where it was, and its tabs open there again.
  const from = [...(said?.from ?? [])]
  const to = [...(said?.to ?? [])]
  for (const path of moving) {
    if (from.includes(path)) continue
    from.push(path)
    to.push(path)
  }
  for (const [at, one] of (said?.from ?? []).entries()) carryOpen(one, said!.to[at]!)
  if (dest && said?.to.length) treeSession().open[dest] = true
  await reloadOpen()
  reopen(closed, from, to)
  if (!said) return null
  if (said.error) {
    const some = said.to.length ? ` (${said.to.length} moved before that)` : ''
    git.note(`Move: ${said.error}${some}`, 'error')
    return null
  }
  return said.to
}

/** Copies paths into a folder; a taken name gets ` copy`. */
export async function copyEntries(paths: string[], dest: string) {
  // What the editor has not written yet is part of what is being copied.
  await flushAll()
  const made = await useGit().copyPaths(topmost(paths), dest)
  if (made === null) return null
  if (dest) treeSession().open[dest] = true
  await reloadOpen()
  return made
}

/** A copy next to the original. */
export function duplicateEntry(path: string) {
  return copyEntries([path], parentOf(path))
}

/**
 * Moves paths to the Trash. The backend saves what git can see of them on the
 * undo history first; the notice carries that step's Undo.
 */
export async function deleteEntries(paths: string[]) {
  const going = topmost(paths)
  const closed = await letGo(going)
  if (!closed) return null
  const git = useGit()
  // The step this delete records, if it records one: only a step that was not
  // on top before is this delete's to undo.
  const before = git.store.history.undo[0]?.id ?? null
  const said = await git.trashPaths(going)
  if (said === null) {
    reopen(closed, going, going)
    return null
  }
  for (const path of going) forgetOpen(path)
  if (treeClipboard.paths.some((path) => going.some((gone) => within(gone, path)))) clearClip()
  await reloadOpen()
  const step = git.store.history.undo[0]
  useToasts().offer(
    said,
    step && step.kind === 'delete' && step.id !== before ? [{ label: 'Undo', run: () => git.undoStep(step.id) }] : []
  )
  return said
}

/** Pastes what was cut or copied into a folder. */
export async function pasteInto(dest: string) {
  if (!canPaste()) return null
  const paths = [...treeClipboard.paths]
  if (treeClipboard.mode === 'cut') {
    const moved = await moveEntries(paths, dest)
    if (moved !== null) clearClip()
    return moved
  }
  return copyEntries(paths, dest)
}

// --- files dropped in from outside the window

/** One thing dropped from the Finder or Explorer: a file, or a folder to walk. */
export interface Dropped {
  name: string
  file?: File
  entry?: FileSystemEntry
}

/** The biggest file copied in from a drop, as the backend allows. */
const IMPORT_LIMIT = 100 * 1024 * 1024

/**
 * What a drop holds. Read during the drop event itself: the browser empties
 * the list as soon as the handler returns.
 */
export function droppedItems(data: DataTransfer | null): Dropped[] {
  if (!data) return []
  const out: Dropped[] = []
  for (const item of Array.from(data.items ?? [])) {
    if (item.kind !== 'file') continue
    const entry = typeof item.webkitGetAsEntry === 'function' ? item.webkitGetAsEntry() : null
    if (entry) out.push({ name: entry.name, entry })
    else {
      const file = item.getAsFile()
      if (file) out.push({ name: file.name, file })
    }
  }
  if (!out.length) for (const file of Array.from(data.files ?? [])) out.push({ name: file.name, file })
  return out
}

/** Whether a drag carries files from outside the window. */
export function carriesFiles(data: DataTransfer | null) {
  return !!data && Array.from(data.types ?? []).includes('Files')
}

async function walk(entry: FileSystemEntry, path: string, files: { path: string; file: File }[], dirs: string[]) {
  if (entry.isFile) {
    const file = await new Promise<File>((ok, fail) => (entry as FileSystemFileEntry).file(ok, fail))
    files.push({ path, file })
    return
  }
  if (!entry.isDirectory) return
  dirs.push(path)
  const reader = (entry as FileSystemDirectoryEntry).createReader()
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((ok, fail) => reader.readEntries(ok, fail))
    if (!batch.length) break
    for (const child of batch) {
      // A repository's own database is not a project's files.
      if (child.isDirectory && child.name.toLowerCase() === '.git') continue
      await walk(child, `${path}/${child.name}`, files, dirs)
    }
  }
}

export async function base64Of(file: Blob) {
  const bytes = new Uint8Array(await file.arrayBuffer())
  let text = ''
  for (let at = 0; at < bytes.length; at += 0x8000) {
    text += String.fromCharCode(...bytes.subarray(at, at + 0x8000))
  }
  return btoa(text)
}

/** Copies what was dropped into a folder, each under a name not taken there. */
export async function importDropped(items: Dropped[], dest: string) {
  const git = useGit()
  let copied = 0
  const skipped: string[] = []
  for (const item of items) {
    const top = await git.freeName(dest, item.name)
    if (top === null) continue
    const files: { path: string; file: File }[] = []
    const dirs: string[] = []
    try {
      if (item.entry) await walk(item.entry, top, files, dirs)
      else if (item.file) files.push({ path: top, file: item.file })
    } catch (error) {
      git.note(`Could not read ${item.name}: ${String(error)}`, 'error')
      continue
    }
    // A folder with nothing in it is made on its own; the rest come with
    // their files.
    for (const dir of dirs) {
      if (files.some((one) => one.path.startsWith(`${dir}/`))) continue
      await invoke('create_path', { path: dir, dir: true }).catch(() => undefined)
    }
    for (const one of files) {
      if (one.file.size > IMPORT_LIMIT) {
        skipped.push(one.path)
        continue
      }
      if ((await git.importFile(one.path, await base64Of(one.file))) !== null) copied += 1
    }
  }
  if (dest) treeSession().open[dest] = true
  await git.refresh()
  await reloadOpen()
  const where = dest ? `${dest}/` : 'the project'
  if (copied) useToasts().info(`Copied ${copied} ${copied === 1 ? 'file' : 'files'} into ${where}`)
  if (skipped.length) git.note(`Over 100 MB, not copied in: ${skipped.join(', ')}`, 'error')
  return copied
}
