import { invoke } from '@tauri-apps/api/core'
import { markRaw, reactive, ref } from 'vue'
import { useConfig } from './useConfig'

/**
 * What the platform calls its file manager. The backend already opens the right
 * one; this is only so the menu does not offer a Mac user's Finder to someone
 * on Windows.
 */
function fileManagerName(): string {
  const agent = typeof navigator === 'undefined' ? '' : navigator.userAgent
  if (agent.includes('Windows')) return 'Explorer'
  if (agent.includes('Mac OS')) return 'Finder'
  return 'the file manager'
}

export interface RepoInfo {
  path: string
  name: string
  head: string
  detached: boolean
  state: string
  /** This repository's effective `user.name`; empty when git has none. */
  author: string
  /** The matching `user.email`, which the author's picture is looked up by. */
  author_email: string
}

/** A repository that has just been cloned or created. */
export interface NewRepo {
  path: string
  name: string
  /** Why there is no first commit, when there is not one. */
  note: string | null
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
export interface Tag {
  name: string
  /** The commit the tag names, peeled: never an annotated tag's own object. */
  oid: string
  annotated: boolean
  /** The first line of an annotated tag's message. */
  message: string | null
  /** Seconds since the epoch — the tagger's time, or the commit's. */
  when: number
}
export interface Stash { index: number; message: string }

export interface RefTree {
  locals: LocalBranch[]
  remotes: RemoteBranch[]
  tags: Tag[]
  stashes: Stash[]
}

/** What git is part-way through, so the way out can be named correctly. */
export interface InProgress {
  merging: boolean
  rebasing: boolean
  cherry_picking: boolean
  reverting: boolean
  /** Conflicts with nothing running: what a failed `stash pop` leaves. */
  restoring: boolean
}

export interface StatusEntry { path: string; kind: string }
export interface WorkingStatus {
  staged: StatusEntry[]
  unstaged: StatusEntry[]
  conflicted: string[]
}

export interface Segment { x1: number; y1: number; x2: number; y2: number; color: number }
export interface RefLabel {
  kind: string
  name: string
  /** The checked-out branch, or a detached HEAD. */
  head: boolean
}

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
  /** On a local branch but not yet on its upstream. */
  unpushed: boolean
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
export interface FileDiff {
  path: string
  binary: boolean
  hunks: DiffHunk[]
  /** Lines the backend stopped collecting; 0 for any diff worth reading. */
  truncated: number
}

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

/**
 * A push the remote turned down because the branch has moved on there.
 *
 * Kept in the store rather than thrown away with the error, because the way out
 * — pull, or rewrite the remote — is a choice the toolbar has to offer.
 */
export interface PushBlock {
  remote: string
  branch: string
  upstream: string | null
  /** Git's own rejection, shown so the offer is not taken on trust. */
  message: string
}

/**
 * Whether git turned a push down for being behind, as opposed to failing for
 * some other reason. Only these are worth offering a pull or a rewrite for.
 */
function isRejectedForBeingBehind(out: CmdOutput): boolean {
  if (out.ok) return false
  const text = `${out.stdout}\n${out.stderr}`
  return (
    text.includes('[rejected]') &&
    (text.includes('non-fast-forward') ||
      text.includes('fetch first') ||
      text.includes('stale info'))
  )
}

export interface CherryPickOptions {
  /** Stage the changes but do not commit, so they can be re-split first. */
  no_commit?: boolean
  /** Record "(cherry picked from commit …)" in the message. */
  record_origin?: boolean
}

/** How two branches stand to each other. */
export interface BranchRelation {
  /** Commits the source has that the target does not. */
  ahead: number
  /** Commits the target has that the source does not. */
  behind: number
}

/** What deleting a branch would cost, read before the question is asked. */
/** A copy of the branch on a remote, and what deleting it there would cost. */
export interface RemoteCopy {
  /** Full remote-tracking name, e.g. `origin/feature`. */
  name: string
  remote: string
  /** Commits on the remote copy that the branch you are on cannot reach. */
  unmerged: number
}

export interface BranchDeletion {
  name: string
  is_head: boolean
  /** Reachable from HEAD. */
  merged: boolean
  /** The branch HEAD is on; null on a detached HEAD. */
  head: string | null
  /** Other local branches that can also reach the tip. */
  also_on: string[]
  /** Commits on the branch that HEAD cannot reach. */
  only_here: number
  upstream: string | null
  unpushed: number
  /** The copy on the branch's own remote: the only one a delete is offered for. */
  remote: RemoteCopy | null
  /** Copies on forks and mirrors, named but never deleted from here. */
  other_remotes: string[]
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

/** Which of the three index stages a conflicted path has. */
export interface ConflictStages {
  base: boolean
  ours: boolean
  theirs: boolean
}

export interface ConflictFile {
  path: string
  blocks: ConflictBlock[]
  conflict_count: number
  stages: ConflictStages
}

export interface Resolution {
  take_ours: boolean
  take_theirs: boolean
  ours_first: boolean
  custom?: string[] | null
}

/** One line in the activity log at the bottom of the window. */
/**
 * `command` is a git command line the app ran, shown as it would be typed.
 * It reads differently from a result and is styled differently for it.
 */
export type LogLevel = 'info' | 'error' | 'command'

export interface LogLine {
  id: number
  at: number
  level: LogLevel
  text: string
}

/** How many commits a page is, before the settings have been read. */
const COMMIT_PAGE = 500

/**
 * How many commits to ask for at a time.
 *
 * The backend takes this setting into account only when the frontend sends no
 * limit at all, and the frontend always sent one — so "commits per page" in
 * settings moved nothing. It is read here instead, which is where the number
 * is actually decided.
 */
function pageSize(): number {
  const size = useConfig().settings.value?.graph_page_size
  return size && size > 0 ? size : COMMIT_PAGE
}

/** Stands in for the working tree in `store.selected`. */
export const WIP = '__working__' 

// A single shared store: the app has one open repository at a time, so there is
// nothing to gain from per-component state.
const fields = reactive({
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
  /** What git is part-way through, when it is part-way through anything. */
  progress: null as InProgress | null,
  /** Free-text filter over the loaded commits. */
  query: '',
  /** A commit the graph should scroll into view. Carries a sequence number so
      asking for the same commit twice still moves the graph. */
  revealing: null as { oid: string; seq: number } | null,
  /** When set, a file is open full width in place of the graph. */
  viewer: null as { path: string; side?: 'staged' | 'unstaged'; commit?: string } | null,
  /** Number of calls in flight; a counter rather than a flag so overlapping
      operations cannot switch it off early. */
  pending: 0,
  /** What the newest in-flight call is doing, for the progress bar. */
  busyLabel: null as string | null,
  /** Set when a push was rejected, so the toolbar can offer a way out. */
  pushBlocked: null as PushBlock | null,
  log: [] as LogLine[]
})

const logSeq = ref(0)

/** Module scope, so two components asking to reveal cannot mint the same
    sequence number and leave the second request looking like a repeat. */
let revealSeq = 0

// Everything already written reads `store.busy`; keep it as a live getter over
// the counter rather than a second source of truth.
Object.defineProperty(fields, 'busy', {
  get: () => fields.pending > 0,
  enumerable: true
})

/**
 * The store as the rest of the app sees it.
 *
 * `busy` is attached above rather than declared in the object, which left it
 * invisible to TypeScript — so every `store.busy` in a template was an error
 * nobody saw. The cast says what the getter already does.
 */
const store = fields as typeof fields & { readonly busy: boolean }

/**
 * Keeps a backend payload out of Vue's reach.
 *
 * A graph page for a busy repository is a few hundred rows carrying tens of
 * thousands of line segments between them, and a reactive store turns every
 * one of those objects into a proxy on the way to the screen. Nothing here is
 * ever edited in place — each read replaces the last wholesale — so the
 * tracking buys nothing and the raw object is handed over as it arrived.
 */
function raw<T>(value: T): T {
  return value && typeof value === 'object' ? (markRaw(value as object) as T) : value
}

/**
 * What one repository was last seen holding.
 *
 * Two jobs. Switching tabs paints the old picture at once instead of leaving
 * the window empty for as long as seven backend reads take, and a refresh
 * compares what came back against what is already drawn — because a refresh
 * runs on every window focus and every write under `.git`, and almost always
 * reads back exactly what is on screen already.
 */
interface Snapshot {
  info: RepoInfo
  refs: RefTree | null
  status: WorkingStatus | null
  rows: GraphRow[]
  hasMore: boolean
  limit: number
  stashes: StashEntry[]
  history: Stacks
  progress: InProgress | null
  /** Each of the above serialized, so the next read can tell what moved. */
  seen: Record<string, string>
}

const snapshots = new Map<string, Snapshot>()

/** A big repository's snapshot is a megabyte or so; a handful is plenty. */
const SNAPSHOT_LIMIT = 8

function remember(path: string, snapshot: Snapshot) {
  // Re-inserting moves it to the end, so the one dropped is the least recently
  // looked at rather than the one opened longest ago.
  snapshots.delete(path)
  snapshots.set(path, snapshot)
  while (snapshots.size > SNAPSHOT_LIMIT) {
    const oldest = snapshots.keys().next().value
    if (oldest === undefined) break
    snapshots.delete(oldest)
  }
}

/** Empties everything a refresh fills, leaving the view state alone. */
function clearData() {
  store.refs = null
  store.status = null
  store.rows = []
  store.hasMore = false
  store.limit = pageSize()
  store.stashes = []
  store.history = { undo: [], redo: [] }
  store.progress = null
}

/** Puts a remembered snapshot on screen, whole. */
function paint(snapshot: Snapshot) {
  store.refs = snapshot.refs
  store.status = snapshot.status
  store.rows = snapshot.rows
  store.hasMore = snapshot.hasMore
  store.limit = snapshot.limit
  store.stashes = snapshot.stashes
  store.history = snapshot.history
  store.progress = snapshot.progress
}

function note(text: string, level: LogLevel = 'info') {
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

/**
 * Puts the store back to having no repository open.
 *
 * Closing the last tab used to clear the repository and the commit list and
 * leave the rest — so the file viewer, the refs, the status and a rejected push
 * all survived into the welcome pane and were waiting, describing a repository
 * that was no longer open, for whatever opened next. Written once here rather
 * than at each of the two places that empty the store, which is how the two
 * came to disagree about what emptying it means.
 */
function forget() {
  store.repo = null
  clearData()
  store.detail = null
  store.selected = WIP
  store.pushBlocked = null
  store.resolving = null
  store.revealing = null
  store.viewer = null
  store.query = ''
}

export function useGit() {
  async function openRepo(path: string) {
    const info = await guard('Open repository', () =>
      invoke<RepoInfo>('open_repo', { path })
    )
    if (!info) return false
    store.repo = info
    store.selected = WIP
    store.detail = null
    // Everything below is about the repository that was open, and would
    // otherwise act on this one: a rejected push offering to force a branch
    // that is not here, a file viewer on a path that no longer exists.
    store.pushBlocked = null
    store.viewer = null
    store.resolving = null
    store.query = ''

    // Whatever this tab was showing last time, back on screen before the reads
    // below are even sent. A tab that has not been opened this session starts
    // empty rather than wearing the last repository's branches, which is what
    // used to sit there until the reads landed.
    const previous = snapshots.get(info.path)
    if (previous) paint(previous)
    else clearData()

    note(`Opened ${info.path}`)
    // A profile is a person, so opening a repository under one is statement
    // enough: it commits as them from here on. Spoken only when that actually
    // changed something, and never fatal — a repository is still usable when
    // its config will not take a write.
    const identity = await invoke<string | null>('apply_identity').catch(() => null)
    if (identity) note(identity)
    await refresh()
    return true
  }

  /** Reloads refs, status and the graph. Called after anything that mutates. */
  /**
   * Re-reads only the working tree status.
   *
   * A file saved in an editor changes what is staged and what is not, and
   * nothing else. Walking the whole history to find that out is what makes a
   * watcher expensive, so the cheap question has its own answer.
   */
  async function refreshStatus() {
    if (!store.repo) return
    const path = store.repo.path
    const status = await part('the working tree', invoke<WorkingStatus>('working_status'), null)
    if (!status || store.repo?.path !== path) return
    // A save in an editor that touched nothing git cares about still wakes the
    // watcher; comparing costs nothing next to rebuilding the panel.
    const snapshot = snapshots.get(path)
    if (snapshot) {
      const key = JSON.stringify(status)
      if (snapshot.seen.status === key) return
      snapshot.seen.status = key
      snapshot.status = raw(status)
      store.status = snapshot.status
      return
    }
    store.status = raw(status)
  }

  /**
   * Runs one of the reads a refresh is made of.
   *
   * A read that fails leaves the panel it feeds showing what it showed before,
   * which is the right thing on screen — but silently, and a status that has
   * stopped updating looks exactly like a repository that has stopped changing.
   * So the failure is kept and the previous value returned.
   */
  async function part<T>(what: string, call: Promise<T>, fallback: T): Promise<T> {
    try {
      return await call
    } catch (error) {
      note(`Could not read ${what}: ${String(error)}`, 'error')
      return fallback
    }
  }

  async function refresh() {
    if (!store.repo) return
    const path = store.repo.path
    const limit = store.limit
    const [info, refs, status, page, stashes, history, progress] = await Promise.all([
      part('the repository', invoke<RepoInfo>('repo_info'), store.repo),
      part('the branches', invoke<RefTree>('ref_tree'), null),
      part('the working tree', invoke<WorkingStatus>('working_status'), null),
      part(
        'the history',
        invoke<{ rows: GraphRow[]; has_more: boolean }>('commit_graph', { limit }),
        null
      ),
      part('the stashes', invoke<StashEntry[]>('stash_list'), [] as StashEntry[]),
      part('the undo history', invoke<Stacks>('history'), { undo: [], redo: [] } as Stacks),
      part('what git is doing', invoke<InProgress>('in_progress'), null)
    ])
    // A tab switched away from mid-read must not have its answers land on
    // whatever is open now. The backend holds one repository at a time, so a
    // switch part-way through leaves these reads describing a mixture of two —
    // and both the store and the snapshot are better off without them.
    if (store.repo?.path !== path || (info && info.path !== path)) return

    // Only what actually moved is written to the store, because writing a
    // field is what rebuilds the panel reading it — and most refreshes,
    // triggered by a window focus or a file being saved, find nothing new.
    const seen = snapshots.get(path)?.seen ?? {}
    const settle = <T>(field: string, value: T, current: T): T => {
      const key = JSON.stringify(value) ?? 'undefined'
      if (seen[field] === key) return current
      seen[field] = key
      return raw(value)
    }

    if (info) store.repo = settle('info', info, store.repo)
    if (refs) store.refs = settle('refs', refs, store.refs)
    if (status) store.status = settle('status', status, store.status)
    if (page) {
      store.rows = settle('rows', page.rows, store.rows)
      store.hasMore = page.has_more
    }
    store.stashes = settle('stashes', stashes ?? [], store.stashes)
    store.history = settle('history', history ?? { undo: [], redo: [] }, store.history)
    store.progress = settle('progress', progress, store.progress)

    remember(path, {
      info: store.repo!,
      refs: store.refs,
      status: store.status,
      rows: store.rows,
      hasMore: store.hasMore,
      limit,
      stashes: store.stashes,
      history: store.history,
      progress: store.progress,
      seen
    })

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
    store.limit += pageSize()
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

  /**
   * Selects a commit and asks the graph to bring it into view.
   *
   * What a single click on a branch does: pointing at a branch is a question
   * about where it is and what was last put on it, which is answered without
   * touching the working tree. Checking out stays on the double click.
   */
  async function revealCommit(oid: string) {
    revealSeq += 1
    store.revealing = { oid, seq: revealSeq }
    await select(oid)
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

  /**
   * The whole file, for the view that marks the changes in place.
   *
   * Unguarded: a file that cannot be read whole — a binary, or something the
   * size of a lockfile — is an ordinary answer here, and the viewer says so
   * where the file would have been rather than in a passing notice.
   */
  const fileText = (path: string, commit?: string | null, side?: 'staged' | 'unstaged' | null) =>
    invoke<string>('file_text', { path, commit: commit ?? null, side: side ?? null })

  async function run<T>(label: string, command: string, args: Record<string, unknown> = {}) {
    const result = await guard(label, () => invoke<T>(command, args))
    // Refresh whether or not it worked. A git command that fails can still have
    // changed the repository — a branch switch whose stash pop conflicted has
    // switched, a rebase that stopped has moved HEAD — and leaving the window
    // showing the state from before is worse than an extra read.
    await refresh()
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
    forget,
    store,
    note,
    refresh,
    refreshStatus,
    loadMore,
    openRepo,
    select,
    revealCommit,
    selectStash,
    commitFileDiff,
    workingFileDiff,
    fileText,
    run,
    report,

    checkout: (name: string) => run<string>('Checkout', 'checkout', { name }),
    createBranch: (name: string, start?: string) =>
      run<string>('Create branch', 'create_branch', { name, start, checkout: true }),
    deleteBranch: (name: string, force = false) =>
      run<string>('Delete branch', 'delete_branch', { name, force }),
    /** How two branches stand, so a menu offers only the moves that would do
        something. `ahead` is what source has and target does not. */
    branchRelation: (source: string, target: string) =>
      guard('Compare branches', () =>
        invoke<BranchRelation>('branch_relation', { source, target })
      ),
    /** What deleting would cost, so the dialog can ask a real question. */
    deleteBranchPreview: (name: string) =>
      guard('Read branch', () => invoke<BranchDeletion>('delete_branch_preview', { name })),
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
    revealLabel: `Reveal in ${fileManagerName()}`,

    /** The address a remote fetches from, for the edit form. */
    remoteUrl: (name: string) =>
      guard('Read remote', () => invoke<string>('remote_url', { remote: name })),
    remoteAdd: (name: string, url: string) =>
      run<string>('Add remote', 'remote_add', { name, url }),
    remoteSetUrl: (name: string, url: string) =>
      run<string>('Change remote address', 'remote_set_url', { name, url }),
    remoteRename: (from: string, to: string) =>
      run<string>('Rename remote', 'remote_rename', { from, to }),
    remoteRemove: (name: string) => run<string>('Remove remote', 'remote_remove', { name }),

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
    /**
     * Copies one or more commits onto the current branch. The backend puts them
     * in history order, so the caller's selection order does not matter.
     */
    cherryPick: (oids: string[], options: CherryPickOptions = {}) =>
      run<string>('Cherry-pick', 'cherry_pick', { oids, options }),
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
      // A rejection for being behind has an answer; hand it to the toolbar
      // instead of leaving the user to read git's advice in the log.
      store.pushBlocked =
        out && isRejectedForBeingBehind(out)
          ? {
              remote: remoteName,
              branch,
              upstream: store.refs?.locals.find((b) => b.name === branch)?.upstream ?? null,
              message: `${out.stdout}\n${out.stderr}`.trim()
            }
          : null
      await refresh()
      return ok
    },
    dismissPushBlock: () => {
      store.pushBlocked = null
    },
    /** Pushes one branch by name, rather than whatever is checked out. */
    /**
     * Brings a branch up to date whether or not it is checked out, and whether
     * or not there are open changes. The backend does whatever that takes.
     */
    pullBranch: async (branch: string, rebase = false) => {
      const out = await guard('Pull', () =>
        invoke<CmdOutput>('pull_branch', { branch, rebase })
      )
      const ok = report('Pull', out)
      await refresh()
      return ok
    },
    pushBranch: async (branch: string, setUpstream: boolean) => {
      // A branch that already tracks something goes back where it came from,
      // whichever remote that is; only a new branch has to guess.
      const upstream = store.refs?.locals.find((b) => b.name === branch)?.upstream ?? null
      const tracked = upstream?.split('/')[0] ?? null
      const remotes = tracked ? [] : await invoke<string[]>('remotes').catch(() => [])
      const target = tracked ?? remotes[0] ?? 'origin'
      const out = await guard('Push', () =>
        invoke<CmdOutput>('push', {
          remoteName: target,
          branch,
          force: false,
          setUpstream
        })
      )
      const ok = report('Push', out)
      store.pushBlocked =
        out && isRejectedForBeingBehind(out)
          ? {
              remote: target,
              branch,
              upstream,
              message: `${out.stdout}\n${out.stderr}`.trim()
            }
          : null
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
    /**
     * Merges one branch into another, neither of which need be checked out.
     *
     * Git merges into where you stand, which makes "merge these two" a chore of
     * checking out, merging and remembering to come back. The other side works
     * out which of those steps are actually needed, so this is the one to call
     * whenever the target is named rather than implied.
     */
    mergeInto: async (source: string, target: string, noFf = false) => {
      const outcome = await guard('Merge', () =>
        invoke<MergeOutcome>('merge_into', { source, target, noFf })
      )
      if (outcome) {
        note(`Merge ${source} into ${target}: ${outcome.message}`, outcome.ok ? 'info' : 'error')
      }
      await refresh()
      return outcome
    },
    rebaseBranch: async (branch: string, onto: string) => {
      const outcome = await guard('Rebase', () =>
        invoke<MergeOutcome>('rebase_branch', { branch, onto })
      )
      if (outcome) {
        note(`Rebase ${branch} onto ${onto}: ${outcome.message}`, outcome.ok ? 'info' : 'error')
      }
      await refresh()
      return outcome
    },
    abortMerge: () => run<string>('Abort merge', 'abort_merge'),
    /** The way out of a conflicted auto-stash, which no abort can undo. */
    undoRestore: () => run<string>('Undo the switch', 'undo_restore'),

    conflictRead: (path: string) =>
      guard('Read conflict', () => invoke<ConflictFile>('conflict_read', { path })),
    conflictPreview: (path: string, choices: Resolution[]) =>
      guard('Preview resolution', () =>
        invoke<string>('conflict_preview', { path, choices })
      ),
    conflictResolve: (path: string, choices: Resolution[]) =>
      run<string>('Resolve', 'conflict_resolve', { path, choices }),
    conflictResolveWhole: (path: string, side: 'ours' | 'theirs') =>
      run<string>('Resolve', 'conflict_resolve_whole', { path, side }),
    /** Ends a conflict by staging the file exactly as it stands on disk. */
    conflictResolveAsIs: (path: string) =>
      run<string>('Resolve', 'conflict_resolve_as_is', { path })
  }
}

/**
 * The colours the graph's lines are drawn in, in the order they are handed out.
 *
 * The whole job of a lane colour is that two lines running side by side are
 * told apart at a glance, so every entry has to be an unmistakably different
 * hue rather than a lighter version of one already in the list. The greyed
 * slate and the dull gold that used to sit at the end failed that twice over:
 * against this background they read as a line that had been dimmed on purpose,
 * and next to the blue at the top the pale blue at the bottom read as the same
 * line.
 *
 * Index 0 is the trunk's, and the backend holds it back from every other line.
 * Its `PALETTE` counts these, so the two lists have to stay the same length.
 */
export const LANE_COLORS = [
  '#4f9cf9',
  '#f0a83c',
  '#57c184',
  '#e0576d',
  '#a97bf0',
  '#35bec9',
  '#f07ab8',
  '#8fd14f',
  '#f2724b',
  '#7d8cf8'
]

/** The modulo keeps the index in range, which the checker cannot work out. */
export const laneColor = (index: number): string => LANE_COLORS[index % LANE_COLORS.length]!

/**
 * A lane's colour at a given opacity.
 *
 * The chips in the branch column are filled with the colour of the line they
 * name, which only reads as a fill rather than as a block of paint if it is
 * mostly background. `color-mix` would do it in CSS, but the colour arrives
 * bound per element rather than as a class, so handing over the finished rgba
 * saves the stylesheet from having to know about lanes at all.
 */
export function laneTint(index: number, alpha: number) {
  const value = parseInt(laneColor(index).slice(1), 16)
  return `rgba(${(value >> 16) & 255}, ${(value >> 8) & 255}, ${value & 255}, ${alpha})`
}

/*
 * Built once and reused. `toLocaleDateString` and `toLocaleString` construct a
 * formatter per call, which is the most expensive thing the commit list does
 * per row: every commit older than a month takes the date branch below, so a
 * window of fifty rows built a formatter fifty times on every scroll frame.
 */
const DATE_ONLY = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' })
const DATE_TIME = new Intl.DateTimeFormat(undefined, {
  dateStyle: 'medium',
  timeStyle: 'short'
})

export function relativeTime(seconds: number) {
  const diff = Date.now() / 1000 - seconds
  if (diff < 60) return 'just now'
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  if (diff < 86400 * 30) return `${Math.floor(diff / 86400)}d ago`
  return DATE_ONLY.format(seconds * 1000)
}

export function fullTime(seconds: number) {
  return DATE_TIME.format(seconds * 1000)
}

/** Copies text, reporting through the activity log either way. */
export async function copyText(text: string, label = 'Copied') {
  const { note } = useGit()
  try {
    await navigator.clipboard.writeText(text)
    note(`${label}: ${(text.split('\n')[0] ?? text).slice(0, 60)}`)
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
      note(`${label}: ${(text.split('\n')[0] ?? text).slice(0, 60)}`)
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
