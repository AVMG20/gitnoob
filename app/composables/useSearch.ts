import { markRaw, reactive } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from './useInvoke'
import { useGit } from './useGit'
import { useTabs } from './useTabs'
import { editorSelection } from './useCodeMirror'

/**
 * The search window: inside files (⌘⇧F), file names (⌘⇧N) and symbols
 * (⌘⌥O), one window with three modes.
 *
 * Each mode keeps its own last search and options for the session. Pressing
 * another mode's key, or Tab, while the window is open switches mode and
 * takes what was typed along.
 */

export type SearchMode = 'text' | 'files' | 'symbols'

/** The order Tab walks the modes in. */
export const SEARCH_MODES: SearchMode[] = ['text', 'files', 'symbols']

export interface LineHit {
  line: number
  text: string
  /** Where the matches are in `text`, in UTF-16 units. */
  ranges: [number, number][]
}

export interface FileHits {
  request: number
  path: string
  hits: LineHit[]
}

export interface SearchDone {
  request: number
  matches: number
  files: number
  more: number
  /** Stopped at the cap with files left unread; `more` is then a floor. */
  capped?: boolean
  searched: number
  cancelled: boolean
  error: string | null
}

export const search = reactive({
  open: false,
  mode: 'text' as SearchMode,
  text: { query: '', case: false, word: false, regex: false, changedOnly: false, glob: '' },
  /** `deps`: the file names in vendor/ and node_modules/ too, after the project's. */
  files: { query: '', deps: false },
  symbols: { query: '' },
  /**
   * What the text search found, as it arrives: a frame's worth at a time,
   * each file raw, so two thousand hits are not two thousand proxies.
   */
  found: [] as FileHits[],
  done: null as SearchDone | null,
  running: false,
  /** The request whose answers are wanted; anything else is stale. */
  request: 0
})

let lastRequest = 0

/** The next request number: from the clock, so a reload never goes backwards. */
function nextRequest() {
  lastRequest = Math.max(lastRequest + 1, Date.now())
  return lastRequest
}

/** Starts a search inside files with the box as it reads now. */
export async function runTextSearch() {
  const query = search.text.query
  const request = nextRequest()
  search.request = request
  search.found = []
  search.done = null
  pending = []
  if (!query) {
    search.running = false
    cancelRunning(request)
    return
  }
  search.running = true
  await ensureListening()
  try {
    await invoke('search_text', {
      query,
      options: optionsNow(),
      request
    })
  } catch (error) {
    if (search.request === request) {
      search.running = false
      search.done = {
        request,
        matches: 0,
        files: 0,
        more: 0,
        searched: 0,
        cancelled: false,
        error: String(error)
      }
    }
  }
}

function optionsNow() {
  return {
    case: search.text.case,
    word: search.text.word,
    regex: search.text.regex,
    changed_only: search.text.changedOnly,
    glob: search.text.glob
  }
}

/**
 * Searches one file again, after it was edited in the preview, and puts its
 * new hits in place of the old: the same query and options, the same place in
 * the list, without running the whole search again. A file left with nothing
 * in it drops out. Nothing changes if the search moved on meanwhile.
 */
export async function refreshFile(path: string) {
  const query = search.text.query
  const request = search.request
  if (!query) return
  const hits = await invoke<LineHit[]>('search_file', { path, query, options: optionsNow() }).catch(() => null)
  if (!hits || search.request !== request || search.text.query !== query) return
  const at = search.found.findIndex((one) => one.path === path)
  if (at < 0) return
  if (hits.length) search.found.splice(at, 1, markRaw({ request, path, hits }))
  else search.found.splice(at, 1)
}

/** Hits that arrived since the last frame, not handed to the window yet. */
let pending: FileHits[] = []
let frame: number | null = null

/**
 * Hands one event's worth of results to the window, if it is still wanted.
 * Kept until the next frame: a search sends an event per file, hundreds of
 * them in a burst, and one update a frame is all anyone can see.
 */
export function takeHits(payload: FileHits) {
  if (payload.request !== search.request) return
  pending.push(markRaw(payload))
  if (frame === null) frame = nextFrame(flushHits)
}

/** Puts the hits waiting for a frame into the list now. */
export function flushHits() {
  if (frame !== null) cancelFrame(frame)
  frame = null
  const batch = pending.filter((one) => one.request === search.request)
  pending = []
  if (batch.length) search.found.push(...batch)
}

const nextFrame = (run: () => void): number =>
  typeof requestAnimationFrame === 'function' ? requestAnimationFrame(run) : (setTimeout(run, 16) as unknown as number)

const cancelFrame = (handle: number) =>
  typeof cancelAnimationFrame === 'function' ? cancelAnimationFrame(handle) : clearTimeout(handle)

export function takeDone(payload: SearchDone) {
  if (payload.request !== search.request) return
  // Every hit was sent before the end: they all go in with it.
  flushHits()
  search.done = payload
  search.running = false
}

let listening: Promise<UnlistenFn[]> | null = null

function ensureListening() {
  if (!listening) {
    listening = Promise.all([
      listen<FileHits>('search:hit', ({ payload }) => takeHits(payload)),
      listen<SearchDone>('search:done', ({ payload }) => takeDone(payload))
    ]).catch(() => [])
  }
  return listening
}

/**
 * Opens the window, or switches it to the other mode. In-files mode starts
 * from text selected in the editor. `recent` opens the file finder empty, on
 * the list of recent files.
 */
export function openSearch(mode: SearchMode, recent = false) {
  const wasOpen = search.open
  const typed = wasOpen ? queryOf(search.mode) : ''
  // Leaving the search inside files while it is still reading: nobody is
  // looking at what it finds, so it stops rather than read the whole tree.
  if (wasOpen && search.mode === 'text' && mode !== 'text' && search.running) {
    const request = nextRequest()
    search.request = request
    search.running = false
    cancelRunning(request)
  }
  search.mode = mode
  if (wasOpen && typed) setQueryOf(mode, typed)
  if (!wasOpen && mode === 'text') {
    const picked = editorSelection() || windowSelection()
    if (picked) search.text.query = picked
  }
  if (recent) search.files.query = ''
  search.open = true
}

/** What is typed in a mode's box. */
export function queryOf(mode: SearchMode) {
  return mode === 'text' ? search.text.query : mode === 'files' ? search.files.query : search.symbols.query
}

export function setQueryOf(mode: SearchMode, value: string) {
  if (mode === 'text') search.text.query = value
  else if (mode === 'files') search.files.query = value
  else search.symbols.query = value
}

function windowSelection() {
  const picked = typeof window === 'undefined' ? '' : (window.getSelection?.()?.toString() ?? '')
  return picked && !picked.includes('\n') && picked.length <= 200 ? picked : ''
}

export function closeSearch() {
  search.open = false
  if (search.running) {
    const request = nextRequest()
    search.request = request
    search.running = false
    cancelRunning(request)
  }
}

/**
 * Tells the backend a newer search exists, with nothing in it: the one still
 * reading files stops at the next one, rather than reading the whole tree for
 * a window that is shut or a box that was cleared.
 */
function cancelRunning(request: number) {
  void invoke('search_text', {
    query: '',
    options: { case: false, word: false, regex: false, changed_only: false, glob: '' },
    request
  }).catch(() => undefined)
}

/** `app/Tax.php:42` as the path and the line. */
export function splitLine(query: string): { query: string; line: number | null } {
  const found = /^(.*?):(\d+)$/.exec(query.trim())
  if (!found) return { query: query.trim(), line: null }
  return { query: found[1]!, line: Number(found[2]) }
}

export interface FoundName {
  path: string
  indices: number[]
  group: 'recent' | 'changed' | 'files' | 'deps'
  /** A folder, which is shown in the Files tree rather than opened. */
  dir: boolean
}

/**
 * Files and folders whose name matches, ranked: recently opened files first,
 * then the ones the working tree panel lists as changed, then everything
 * else, folders among the files, by how well it matched. An empty query
 * lists the recent files and then the changed ones.
 */
/** A path inside what Composer or npm installed. */
const DEPENDENCY = /^(vendor|node_modules)\//

export async function findNames(query: string, limit = 60): Promise<FoundName[] | null> {
  const git = useGit()
  const recent = useTabs().state.recent
  const changed = [
    ...new Set(
      [...(git.store.status?.unstaged ?? []), ...(git.store.status?.staged ?? [])]
        .filter((entry) => entry.kind !== 'deleted')
        .map((entry) => entry.path)
    )
  ]
  if (!query.trim()) {
    const seen = new Set(recent)
    return [
      ...recent.map((path) => ({ path, indices: [], group: 'recent' as const, dir: false })),
      ...changed
        .filter((path) => !seen.has(path))
        .map((path) => ({ path, indices: [], group: 'changed' as const, dir: false }))
    ].slice(0, limit)
  }
  const request = nextRequest()
  type Found = { path: string; score: number; indices: number[]; dir?: boolean }
  let found: Found[]
  try {
    found = await invoke<Found[]>('find_files', {
      query,
      limit,
      request,
      also: [...recent, ...changed],
      dependencies: search.files.deps,
      folders: true
    })
  } catch {
    return null
  }
  if (request !== lastRequest) return null
  // Recent and changed are files: a folder goes with the rest, by score.
  const rank = (one: Found) =>
    !one.dir && recent.includes(one.path)
      ? 0
      : !one.dir && changed.includes(one.path)
        ? 1
        : DEPENDENCY.test(one.path)
          ? 3
          : 2
  return found
    .map((one, order) => ({ one, order, rank: rank(one) }))
    .sort((a, b) => a.rank - b.rank || a.order - b.order)
    .map(({ one, rank }) => ({
      path: one.path,
      indices: one.indices,
      group: (['recent', 'changed', 'files', 'deps'] as const)[rank]!,
      dir: !!one.dir
    }))
}

export interface FoundSymbol {
  name: string
  kind: string
  path: string
  line: number
  column: number
  indices: number[]
}

let symbolAsk = 0

/** Class, function and method names that match, best first; null when stale. */
export async function findSymbols(query: string, limit = 60): Promise<FoundSymbol[] | null> {
  const mine = ++symbolAsk
  if (!query.trim()) return []
  const found = await invoke<FoundSymbol[]>('find_symbols', { query, limit }).catch(() => null)
  return mine === symbolAsk ? (found ?? []) : null
}

/** Forgets the window's state. For tests. */
export function resetSearch() {
  search.open = false
  search.mode = 'text'
  search.text = { query: '', case: false, word: false, regex: false, changedOnly: false, glob: '' }
  search.files = { query: '', deps: false }
  search.symbols = { query: '' }
  search.found = []
  search.done = null
  search.running = false
  search.request = 0
  pending = []
  if (frame !== null) cancelFrame(frame)
  frame = null
  listening = null
}
