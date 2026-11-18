import { reactive } from 'vue'
import { invoke } from './useInvoke'
import { useGit } from './useGit'
import { setFileRevealer } from './useTabs'

/**
 * What the sidebar shows for each project: the branch list, or the files.
 *
 * Kept per project for the session, with the filter, the folders that are
 * open and how far the list is scrolled, so going to another project and back
 * finds the sidebar the way it was left.
 */

export type SideMode = 'branches' | 'files'

export interface TreeSession {
  mode: SideMode
  /** The filter box over the files. */
  filter: string
  /** Folders that are open, by path relative to the work tree. */
  open: Record<string, true>
  scroll: number
  /** A file to bring into view: the one just opened, or one asked for. */
  reveal: { path: string; seq: number } | null
}

// Plain maps of reactive values: creating a project's entry on first look is
// not a change anything on screen has to hear about.
const sessions = new Map<string, TreeSession>()
/** For the window with no repository open, which has no files to show. */
const nowhere: TreeSession = reactive({ mode: 'branches', filter: '', open: {}, scroll: 0, reveal: null })
let revealSeq = 0

/** The sidebar state of the project on screen. */
export function treeSession(): TreeSession {
  const path = useGit().store.repo?.path
  if (!path) return nowhere
  let found = sessions.get(path)
  if (!found) {
    found = reactive({ mode: 'branches', filter: '', open: {}, scroll: 0, reveal: null }) as TreeSession
    sessions.set(path, found)
  }
  return found
}

/** Every folder a path sits in, outermost first: `a/b/c.txt` is `a`, `a/b`. */
export function ancestors(path: string): string[] {
  const parts = path.split('/')
  const out: string[] = []
  for (let at = 1; at < parts.length; at++) out.push(parts.slice(0, at).join('/'))
  return out
}

/**
 * The folder selected in the Files tree while the tree has the focus, else
 * null: ⌘⇧F then searches inside it.
 */
export const treeFolder = reactive({ dir: null as string | null })

/** Switches the sidebar to the files and brings one of them into view. */
export function revealInFiles(path: string) {
  const session = treeSession()
  session.mode = 'files'
  for (const dir of ancestors(path)) session.open[dir] = true
  revealSeq += 1
  session.reveal = { path, seq: revealSeq }
}

// A file that was looked for, opened, brings the tree forward with it.
setFileRevealer(revealInFiles)

/** One entry in a folder, as `list_dir` gives it. */
export interface DirEntry {
  name: string
  path: string
  dir: boolean
  ignored: boolean
  link: boolean
}

/** A file the finder matched, with which characters of its path did. */
export interface FoundFile {
  path: string
  score: number
  indices: number[]
}

/**
 * Each project's folders as last read, by folder path (`''` is the root).
 *
 * Only folders somebody opened are ever read, so a repository of fifty
 * thousand files costs one folder's worth of reading to show.
 */
const listings = new Map<string, Record<string, DirEntry[]>>()

function listingsOf(repo: string): Record<string, DirEntry[]> {
  let found = listings.get(repo)
  if (!found) {
    found = reactive({}) as Record<string, DirEntry[]>
    listings.set(repo, found)
  }
  return found
}

/** The folders read so far in the project on screen. */
export function listingsHere(): Record<string, DirEntry[]> {
  const path = useGit().store.repo?.path
  return path ? listingsOf(path) : reactive({})
}

/**
 * Projects whose root folder could not be read: the tree has nothing to show
 * for them and says so, rather than waiting on a list that is not coming.
 */
export const rootFailed = reactive<Record<string, true>>({})

/** Reads, or reads again, what is in one folder. */
export async function loadDir(dir: string) {
  const repo = useGit().store.repo?.path
  if (!repo) return
  try {
    const entries = await invoke<DirEntry[]>('list_dir', { path: dir })
    // A slower answer for a project that is no longer on screen is still that
    // project's: it goes where it belongs.
    listingsOf(repo)[dir] = entries
    if (dir === '') delete rootFailed[repo]
  } catch {
    if (dir === '') {
      // The work tree itself: moved, removed, or not readable. An empty list
      // and a note, with a way to ask again.
      listingsOf(repo)[''] = []
      rootFailed[repo] = true
      return
    }
    // Gone since it was opened, or never a folder: shut it rather than show
    // an error where a list was.
    const into = listings.get(repo)
    if (into) delete into[dir]
    const session = sessions.get(repo)
    if (session) delete session.open[dir]
  }
}

/** Reads every open folder again: something on disk moved. */
export async function reloadOpen() {
  const session = treeSession()
  const here = listingsHere()
  const dirs = ['', ...Object.keys(session.open)].filter((dir) => dir === '' || here[dir])
  await Promise.all(dirs.map((dir) => loadDir(dir)))
}

let findSeq = 0

/**
 * Files whose path matches, best first, or null when a newer search has
 * overtaken this one and its answer is not wanted.
 */
export async function findFiles(query: string, limit = 200): Promise<FoundFile[] | null> {
  // From the clock, so the numbers keep going up across a reload of the page:
  // the backend drops any request older than the newest it has seen.
  findSeq = Math.max(findSeq + 1, Date.now())
  const mine = findSeq
  try {
    const found = await invoke<FoundFile[]>('find_files', { query, limit, request: mine })
    return mine === findSeq ? found : null
  } catch {
    return mine === findSeq ? [] : null
  }
}

/** Forgets every project's sidebar. For tests. */
export function resetFileTree() {
  listings.clear()
  sessions.clear()
  for (const key of Object.keys(rootFailed)) delete rootFailed[key]
  nowhere.mode = 'branches'
  nowhere.filter = ''
  nowhere.open = {}
  nowhere.scroll = 0
  nowhere.reveal = null
}
