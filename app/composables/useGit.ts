import { invoke } from '@tauri-apps/api/core'
import { reactive, ref } from 'vue'

export interface RepoInfo {
  path: string
  name: string
  head: string
  detached: boolean
  state: string
}

export interface LocalBranch {
  name: string
  oid: string
  is_head: boolean
  upstream: string | null
  ahead: number
  behind: number
}

export interface RemoteBranch { name: string; remote: string; oid: string }
export interface Tag { name: string; oid: string }
export interface Stash { index: number; message: string }

export interface RefTree {
  locals: LocalBranch[]
  remotes: RemoteBranch[]
  tags: Tag[]
  stashes: Stash[]
}

export interface StatusEntry { path: string; kind: string }
export interface WorkingStatus {
  staged: StatusEntry[]
  unstaged: StatusEntry[]
  conflicted: string[]
}

export interface Segment { x1: number; y1: number; x2: number; y2: number; color: number }
export interface RefLabel { kind: string; name: string }

export interface GraphRow {
  oid: string
  short: string
  summary: string
  author: string
  email: string
  time: number
  parents: string[]
  lane: number
  color: number
  width: number
  segments: Segment[]
  labels: RefLabel[]
}

export interface FileChange {
  path: string
  old_path: string | null
  status: string
  additions: number
  deletions: number
  binary: boolean
}

export interface CommitDetail {
  oid: string
  short: string
  summary: string
  body: string
  author: string
  email: string
  time: number
  committer: string
  commit_time: number
  parents: string[]
  files: FileChange[]
}

export interface DiffLine {
  origin: string
  old_lineno: number | null
  new_lineno: number | null
  content: string
}
export interface DiffHunk { header: string; lines: DiffLine[] }
export interface FileDiff { path: string; binary: boolean; hunks: DiffHunk[] }

export interface CommitSummary {
  oid: string
  short: string
  summary: string
  author: string
  time: number
}

export interface PushPreview {
  branch: string
  remote: string
  upstream: string | null
  new_upstream: boolean
  ahead: number
  behind: number
  force_needed: boolean
  will_orphan: CommitSummary[]
  will_push: CommitSummary[]
}

export interface MergeOutcome { ok: boolean; message: string; conflicts: string[] }
export interface AmendDraft { summary: string; body: string; is_pushed: boolean; short: string }

export interface StashEntry {
  index: number
  oid: string
  message: string
  branch: string | null
  time: number
  files: number
}

export interface HistoryEntry {
  id: number
  label: string
  kind: string
  branch: string | null
  before: string | null
  after: string | null
  mode: 'soft' | 'hard' | 'checkout' | 'stash'
  destructive: boolean
  at: number
}

export type ResetMode = 'soft' | 'mixed' | 'hard'

export interface ResetPreview {
  target: string
  short: string
  summary: string
  branch: string | null
  dropped: CommitSummary[]
  diverges: boolean
  staged_files: number
  unstaged_files: number
}

export interface Stacks {
  undo: HistoryEntry[]
  redo: HistoryEntry[]
}

export interface CmdOutput {
  argv: string[]
  ok: boolean
  code: number
  stdout: string
  stderr: string
}

export type ConflictBlock =
  | { kind: 'context'; lines: string[] }
  | {
      kind: 'conflict'
      index: number
      ours: string[]
      base: string[]
      theirs: string[]
      has_base: boolean
      ours_label: string
      theirs_label: string
    }

export interface ConflictFile {
  path: string
  blocks: ConflictBlock[]
  conflict_count: number
}

export interface Resolution {
  take_ours: boolean
  take_theirs: boolean
  ours_first: boolean
  custom?: string[] | null
}

/** One line in the activity log at the bottom of the window. */
export interface LogLine {
  id: number
  at: number
  level: 'info' | 'error'
  text: string
}

const COMMIT_PAGE = 500

/** Stands in for the working tree in `store.selected`. */
export const WIP = '__working__' 

// A single shared store: the app has one open repository at a time, so there is
// nothing to gain from per-component state.
const store = reactive({
  repo: null as RepoInfo | null,
  refs: null as RefTree | null,
  status: null as WorkingStatus | null,
  rows: [] as GraphRow[],
  hasMore: false,
  limit: COMMIT_PAGE,
  /** Either `WIP` or a commit id. The working tree is selected by default. */
  selected: WIP as string,
  detail: null as CommitDetail | null,
  stashes: [] as StashEntry[],
  history: { undo: [], redo: [] } as Stacks,
  /** Set while the resolver is open on a conflicted file. */
  resolving: null as string | null,
  /** Free-text filter over the loaded commits. */
  query: '',
  /** When set, a file is open full width in place of the graph. */
  viewer: null as { path: string; side?: 'staged' | 'unstaged'; commit?: string } | null,
  /** Number of calls in flight; a counter rather than a flag so overlapping
      operations cannot switch it off early. */
  pending: 0,
  /** What the newest in-flight call is doing, for the progress bar. */
  busyLabel: null as string | null,
  log: [] as LogLine[]
})

const logSeq = ref(0)

// Everything already written reads `store.busy`; keep it as a live getter over
// the counter rather than a second source of truth.
Object.defineProperty(store, 'busy', {
  get: () => store.pending > 0,
  enumerable: true
})

function note(text: string, level: 'info' | 'error' = 'info') {
  if (!text.trim()) return
  store.log.unshift({ id: ++logSeq.value, at: Date.now(), level, text: text.trim() })
  if (store.log.length > 200) store.log.length = 200
}

/** Runs a backend call, surfacing failures in the log instead of throwing. */
async function guard<T>(label: string, fn: () => Promise<T>): Promise<T | null> {
  store.pending += 1
  store.busyLabel = label
  try {
    return await fn()
  } catch (error) {
    note(`${label}: ${String(error)}`, 'error')
    return null
  } finally {
    store.pending -= 1
    if (store.pending <= 0) {
      store.pending = 0
      store.busyLabel = null
    }
  }
}

export function useGit() {
  async function openRepo(path: string) {
    const info = await guard('Open repository', () =>
      invoke<RepoInfo>('open_repo', { path })
    )
    if (!info) return false
    store.repo = info
    store.limit = COMMIT_PAGE
    store.selected = WIP
    store.detail = null
    note(`Opened ${info.path}`)
    await refresh()
    return true
  }

  /** Reloads refs, status and the graph. Called after anything that mutates. */
  async function refresh() {
    if (!store.repo) return
    const [info, refs, status, page, stashes, history] = await Promise.all([
      invoke<RepoInfo>('repo_info').catch(() => store.repo),
      invoke<RefTree>('ref_tree').catch(() => null),
      invoke<WorkingStatus>('working_status').catch(() => null),
      invoke<{ rows: GraphRow[]; has_more: boolean }>('commit_graph', {
        limit: store.limit
      }).catch(() => null),
      invoke<StashEntry[]>('stash_list').catch(() => []),
      invoke<Stacks>('history').catch(() => ({ undo: [], redo: [] }))
    ])
    if (info) store.repo = info
    if (refs) store.refs = refs
    if (status) store.status = status
    if (page) {
      store.rows = page.rows
      store.hasMore = page.has_more
    }
    store.stashes = stashes ?? []
    store.history = history ?? { undo: [], redo: [] }

    // An amend rewrites the commit that was open; fall back to the working tree.
    if (store.selected !== WIP && !store.rows.some((r) => r.oid === store.selected)) {
      store.selected = WIP
      store.detail = null
    }
    // Keep the detail panel current for whatever is selected.
    if (store.selected !== WIP) {
      store.detail = await invoke<CommitDetail>('commit_detail', { oid: store.selected }).catch(
        () => store.detail
      )
    }
  }

  async function loadMore() {
    store.limit += COMMIT_PAGE
    await refresh()
  }

  async function select(oid: string) {
    store.selected = oid
    if (oid === WIP) {
      store.detail = null
      return
    }
    store.detail = await guard('Load commit', () =>
      invoke<CommitDetail>('commit_detail', { oid })
    )
  }

  /** Opens a stash's diff in the detail panel, reusing the commit view. */
  async function selectStash(index: number) {
    const oid = await guard('Read stash', () => invoke<string>('stash_oid', { index }))
    if (oid) await select(oid)
  }

  const commitFileDiff = (oid: string, path: string) =>
    guard('Load diff', () => invoke<FileDiff>('commit_file_diff', { oid, path }))

  const workingFileDiff = (path: string, side: 'staged' | 'unstaged') =>
    guard('Load diff', () => invoke<FileDiff>('working_file_diff', { path, side }))

  async function run<T>(label: string, command: string, args: Record<string, unknown> = {}) {
    const result = await guard(label, () => invoke<T>(command, args))
    if (result !== null) await refresh()
    return result
  }

  /** Reports a `git` CLI run in the log, whichever way it went. */
  function report(label: string, out: CmdOutput | null) {
    if (!out) return false
    const text = [out.stdout, out.stderr].filter((s) => s.trim()).join('\n')
    note(`${label}: ${text || (out.ok ? 'done' : `exit ${out.code}`)}`, out.ok ? 'info' : 'error')
    return out.ok
  }

  return {
    store,
    note,
    refresh,
    loadMore,
    openRepo,
    select,
    selectStash,
    commitFileDiff,
    workingFileDiff,
    run,
    report,

    checkout: (name: string) => run<string>('Checkout', 'checkout', { name }),
    createBranch: (name: string, start?: string) =>
      run<string>('Create branch', 'create_branch', { name, start, checkout: true }),
    deleteBranch: (name: string, force = false) =>
      run<string>('Delete branch', 'delete_branch', { name, force }),
    renameBranch: (from: string, to: string) =>
      run<string>('Rename branch', 'rename_branch', { from, to }),
    setUpstream: (branch: string, upstream: string) =>
      run<string>('Set upstream', 'set_upstream', { branch, upstream }),
    unsetUpstream: (branch: string) => run<string>('Unset upstream', 'unset_upstream', { branch }),
    addToGitignore: (pattern: string) =>
      run<string>('Ignore', 'add_to_gitignore', { pattern }),
    commitPatch: (oid: string) => guard('Read patch', () => invoke<string>('commit_patch', { oid })),
    applyHunk: (path: string, hunkIndex: number, action: 'stage' | 'unstage' | 'discard') =>
      run<string>(
        action === 'stage' ? 'Stage hunk' : action === 'unstage' ? 'Unstage hunk' : 'Discard hunk',
        'apply_hunk',
        { path, hunkIndex, action }
      ),
    reveal: (path: string) => guard('Reveal', () => invoke('reveal', { path })),

    stage: (paths: string[]) => run<string>('Stage', 'stage', { paths }),
    stageAll: () => run<string>('Stage all', 'stage_all'),
    unstage: (paths: string[]) => run<string>('Unstage', 'unstage', { paths }),
    discard: (paths: string[]) => run<string>('Discard', 'discard', { paths }),
    commit: (message: string, amend = false) =>
      run<string>('Commit', 'commit', { message, amend }),
    amendDraft: () => guard('Read HEAD', () => invoke<AmendDraft>('amend_draft')),
    stashPush: (message?: string) => run<string>('Stash', 'stash_push', { message }),
    stashPop: (index: number) => run<string>('Stash pop', 'stash_pop', { index }),
    stashApply: (index: number) => run<string>('Stash apply', 'stash_apply', { index }),
    stashDrop: (index: number) => run<string>('Stash drop', 'stash_drop', { index }),
    stashBranch: (index: number, name: string) =>
      run<string>('Branch from stash', 'stash_branch', { index, name }),

    resetPreview: (oid: string) =>
      guard('Read reset', () => invoke<ResetPreview>('reset_preview', { oid })),
    reset: (oid: string, mode: ResetMode) => run<string>('Reset', 'reset', { oid, mode }),
    cherryPick: (oid: string) => run<string>('Cherry-pick', 'cherry_pick', { oid }),
    revert: (oid: string) => run<string>('Revert', 'revert', { oid }),
    createTag: (name: string, oid: string, message?: string) =>
      run<string>('Tag', 'create_tag', { name, oid, message }),
    deleteTag: (name: string) => run<string>('Delete tag', 'delete_tag', { name }),
    commitMessageText: (oid: string) =>
      guard('Read message', () => invoke<string>('commit_message_text', { oid })),

    undo: async () => {
      const message = await guard('Undo', () => invoke<string>('undo'))
      if (message) note(message)
      await refresh()
      return message
    },
    redo: async () => {
      const message = await guard('Redo', () => invoke<string>('redo'))
      if (message) note(message)
      await refresh()
      return message
    },

    fetch: async (remote?: string) => {
      const out = await guard('Fetch', () => invoke<CmdOutput>('fetch', { remote }))
      const ok = report('Fetch', out)
      await refresh()
      return ok
    },
    pull: async (rebase = false) => {
      const out = await guard('Pull', () => invoke<CmdOutput>('pull', { rebase }))
      const ok = report('Pull', out)
      await refresh()
      return ok
    },
    pushPreview: (branch?: string, fetchFirst = false) =>
      guard('Push preview', () =>
        invoke<PushPreview>('push_preview', { branch, fetchFirst })
      ),
    push: async (remoteName: string, branch: string, force: boolean, setUpstream: boolean) => {
      const out = await guard('Push', () =>
        invoke<CmdOutput>('push', { remoteName, branch, force, setUpstream })
      )
      const ok = report(force ? 'Force push' : 'Push', out)
      await refresh()
      return ok
    },
    /** Pushes one branch by name, rather than whatever is checked out. */
    pushBranch: async (branch: string, setUpstream: boolean) => {
      const remotes = await invoke<string[]>('remotes').catch(() => [])
      const target = remotes[0] ?? 'origin'
      const out = await guard('Push', () =>
        invoke<CmdOutput>('push', {
          remoteName: target,
          branch,
          force: false,
          setUpstream
        })
      )
      const ok = report('Push', out)
      await refresh()
      return ok
    },
    deleteRemoteBranch: async (remoteName: string, branch: string) => {
      const out = await guard('Delete on remote', () =>
        invoke<CmdOutput>('delete_remote_branch', { remoteName, branch })
      )
      const ok = report('Delete on remote', out)
      await refresh()
      return ok
    },
    pushTag: async (remoteName: string, tag: string) => {
      const out = await guard('Push tag', () => invoke<CmdOutput>('push_tag', { remoteName, tag }))
      const ok = report('Push tag', out)
      await refresh()
      return ok
    },

    rebase: async (onto: string) => {
      const outcome = await guard('Rebase', () => invoke<MergeOutcome>('rebase', { onto }))
      if (outcome) note(`Rebase onto ${onto}: ${outcome.message}`, outcome.ok ? 'info' : 'error')
      await refresh()
      return outcome
    },
    abortRebase: () => run<string>('Abort rebase', 'abort_rebase'),
    continueRebase: async () => {
      const outcome = await guard('Continue rebase', () =>
        invoke<MergeOutcome>('continue_rebase')
      )
      if (outcome) note(`Rebase: ${outcome.message}`, outcome.ok ? 'info' : 'error')
      await refresh()
      return outcome
    },

    merge: async (branch: string, noFf = false) => {
      const outcome = await guard('Merge', () =>
        invoke<MergeOutcome>('merge', { branch, noFf })
      )
      if (outcome) note(`Merge ${branch}: ${outcome.message}`, outcome.ok ? 'info' : 'error')
      await refresh()
      return outcome
    },
    abortMerge: () => run<string>('Abort merge', 'abort_merge'),

    conflictRead: (path: string) =>
      guard('Read conflict', () => invoke<ConflictFile>('conflict_read', { path })),
    conflictPreview: (path: string, choices: Resolution[]) =>
      guard('Preview resolution', () =>
        invoke<string>('conflict_preview', { path, choices })
      ),
    conflictResolve: (path: string, choices: Resolution[]) =>
      run<string>('Resolve', 'conflict_resolve', { path, choices }),
    conflictResolveWhole: (path: string, side: 'ours' | 'theirs') =>
      run<string>('Resolve', 'conflict_resolve_whole', { path, side })
  }
}

export const LANE_COLORS = [
  '#4f9cf9',
  '#f0a83c',
  '#57c184',
  '#e0576d',
  '#a97bf0',
  '#35bec9',
  '#d98cc4',
  '#8ea6bd',
  '#c9b356',
  '#6fb3e0'
]

export const laneColor = (index: number) => LANE_COLORS[index % LANE_COLORS.length]

export function relativeTime(seconds: number) {
  const diff = Date.now() / 1000 - seconds
  if (diff < 60) return 'just now'
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  if (diff < 86400 * 30) return `${Math.floor(diff / 86400)}d ago`
  return new Date(seconds * 1000).toLocaleDateString()
}

export function fullTime(seconds: number) {
  return new Date(seconds * 1000).toLocaleString()
}

/** Copies text, reporting through the activity log either way. */
export async function copyText(text: string, label = 'Copied') {
  const { note } = useGit()
  try {
    await navigator.clipboard.writeText(text)
    note(`${label}: ${text.split('\n')[0].slice(0, 60)}`)
    return true
  } catch {
    // Some webview builds refuse the async API without a user gesture; the old
    // one still works.
    try {
      const field = document.createElement('textarea')
      field.value = text
      field.style.position = 'fixed'
      field.style.opacity = '0'
      document.body.appendChild(field)
      field.select()
      document.execCommand('copy')
      field.remove()
      note(`${label}: ${text.split('\n')[0].slice(0, 60)}`)
      return true
    } catch (error) {
      note(`Could not copy: ${String(error)}`, 'error')
      return false
    }
  }
}

/** True when a commit matches the search box: message, author or hash. */
export function rowMatches(row: GraphRow, query: string) {
  const needle = query.trim().toLowerCase()
  if (!needle) return false
  return (
    row.summary.toLowerCase().includes(needle) ||
    row.author.toLowerCase().includes(needle) ||
    row.email.toLowerCase().includes(needle) ||
    row.oid.startsWith(needle) ||
    row.labels.some((label) => label.name.toLowerCase().includes(needle))
  )
}

/** Splits text into matched and unmatched pieces, for highlighting. */
export function highlight(text: string, query: string) {
  const needle = query.trim()
  if (!needle) return [{ text, hit: false }]
  const parts: { text: string; hit: boolean }[] = []
  const lower = text.toLowerCase()
  const target = needle.toLowerCase()
  let at = 0
  while (at < text.length) {
    const found = lower.indexOf(target, at)
    if (found === -1) {
      parts.push({ text: text.slice(at), hit: false })
      break
    }
    if (found > at) parts.push({ text: text.slice(at, found), hit: false })
    parts.push({ text: text.slice(found, found + needle.length), hit: true })
    at = found + needle.length
  }
  return parts
}
