import { aimAt, invoke } from './useInvoke'
import { markRaw, reactive, ref } from 'vue'
import { useConfig } from './useConfig'
import { parseCommandLine } from './cli'
import type { LfsStatus } from './useLfs'
import { useToasts } from './useToasts'

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

/** A local branch and its remote copy that have both moved on. */
export interface Diverged {
  branch: string
  /** The remote-tracking ref it was measured against, e.g. `origin/main`. */
  upstream: string
  /** Commits only the local branch has. */
  ahead: number
  /** Commits only the remote has. */
  behind: number
}

export interface CheckoutOutcome {
  message: string
  /** Set when settings say to ask what to do about the divergence. */
  diverged: Diverged | null
}

/** One run of consecutive lines that came in with the same commit. */
export interface BlameRun {
  oid: string
  short: string
  summary: string
  author: string
  email: string
  time: number
  /** The first line of the run, counting from one. */
  start: number
  lines: number
  /** Work that is not committed yet, which blame cannot answer for. */
  uncommitted: boolean
}

/** One commit in a file's own history. */
export interface FileCommit {
  oid: string
  short: string
  author: string
  email: string
  time: number
  summary: string
}

/** What git made of a commit's signature. */
export type SignatureVerdict = 'good' | 'untrusted' | 'bad' | 'unchecked' | 'none'

/** The mark one row of the graph carries. Rows with no signature carry none. */
export interface SignatureMark {
  verdict: SignatureVerdict
  signer: string | null
}

/** Everything git will say about one commit's signature. */
export interface CommitSignature {
  verdict: SignatureVerdict | null
  signer: string | null
  key: string | null
  fingerprint: string | null
  /** gpg's or ssh-keygen's own words, for the fold-out under the line. */
  raw: string | null
}

/** What this repository would do if you committed right now. */
export interface SigningSetup {
  signs: boolean
  signs_tags: boolean
  /** `openpgp`, `ssh` or `x509`. */
  format: string
  key: string | null
}

/** Where a submodule stands, from the mark `git submodule status` prints. */
export type SubmoduleState = 'ready' | 'absent' | 'moved' | 'conflicted'

/** One repository kept inside this one. */
export interface Submodule {
  /** The name in `.gitmodules`, which is not always the path. */
  name: string
  path: string
  /** The same place, absolute, so it can be opened as a repository of its own. */
  abs: string
  url: string | null
  branch: string | null
  oid: string
  short: string
  /** What `git describe` made of the pinned commit, when it made anything. */
  described: string | null
  state: SubmoduleState
}

/** One folder this repository is checked out into. */
export interface Worktree {
  path: string
  /** The folder's own name, which is how the row reads. */
  name: string
  /** The branch checked out there; null for a detached HEAD. */
  branch: string | null
  oid: string
  /** The folder the repository itself lives in. */
  is_main: boolean
  /** The folder this window has open right now. */
  is_current: boolean
  locked: boolean
}

/** What git is part-way through, so the way out can be named correctly. */
export interface InProgress {
  merging: boolean
  rebasing: boolean
  cherry_picking: boolean
  reverting: boolean
  /** Conflicts with nothing running: what a failed `stash pop` leaves. */
  restoring: boolean
  /** The message git already wrote for it, e.g. "Merge branch 'x' into 'y'". */
  prepared: string | null
}

export interface StatusEntry { path: string; kind: string }
export interface WorkingStatus {
  staged: StatusEntry[]
  unstaged: StatusEntry[]
  conflicted: string[]
}

export interface Segment {
  x1: number
  y1: number
  x2: number
  y2: number
  color: number
  /** A stash's line, drawn broken: it hangs off the history, it is not in it. */
  dashed: boolean
}
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
  /** Which stash this row is, when it is one rather than a commit. */
  stash: number | null
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
  /**
   * `' '` context, `'+'` added, `'-'` deleted, and `'\\'` for git's "no
   * newline at end of file", which is a remark about its neighbours rather
   * than a line of either file and carries no line number.
   */
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
  /** Other local branches that can also reach the tip. Not a promise: a branch
      that gets reset holds every commit right up until it does not. */
  also_on: string[]
  /** The branch everything was measured against: the trunk, or HEAD when the
      repository has no trunk to speak of. */
  against: string | null
  /** The trunk already holds every commit on the branch. */
  trunk_holds: boolean
  /** Commits on the branch that `against` cannot reach. */
  only_here: number
  upstream: string | null
  unpushed: number
  /** The copy on the branch's own remote: the only one a delete is offered for. */
  remote: RemoteCopy | null
  /** Copies on forks and mirrors, named but never deleted from here. */
  other_remotes: string[]
}

/**
 * The branch a repository is organised around.
 *
 * Named by the user where they have said so, guessed from the usual names
 * otherwise. What "has this work landed?" is really asking about.
 */
export interface Trunk {
  /** `main`, `master`, `origin/main`… or null when the repository has none. */
  name: string | null
  /** True when this repository was told, rather than the names being tried. */
  chosen: boolean
}

export interface MergeOutcome { ok: boolean; message: string; conflicts: string[] }
export interface AmendDraft { summary: string; body: string; is_pushed: boolean; short: string }

/** What rewording a given commit would take, asked before the editor opens. */
export interface RewordCheck {
  summary: string
  body: string
  /** False when it is not the newest commit; `reason` says so. */
  can: boolean
  reason: string | null
  /** True when a remote already has it, so rewording needs a force push. */
  is_pushed: boolean
}

export interface StashEntry {
  index: number
  oid: string
  message: string
  branch: string | null
  time: number
  files: number
}

/** What a run over several stashes did. */
export interface StashRun {
  /** The ones that went on, oldest first. */
  applied: string[]
  /** The one that stopped the run, when one did. */
  stopped: { message: string; reason: string } | null
  /** Files the stash that stopped it left with both sides in them. */
  conflicted: string[]
}

export interface HistoryEntry {
  id: number
  label: string
  kind: string
  branch: string | null
  before: string | null
  after: string | null
  mode: 'soft' | 'hard' | 'stash'
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
 * `command` is a git command line the app ran, shown as it would be typed. It
 * reads differently from a result and is styled differently for it, and
 * `failed` is one of those that came back non-zero.
 *
 * A failed command line is not an `error`. The two shared a level while red
 * text was all either of them got; now that an error also raises a notice, a
 * sequence where six commands fail on the way to one reported failure would
 * raise seven, and the one worth reading would be the one underneath.
 */
/** `output` is what a typed command printed: shown as it came, never a notice. */
export type LogLevel = 'info' | 'error' | 'command' | 'failed' | 'output'

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

/**
 * How deep a commit can sit and still be worth loading the graph down to.
 *
 * Twenty thousand rows is a few seconds and some tens of megabytes; past that
 * the honest answer is that the commit is too far back to scroll to.
 */
const REVEAL_LIMIT = 20000

/** Stands in for the working tree in `store.selected`. */
export const WIP = '__working__' 

// A single shared store: the app has one open repository at a time, so there is
// nothing to gain from per-component state.
const fields = reactive({
  repo: null as RepoInfo | null,
  refs: null as RefTree | null,
  /** The branch this repository is organised around. */
  trunk: { name: null, chosen: false } as Trunk,
  status: null as WorkingStatus | null,
  /** Every folder the repository is checked out into; one entry is this one. */
  worktrees: [] as Worktree[],
  /** Every repository kept inside this one. Empty for almost every project. */
  submodules: [] as Submodule[],
  /** Signature verdicts by commit, empty while the setting is off. */
  signatures: {} as Record<string, SignatureMark>,
  /** Whether a commit made here would be signed, and with what. */
  signing: null as SigningSetup | null,
  /** Whether this repository uses LFS, and whether the tool for it is here. */
  lfs: null as LfsStatus | null,
  /**
   * The stash being read in the content view, by commit id.
   *
   * By id rather than by position: applying or dropping one renumbers every
   * entry below it, and a pane holding a number would quietly start showing
   * somebody else's stash.
   */
  stashView: null as string | null,
  /**
   * The submodules stepped into to reach what is on screen, outermost first.
   *
   * Empty for an ordinary repository. Each entry is where to go back to and
   * what to call both ends of the step, so the toolbar can draw the trail and
   * the way out of it. A chain rather than one entry, because a submodule can
   * have submodules of its own. The name it came from is carried rather than
   * read off the path, because a project is called whatever the profile calls
   * it.
   */
  inside: [] as { path: string; name: string; from: string; fromName: string }[],
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
  /** Set when a checkout found the branch and its remote had both moved on
      and settings said to ask, so the toolbar can put the question up. */
  divergedCheckout: null as Diverged | null,
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
  trunk: Trunk
  status: WorkingStatus | null
  worktrees: Worktree[]
  submodules: Submodule[]
  signatures: Record<string, SignatureMark>
  signing: SigningSetup | null
  lfs: LfsStatus | null
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
  store.trunk = { name: null, chosen: false }
  store.status = null
  store.worktrees = []
  store.submodules = []
  store.signatures = {}
  store.signing = null
  store.lfs = null
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
  store.trunk = snapshot.trunk
  store.status = snapshot.status
  store.worktrees = snapshot.worktrees
  store.submodules = snapshot.submodules
  store.signatures = snapshot.signatures
  store.signing = snapshot.signing
  store.lfs = snapshot.lfs
  store.rows = snapshot.rows
  store.hasMore = snapshot.hasMore
  store.limit = snapshot.limit
  store.stashes = snapshot.stashes
  store.history = snapshot.history
  store.progress = snapshot.progress
}

/**
 * Writes a line to the activity log, and stands a failure up in the corner.
 *
 * The log is a record: it holds everything, in order, and it is where you go
 * to find out what the app ran. That makes it the wrong place to be told
 * something went wrong — the next refresh writes six more lines over it. So
 * anything at `error` also becomes a notice that stays until it is dismissed,
 * which is every failure the app reports and none of the command lines.
 */
function note(text: string, level: LogLevel = 'info') {
  if (!text.trim()) return
  store.log.unshift({ id: ++logSeq.value, at: Date.now(), level, text: text.trim() })
  if (store.log.length > 200) store.log.length = 200
  if (level === 'error') useToasts().fail(text)
}

/** An argument as it would have to be typed: quoted only when it needs it. */
function quoteArg(arg: string) {
  return arg === '' || /[\s'"\\]/.test(arg) ? `'${arg.replace(/'/g, "'\\''")}'` : arg
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
  // No repository is open, so nothing that follows is about one.
  aimAt(null)
  store.repo = null
  clearData()
  store.detail = null
  store.selected = WIP
  store.pushBlocked = null
  store.divergedCheckout = null
  store.resolving = null
  store.revealing = null
  store.viewer = null
  store.inside = []
  store.stashView = null
  store.query = ''
}

export function useGit() {
  /**
   * Opens a repository and points every later call at it.
   *
   * `record` false steps into one without it becoming a tab: a submodule is a
   * repository, but it is not a project the user opened, and a tab strip that
   * grows a new entry every time you look inside one is a tab strip nobody
   * asked for.
   */
  async function openRepo(path: string, record = true) {
    const info = await guard('Open repository', () =>
      invoke<RepoInfo>('open_repo', { path, record })
    )
    if (!info) return false
    // From here every call says it is about this repository, so a switch to
    // another tab cannot retarget work that is already under way.
    aimAt(info.path)
    store.repo = info
    store.selected = WIP
    store.detail = null
    // Everything below is about the repository that was open, and would
    // otherwise act on this one: a rejected push offering to force a branch
    // that is not here, a file viewer on a path that no longer exists.
    store.pushBlocked = null
    store.divergedCheckout = null
    store.viewer = null
    store.resolving = null
    store.stashView = null
    store.query = ''
    // Opening a project is leaving whatever submodule was being looked at.
    // Stepping into one passes `record` false and keeps the trail, which the
    // caller then adds to.
    if (record) store.inside = []

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
    const [
      info,
      refs,
      trunk,
      status,
      worktrees,
      submodules,
      signatures,
      signing,
      lfs,
      page,
      stashes,
      history,
      progress
    ] = await Promise.all([
      part('the repository', invoke<RepoInfo>('repo_info'), store.repo),
      part('the branches', invoke<RefTree>('ref_tree'), null),
      part('the main branch', invoke<Trunk>('trunk_branch'), store.trunk),
      part('the working tree', invoke<WorkingStatus>('working_status'), null),
      part('the worktrees', invoke<Worktree[]>('worktree_list'), [] as Worktree[]),
      part('the submodules', invoke<Submodule[]>('submodule_list'), [] as Submodule[]),
      // Answered with an empty map, without running anything, while the
      // setting is off — so this costs one round trip rather than a gpg run
      // per commit for the repositories that never asked for it.
      part(
        'the signatures',
        invoke<Record<string, SignatureMark>>('signature_marks', { limit }),
        {} as Record<string, SignatureMark>
      ),
      part('the signing setup', invoke<SigningSetup>('signing_setup'), null),
      part('the LFS setup', invoke<LfsStatus>('lfs_status'), null),
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
    if (trunk) store.trunk = settle('trunk', trunk, store.trunk)
    if (status) store.status = settle('status', status, store.status)
    store.worktrees = settle('worktrees', worktrees ?? [], store.worktrees)
    store.submodules = settle('submodules', submodules ?? [], store.submodules)
    store.signatures = settle('signatures', signatures ?? {}, store.signatures)
    store.signing = settle('signing', signing, store.signing)
    store.lfs = settle('lfs', lfs, store.lfs)
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
      trunk: store.trunk,
      status: store.status,
      worktrees: store.worktrees,
      submodules: store.submodules,
      signatures: store.signatures,
      signing: store.signing,
      lfs: store.lfs,
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
    // A stash is a commit, and the graph draws it as a row of its own, so
    // choosing one has to open the stash rather than the commit view of it.
    // Decided here rather than at each call site: every way of selecting —
    // the graph, the sidebar, revealing one — goes through this.
    store.stashView = store.stashes.some((one) => one.oid === oid) ? oid : null
    if (store.stashView) store.viewer = null
    if (oid === WIP) {
      store.detail = null
      return
    }
    store.detail = await guard('Load commit', () =>
      invoke<CommitDetail>('commit_detail', { oid })
    )
  }

  /**
   * Loads far enough back to have a commit's row, when it is not on screen.
   *
   * The graph holds a page at a time. In a repository of a few hundred commits
   * every branch tip is in the first page and this never runs; in one of
   * seventeen thousand, spread over hundreds of branches, most tips are older
   * than the page and the row to scroll to does not exist yet — which is why
   * clicking such a branch looked like it did nothing at all.
   */
  async function loadUpTo(oid: string): Promise<'here' | 'missing' | 'far'> {
    if (store.rows.some((row) => row.oid === oid)) return 'here'
    const depth = await guard('Find commit', () =>
      invoke<number | null>('commit_depth', { oid })
    )
    // Nothing points at it, or this clone has never fetched it.
    if (depth === null || depth === undefined) return 'missing'
    const wanted = depth + 1
    if (wanted <= store.limit) return 'here'
    // So far back that a page holding it would be a download rather than a
    // scroll. The detail panel still answers the question the click asked.
    if (wanted > REVEAL_LIMIT) {
      note(
        `That commit is ${wanted.toLocaleString()} commits back — too far to draw. ` +
          'Its details are on the right.',
        'info'
      )
      return 'far'
    }
    // Whole pages, so the number in settings still means what it says.
    const page = pageSize()
    store.limit = Math.ceil(wanted / page) * page
    await refresh()
    return 'here'
  }

  /**
   * Selects a commit and asks the graph to bring it into view.
   *
   * What a single click on a branch does: pointing at a branch is a question
   * about where it is and what was last put on it, which is answered without
   * touching the working tree. Checking out stays on the double click.
   */
  async function revealCommit(oid: string) {
    // Ask for the rows before asking the graph to scroll to one of them: the
    // scroll is a one-shot, and a row that arrives after it has nothing to
    // move to it.
    const found = await loadUpTo(oid)
    // A commit this clone does not have is not worth asking the backend to
    // describe: the answer is an error, and the caller knows better than this
    // does what to say about it.
    if (found === 'missing') return false
    revealSeq += 1
    store.revealing = { oid, seq: revealSeq }
    await select(oid)
    return true
  }

  /**
   * Opens a stash: its diff in the detail panel, and the stash itself in the
   * content view, the way a commit gets both. `select` recognises it as a
   * stash and does the rest.
   */
  async function selectStash(index: number) {
    const oid = await guard('Read stash', () => invoke<string>('stash_oid', { index }))
    if (oid) await select(oid)
  }

  /** One commit's files and message, for a pane that owns its own reading. */
  const commitDetail = (oid: string) =>
    guard('Read commit', () => invoke<CommitDetail>('commit_detail', { oid }))

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
  /**
   * One commit's signature, asked for as it is selected.
   *
   * Unguarded and unrefreshed: this is a read whose commonest answer is "it
   * was not signed", and a repository where nothing is signed should not put
   * a line in the log every time a row is clicked.
   */
  /**
   * Who last touched each line. Unguarded: a file with no history yet is an
   * ordinary answer, said where the file would have been.
   */
  const blameFile = (path: string, commit?: string | null) =>
    invoke<BlameRun[]>('blame_file', { path, commit: commit ?? null })

  /** Every commit that touched a file, newest first, across renames. */
  const fileHistory = (path: string, limit = 200) =>
    guard('File history', () => invoke<FileCommit[]>('file_history', { path, limit }))

  const commitSignature = (oid: string) =>
    invoke<CommitSignature>('commit_signature', { oid }).catch(() => null)

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

  /**
   * Runs what was typed at the log's prompt, and writes back what git said.
   *
   * The command goes in first and its output under it, which is the order the
   * console reads them in. The output is not a notice — you typed the command
   * with the console open, and that is where the answer is. The refresh is the
   * same one every button gets, since `git commit` typed here changes the
   * window exactly as much as clicking it would.
   */
  async function typed(line: string) {
    const parsed = parseCommandLine(line)
    if ('error' in parsed) {
      note(parsed.error, 'error')
      return false
    }
    const shown = ['git', ...parsed.args.map(quoteArg)].join(' ')
    const out = await run<CmdOutput>(shown, 'run_git', { args: parsed.args })
    if (!out) return false
    note(shown, out.ok ? 'command' : 'failed')
    const text = [out.stdout, out.stderr].filter((s) => s.trim()).join('\n')
    note(text || (out.ok ? '' : `exit ${out.code}`), 'output')
    return out.ok
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
    typed,
    refresh,
    refreshStatus,
    loadMore,
    openRepo,
    select,
    revealCommit,
    selectStash,
    commitDetail,
    commitFileDiff,
    workingFileDiff,
    fileText,
    run,
    report,

    /**
     * Switches branch, and says so only when something happened worth saying.
     *
     * A plain switch answers with nothing; one that had to set the uncommitted
     * work down and pick it up on the other side answers with a sentence, and
     * that is worth a notice — the branch changed and the changes came along,
     * which is not obvious from a list that looks the same afterwards. So does
     * a checkout of a remote branch that pulled the local one up to date,
     * which is more than the click looks like it did.
     *
     * The one question a checkout can leave open — the branch and its remote
     * have both moved on, and settings say to ask — lands in the store for the
     * toolbar's strip, the same way a refused push does.
     */
    checkout: async (name: string) => {
      const outcome = await run<CheckoutOutcome>('Checkout', 'checkout', { name })
      if (outcome?.message.trim()) useToasts().info(outcome.message.trim())
      store.divergedCheckout = outcome?.diverged ?? null
      return outcome
    },
    dismissDiverged: () => {
      store.divergedCheckout = null
    },
    /**
     * Checks a branch out into a new folder, so it and the current one are
     * open side by side. `track` names the remote-tracking ref to create the
     * branch from when it does not exist here yet.
     *
     * What git says while doing it — "Preparing worktree", "HEAD is now at
     * …" — goes to the log and no further: the folder appears in the worktree
     * list, which is the answer to the click, and a notice quoting git over the
     * top of it is one more thing to dismiss.
     */
    worktreeAdd: (path: string, branch: string, track?: string) =>
      run<string>('Add worktree', 'worktree_add', { path, branch, track }),
    worktreeRemove: (path: string, force = false) =>
      run<string>('Remove worktree', 'worktree_remove', { path, force }),
    /**
     * Submodule work. Every one of these ends in a refresh, because a
     * submodule that has just been cloned or emptied changes the parent's
     * working tree as surely as a checkout does.
     */
    commitSignature,
    blameFile,
    fileHistory,
    /** Fetches the real contents of one LFS file, or of every one of them. */
    lfsPull: (path?: string) =>
      run<string>(path ? `Fetch ${path}` : 'Fetch LFS files', 'lfs_pull', {
        path: path ?? null
      }),
    submoduleUpdate: (path?: string, recursive = false) =>
      run<string>(
        path ? `Update ${path}` : 'Update submodules',
        'submodule_update',
        { path: path ?? null, recursive }
      ),
    submoduleSync: (path?: string) =>
      run<string>('Sync submodule URLs', 'submodule_sync', { path: path ?? null }),
    submoduleAdd: (url: string, path: string) =>
      run<string>('Add submodule', 'submodule_add', { url, path }),
    submoduleDeinit: (path: string, force = false) =>
      run<string>('Empty submodule', 'submodule_deinit', { path, force }),
    submoduleRemove: (path: string) =>
      run<string>('Remove submodule', 'submodule_remove', { path }),
    /**
     * Checks out the branch a review was opened from, whatever it takes.
     *
     * A review from a fork has no branch in any remote this clone knows, so
     * `checkout` by name fails; the backend adds the fork as a remote, fetches
     * the one branch and tracks it.
     */
    checkoutReview: (review: {
      number: number
      branch: string
      head_sha: string
      source: {
        owner: string
        ssh_url: string
        https_url: string
        is_fork: boolean
      } | null
    }) => run<string>('Check out review', 'checkout_review', { review }),
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
    /** Names the branch this repository is organised around; null forgets it. */
    setTrunk: (name: string | null) => run<string>('Main branch', 'set_trunk_branch', { name }),
    renameBranch: (from: string, to: string) =>
      run<string>('Rename branch', 'rename_branch', { from, to }),
    setUpstream: (branch: string, upstream: string) =>
      run<string>('Set upstream', 'set_upstream', { branch, upstream }),
    unsetUpstream: (branch: string) => run<string>('Unset upstream', 'unset_upstream', { branch }),
    addToGitignore: (pattern: string) =>
      run<string>('Ignore', 'add_to_gitignore', { pattern }),
    commitPatch: (oid: string) => guard('Read patch', () => invoke<string>('commit_patch', { oid })),
    /**
     * Applies one hunk, or only the lines picked out of it.
     *
     * `lines` left out means the whole hunk, which is what the buttons did
     * before there was any picking and what they still say when nothing is
     * picked.
     */
    applyHunk: (
      path: string,
      hunkIndex: number,
      action: 'stage' | 'unstage' | 'discard',
      lines?: { added: number[]; removed: number[] }
    ) => {
      const some = lines ? `${lines.added.length + lines.removed.length} lines` : 'hunk'
      const verb =
        action === 'stage' ? 'Stage' : action === 'unstage' ? 'Unstage' : 'Discard'
      return run<string>(`${verb} ${some}`, 'apply_hunk', {
        path,
        hunkIndex,
        action,
        lines: lines ?? null
      })
    },
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
    /** Deletes files git is not tracking, which is what discarding one means. */
    deleteUntracked: (paths: string[]) =>
      run<string>('Delete', 'delete_untracked', { paths }),
    commit: (message: string, amend = false) =>
      run<string>('Commit', 'commit', { message, amend }),
    amendDraft: () => guard('Read HEAD', () => invoke<AmendDraft>('amend_draft')),
    rewordCheck: (oid: string) =>
      guard('Read commit', () => invoke<RewordCheck>('reword_check', { oid })),
    /** Gives a commit a new message. Answers with the id it now has. */
    reword: (oid: string, message: string) => run<string>('Reword', 'reword', { oid, message }),
    stashPush: (message?: string) => run<string>('Stash', 'stash_push', { message }),
    stashPop: (index: number) => run<string>('Stash pop', 'stash_pop', { index }),
    stashApply: (index: number) => run<string>('Stash apply', 'stash_apply', { index }),
    /**
     * Several at once, oldest first. `dropAfter` makes it a pop.
     *
     * Unlike the single ones this hands the outcome back rather than only a
     * sentence: the caller has to be able to say which went on and which one
     * stopped the run.
     */
    stashApplyMany: (indexes: number[], dropAfter = false) =>
      run<StashRun>(
        `${dropAfter ? 'Pop' : 'Apply'} ${indexes.length} stashes`,
        'stash_apply_many',
        { indexes, dropAfter }
      ),
    stashDrop: (index: number) => run<string>('Stash drop', 'stash_drop', { index }),
    /** Gives a stash a new description, leaving it where it is in the list. */
    stashRename: (index: number, message: string) =>
      run<string>('Rename stash', 'stash_rename', { index, message }),
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
      run<string>('Resolve', 'conflict_resolve_as_is', { path }),
    /** Takes one side in every conflicted file at once. */
    conflictResolveAll: (side: 'ours' | 'theirs') =>
      run<string>('Resolve every file', 'conflict_resolve_all', { side }),
    /** Stages every conflicted file as it stands, markers permitting. */
    conflictStageAll: () => run<string>('Stage every file', 'conflict_stage_all'),
    /** Which conflicted files still have git's markers in them. */
    conflictMarked: () => guard('Read conflicts', () => invoke<string[]>('conflict_marked'))
  }
}

/**
 * The colours the graph's lines are drawn in, in the order they are handed out.
 *
 * The whole job of a lane colour is that two lines running side by side are
 * told apart at a glance, so every entry has to be an unmistakably different
 * hue rather than a lighter version of one already in the list. Index 0 is the
 * trunk's, and the backend holds it back from every other line: its `PALETTE`
 * counts these, so the two lists have to stay the same length.
 *
 * They live in the stylesheet — `--lane-1` to `--lane-10` — because a lane on
 * white has to be darker than the same lane on black to read as the same line,
 * and only the theme knows which it is. Read once when the theme changes rather
 * than per row: the graph asks for a colour thousands of times a frame.
 */
const FALLBACK_LANES = [
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

const lanes = ref<string[]>(FALLBACK_LANES)

/** Reads the lane colours out of the stylesheet. Called when the theme lands. */
export function refreshLanes() {
  if (typeof window === 'undefined') return
  const style = getComputedStyle(document.documentElement)
  const found = FALLBACK_LANES.map((_, at) =>
    style.getPropertyValue(`--lane-${at + 1}`).trim()
  )
  // A stylesheet that has not arrived yet answers with empty strings, and a
  // graph drawn in no colour at all is worse than one drawn in the old ones.
  if (found.every((one) => one.length > 0)) lanes.value = found
}

/** The modulo keeps the index in range, which the checker cannot work out. */
export const laneColor = (index: number): string =>
  lanes.value[index % lanes.value.length] ?? FALLBACK_LANES[0]!

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
