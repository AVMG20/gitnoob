import { reactive, watch } from 'vue'

const KEY = 'gitnoob.fileview'

const state = reactive({
  /** `tree` groups by directory; `path` lists full paths flat. */
  mode: 'tree' as 'tree' | 'path',
  /** Directories the user has folded away, keyed by full directory path. */
  collapsed: new Set<string>()
})

try {
  const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}')
  if (saved.mode === 'path' || saved.mode === 'tree') state.mode = saved.mode
  if (Array.isArray(saved.collapsed)) state.collapsed = new Set(saved.collapsed)
} catch {
  // No stored preference is not a problem.
}

watch(
  () => [state.mode, state.collapsed.size],
  () => {
    try {
      localStorage.setItem(
        KEY,
        JSON.stringify({ mode: state.mode, collapsed: [...state.collapsed] })
      )
    } catch {
      // Storage can be refused; the choice still holds for this session.
    }
  }
)

export interface FileEntry {
  path: string
  /** Status letter class: added, modified, deleted, renamed, untracked… */
  kind: string
  additions?: number
  deletions?: number
}

/** The four things that can have happened to a file, for counting purposes. */
export type Change = 'added' | 'modified' | 'deleted' | 'renamed'

export type Tally = Record<Change, number>

/**
 * Which of the four a git status letter belongs to.
 *
 * A file git has never seen is an addition waiting to happen, and a file whose
 * mode changed has been edited; neither deserves a count of its own in a
 * folder's summary.
 */
export function change(kind: string): Change {
  switch (kind) {
    case 'added':
    case 'untracked':
    case 'copied':
      return 'added'
    case 'deleted':
      return 'deleted'
    case 'renamed':
      return 'renamed'
    default:
      return 'modified'
  }
}

export interface Row {
  key: string
  depth: number
  kind: 'dir' | 'file'
  /** Just the segment name, for display. */
  name: string
  /** Full path, for selection and actions. */
  path: string
  entry?: FileEntry
  /** Files inside, for a directory row. */
  count?: number
  /** What happened inside, for a directory row: how many of each kind. */
  tally?: Tally
  collapsed?: boolean
}

/**
 * Turns a flat file list into the rows to render.
 *
 * Directories with a single child are joined into one row — `app/components` in
 * place of `app` then `components` — which is what keeps a deep tree readable.
 */
export function buildRows(files: FileEntry[], mode: 'tree' | 'path', collapsed: Set<string>): Row[] {
  if (mode === 'path') {
    return files.map((entry) => ({
      key: entry.path,
      depth: 0,
      kind: 'file' as const,
      name: entry.path,
      path: entry.path,
      entry
    }))
  }

  interface Node {
    name: string
    path: string
    dirs: Map<string, Node>
    files: FileEntry[]
  }

  const root: Node = { name: '', path: '', dirs: new Map(), files: [] }

  for (const entry of files) {
    const parts = entry.path.split('/')
    const fileName = parts.pop() ?? entry.path
    let node = root
    let walked = ''
    for (const part of parts) {
      walked = walked ? `${walked}/${part}` : part
      let next = node.dirs.get(part)
      if (!next) {
        next = { name: part, path: walked, dirs: new Map(), files: [] }
        node.dirs.set(part, next)
      }
      node = next
    }
    node.files.push({ ...entry, path: entry.path })
    // Keep the leaf name for display without re-splitting later.
    node.files[node.files.length - 1] = { ...entry }
    void fileName
  }

  const total = (node: Node): number =>
    node.files.length + [...node.dirs.values()].reduce((sum, dir) => sum + total(dir), 0)

  /** What happened anywhere below a folder, so a folded one still says so. */
  const tally = (node: Node): Tally => {
    const counts: Tally = { added: 0, modified: 0, deleted: 0, renamed: 0 }
    for (const entry of node.files) counts[change(entry.kind)]++
    for (const dir of node.dirs.values()) {
      const inside = tally(dir)
      for (const key of Object.keys(counts) as Change[]) counts[key] += inside[key]
    }
    return counts
  }

  const rows: Row[] = []

  const walk = (node: Node, depth: number) => {
    for (const dir of [...node.dirs.values()].sort((a, b) => a.name.localeCompare(b.name))) {
      // Collapse a chain of single-child directories into one row.
      let joined = dir
      let label = dir.name
      while (joined.dirs.size === 1 && joined.files.length === 0) {
        const only = [...joined.dirs.values()][0]
        label = `${label}/${only.name}`
        joined = only
      }

      const isCollapsed = collapsed.has(joined.path)
      rows.push({
        key: `d:${joined.path}`,
        depth,
        kind: 'dir',
        name: label,
        path: joined.path,
        count: total(joined),
        tally: tally(joined),
        collapsed: isCollapsed
      })
      if (!isCollapsed) walk(joined, depth + 1)
    }

    for (const entry of node.files.sort((a, b) => a.path.localeCompare(b.path))) {
      rows.push({
        key: `f:${entry.path}`,
        depth,
        kind: 'file',
        name: entry.path.split('/').pop() ?? entry.path,
        path: entry.path,
        entry
      })
    }
  }

  walk(root, 0)
  return rows
}

export function useFileView() {
  return {
    state,
    toggleDir(path: string) {
      if (state.collapsed.has(path)) state.collapsed.delete(path)
      else state.collapsed.add(path)
      // Reassign so the reactive watcher and computeds see the change.
      state.collapsed = new Set(state.collapsed)
    },
    expandAll() {
      state.collapsed = new Set()
    }
  }
}
