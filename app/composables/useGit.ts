import { aimAt, aimedAt, invoke } from './useInvoke'
import { markRaw, reactive, ref } from 'vue'
import { useConfig } from './useConfig'
import { useForge } from './useForge'
import { stepFile, useFileView, walkOrder } from './useFileView'
import { useHome } from './useHome'
import { parseCommandLine } from './cli'
import type { LfsStatus } from './useLfs'
import { useToasts } from './useToasts'
import { commitCount, failedRemotes, protectedPushWarning, trunkLocalName } from './useSync'
import { afterPush } from './useForgeNudges'
import { moveDraft } from './useCommitDraft'

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
  /** The upstream it was set to follow, when that was deleted on the remote
      and pruned here. `upstream` is null then. */
  gone?: string | null
  /** Where a push lands, `remote/name`: the upstream, except for a branch
      that follows another branch's name, which is pushed under its own. */
  push_to?: string | null
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
  /**
   * A switch put the work back and it did not fit: conflicts, nothing running,
   * and the auto-stash it made still on the list — so the switch can be undone.
   */
  restoring: boolean
  /** The stash a conflicted apply came from, while it can still be undone. */
  applied_stash: string | null
  /** The message git already wrote for it, e.g. "Merge branch 'x' into 'y'". */
  prepared: string | null
  /**
   * Set when this merge or rebase was started for another branch: the branch
   * you were on, which finishing or aborting it takes you back to.
   */
  returns_to: string | null
  /** A cherry-pick or revert of several commits that is part-way through. */
  sequencer?: 'cherry-pick' | 'revert' | null
  /** How many commits that run still has to apply, the stopped one included. */
  sequence_left?: number
  /** Names what is under way, the same for as long as it lasts. */
  operation?: string | null
}

export interface StatusEntry {
  path: string
  kind: string
  /** Where a moved file came from. Null for everything that stayed put. */
  from?: string | null
  /** Lines added and removed on this side. Zero for a binary file. */
  additions?: number
  deletions?: number
}
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
  /** Not history, drawn broken: a stash's line, or the link from a squash or
      a rebase back to the branch it folded in. */
  dashed: boolean
  /** On the upstream and not here yet: what a pull would bring. Drawn held
      back, so it reads as above where you stand rather than as part of it. */
  faint: boolean
  /** The line you are on, from your commit down. Drawn a shade brighter. */
  current: boolean
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
  /** On an upstream and on no local branch: not pulled yet. */
  unpulled: boolean
  /** Branch tips whose work this commit carries without being a merge of
      them — a squash, or the last commit of a rebase. */
  carries: string[]
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
  /** The name the file had before it was moved, when the diff is a move. */
  from?: string | null
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
  /** Where the upstream stood when this was read; a force push leases on it. */
  upstream_oid: string | null
  new_upstream: boolean
  ahead: number
  behind: number
  force_needed: boolean
  will_orphan: CommitSummary[]
  will_push: CommitSummary[]
  /** Every commit a force push would drop has a copy in the branch: it was
      rebased, squashed or amended here. */
  rewritten?: boolean
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
  /** What the remote has that the branch does not is only older copies of
      its own commits, so force pushing is the answer to lead with. */
  rewritten?: boolean
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

/** One commit a squash would fold into the others. */
export interface Folded {
  oid: string
  short: string
  summary: string
  /** The whole message, which is what the joined default is built from. */
  message: string
  author: string
  time: number
  /** Already on a remote, so folding it means a force push afterwards. */
  pushed: boolean
}

/** What the squash dialog shows before the fold is asked for. */
export interface SquashPreview {
  /** Oldest first — the order the messages are joined in. */
  commits: Folded[]
  /** The joined message, for the box to start from. */
  message: string
  /** The commit the fold lands on, short, or null at the root. */
  onto: string | null
  /** How many commits sit above the run and get replayed onto the fold. */
  above: number
  branch: string | null
  /** Why it cannot be done, or null when it can. */
  refusal: string | null
}

/** How two branches stand to each other. */
export interface BranchRelation {
  /** Commits the source has that the target does not. */
  ahead: number
  /** Commits the target has that the source does not. */
  behind: number
  /** Whether merging would go through on its own, worked out in memory.
      Null when it could not be asked (git older than 2.38). */
  clean?: boolean | null
  /** The files that merge would stop on. */
  conflicts?: string[]
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
  /** How the work reached the trunk: merged, squash or rebase. */
  landed?: 'merged' | 'squash' | 'rebase' | null
  /** The upstream it followed, when that has been deleted on the remote. */
  gone?: string | null
}

/** A local branch that looks finished, and why. */
export interface Tidy {
  name: string
  oid: string
  reason: 'merged' | 'squash' | 'rebase' | 'gone'
  /** The trunk ref the work was found on. */
  into: string | null
  gone: string | null
  /** Deleting it loses nothing. */
  proven: boolean
}

/** A deleted branch, with what it takes to put it back. */
export interface Removed {
  name: string
  oid: string
  remote: string | null
  merge: string | null
}

export interface Removal {
  removed: Removed[]
  refused: string[]
  /** The undo step that puts the whole batch back. */
  step: number | null
}

/** A branch to delete, with the commit it pointed at when it was offered. */
export interface Doomed {
  name: string
  oid: string
}

/** The local trunk, moved up to its upstream after a fetch. */
export interface TrunkMove {
  branch: string
  upstream: string
  commits: number
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

/** How a commit went, with what a hook did when one stopped it. */
export interface CommitAttempt {
  ok: boolean
  /** Git's answer, or everything git and the hooks printed. */
  output: string
  hook: {
    hooks: string[]
    /** Staged files the hook rewrote; staging them again picks up just that. */
    changed: string[]
    /** Rewritten files that also had unstaged edits; left alone. */
    mixed: string[]
  } | null
}

/** One thing in the staged diff worth a second look. */
export interface GuardFinding {
  path: string
  line: number | null
  kind: 'conflict' | 'secret' | 'file' | 'debug'
  what: string
  excerpt: string | null
}
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
  /** Whether somebody named it, rather than git or this app. */
  named?: boolean
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
  mode: 'soft' | 'hard' | 'stash' | 'apply' | 'pop' | 'drop' | 'files' | 'branch'
  destructive: boolean
  at: number
  /** Why this step cannot be taken as the repository stands; absent when it can. */
  unavailable?: string | null
}

/** One place HEAD or the current branch has been, from the reflog. */
export interface Moment {
  oid: string
  short: string
  previous: string | null
  time: number
  /** What git wrote down about the move: "commit: Tidy", "reset: moving to …". */
  action: string
  subject: string
  /** "HEAD", the branch's name, or both. */
  logs: string[]
}

/** A stash commit that is no longer on the list. */
export interface LostStash {
  oid: string
  short: string
  message: string
  time: number
}

/** `keep` moves the branch and keeps uncommitted work, or refuses. */
export type ResetMode = 'soft' | 'mixed' | 'hard' | 'keep'

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
      /** The sides only look different: the same lines, or spacing alone. */
      trivial?: 'same' | 'whitespace' | null
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
  /** rerere wrote last time's answer into the file; it waits to be accepted. */
  rerere?: boolean
  /** A lockfile: take a side and regenerate rather than merge lines. */
  lockfile?: boolean
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
  /**
   * The branches the graph is soloed on, by full ref name (`refs/heads/x`,
   * `refs/remotes/origin/x`). Empty draws every branch.
   */
  solo: [] as string[],
  limit: COMMIT_PAGE,
  /** Either `WIP` or a commit id. The working tree is selected by default. */
  selected: WIP as string,
  detail: null as CommitDetail | null,
  stashes: [] as StashEntry[],
  /** The name of a stash just put back, for the commit box to start from. */
  carried: null as string | null,
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
  log: [] as LogLine[],
  /** Local branches that look finished, as of the last fetch. */
  tidy: [] as Tidy[],
  /** A push waiting on the user's say-so, and why it is being asked. */
  pushConfirm: null as { branch: string; setUpstream: boolean; reason: string } | null,
  /** A push held back because the branch's upstream was deleted on the
      remote, so the toolbar can ask whether it was finished or meant to go
      back up. `gone` is the deleted upstream, `remote/name`. */
  pushGone: null as { branch: string; gone: string } | null
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

/**
 * Each repository's solo, by path, for as long as the window is open.
 *
 * Kept when you switch tabs and come back, because you were in the middle of
 * reading that branch. Not kept past a restart: a graph that opens with most of
 * its branches missing is a graph that looks broken until you spot why.
 */
const solos = new Map<string, string[]>()

/** The full ref name a solo is kept under. */
export const localRef = (name: string) => `refs/heads/${name}`
export const remoteRef = (remote: string, name: string) => `refs/remotes/${remote}/${name}`

/** A soloed ref as the sidebar writes it: `serp`, `origin/serp`. */
export function soloName(full: string) {
  return full.replace(/^refs\/(heads|remotes)\//, '')
}

/** Whether a soloed ref is still one of the branches the sidebar lists. */
function refExists(refs: RefTree, full: string) {
  return full.startsWith('refs/heads/')
    ? refs.locals.some((branch) => localRef(branch.name) === full)
    : refs.remotes.some((branch) => remoteRef(branch.remote, branch.name) === full)
}

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
  store.carried = null
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
 * How loudly to report a merge-like outcome. Stopping on conflicts is not a
 * failure when it was done on another branch's behalf: you are left on that
 * branch with the resolver open, which is the answer rather than an error.
 */
function level(outcome: MergeOutcome): LogLevel {
  return outcome.ok || outcome.conflicts.length ? 'info' : 'error'
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
  store.tidy = []
  store.pushConfirm = null
  store.pushGone = null
  store.resolving = null
  store.revealing = null
  store.viewer = null
  store.inside = []
  store.query = ''
  store.solo = []
}

/**
 * Whether git is part-way through something with its own way out.
 *
 * A merge, a rebase, a cherry-pick or a revert is aborted as a whole, and
 * while one is running "throw the conflict away" is both the wrong tool and a
 * lie: mid-rebase the committed side is the branch being rebased *onto*, so
 * taking a file back to it keeps somebody else's version, not yours.
 */
export function isRunning(progress: InProgress | null | undefined) {
  return (
    !!progress &&
    (progress.merging || progress.rebasing || progress.cherry_picking || progress.reverting)
  )
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
  /** `replace` names the tab this repository takes the place of. */
  async function openRepo(path: string, record = true, replace?: string) {
    const info = await guard('Open repository', () =>
      invoke<RepoInfo>('open_repo', { path, record, replace: replace ?? null })
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
    store.tidy = []
    store.pushConfirm = null
    store.pushGone = null
    store.viewer = null
    store.resolving = null
    store.query = ''
    store.solo = solos.get(info.path) ?? []
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
    const solo = store.solo
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
        invoke<{ rows: GraphRow[]; has_more: boolean }>('commit_graph', { limit, solo }),
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
    // A solo changed while this was out describes the graph before it, and
    // the refresh the change started is already on its way with the new one.
    if (page && store.solo === solo) {
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

    pruneSolo()

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

  /**
   * Draws only these branches in the graph, or every branch again for none.
   *
   * Only the picture changes. Nothing is fetched, nothing about the branches
   * moves, and the sidebar still lists them all.
   */
  async function soloOn(names: string[]) {
    if (!store.repo) return
    const path = store.repo.path
    store.solo = [...new Set(names)]
    if (store.solo.length) solos.set(path, store.solo)
    else solos.delete(path)
    await refresh()
  }

  /**
   * Lets go of soloed branches that are not there any more.
   *
   * Deleted here, or pruned by a fetch: the backend skips a name it cannot
   * find, so the graph is right either way, but a banner still saying it is
   * soloing a branch that no longer exists is not.
   */
  function pruneSolo() {
    const refs = store.refs
    if (!refs || !store.solo.length) return
    const live = store.solo.filter((full) => refExists(refs, full))
    if (live.length !== store.solo.length) void soloOn(live)
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
      invoke<number | null>('commit_depth', { oid, solo: store.solo })
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
   * Opens a stash, which is opening the commit it is: the graph draws it as a
   * row and the panel beside it lists what it holds, exactly as for any other.
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

  /**
   * Puts stashes back, and hands the commit box the name one of them was given.
   *
   * A stash pushed from the commit box is named after the summary that was in
   * it, and somebody who named a stash well has usually written the commit's
   * message already. Read before the run, since a pop takes the stash off the
   * list. Only a name somebody chose: git's own "WIP on" names repeat the last
   * commit's subject, and more than one named stash gives no one message.
   */
  async function carrying<T>(indexes: number[], put: () => Promise<T | null>) {
    const names = store.stashes
      .filter((one) => indexes.includes(one.index) && one.named && one.message.trim())
      .map((one) => one.message.trim())
    const result = await put()
    const stopped = (result as StashRun | null)?.stopped
    if (result && !stopped && names.length === 1) store.carried = names[0] ?? null
    return result
  }

  /**
   * Runs something that only moves files between the index and the working
   * tree, and re-reads only what that can change.
   *
   * Staging and unstaging never touch a ref, so a full refresh — thirteen
   * reads and a walk of the history — was the wrong answer to every click on
   * a file. The status, and what git is part-way through (staging the last
   * conflicted file can end a merge's conflicts), are the whole of it.
   */
  async function runStatus<T>(label: string, command: string, args: Record<string, unknown> = {}) {
    const result = await guard(label, () => invoke<T>(command, args))
    await refreshWorking()
    useHome().stale()
    return result
  }

  /** The status and what git is part-way through, and nothing else. */
  async function refreshWorking() {
    if (!store.repo) return
    const path = store.repo.path
    const [, progress] = await Promise.all([
      refreshStatus(),
      part('what git is doing', invoke<InProgress>('in_progress'), store.progress)
    ])
    if (store.repo?.path !== path) return
    const key = JSON.stringify(progress) ?? 'undefined'
    const snapshot = snapshots.get(path)
    if (snapshot) {
      if (snapshot.seen.progress === key) return
      snapshot.seen.progress = key
      snapshot.progress = raw(progress)
      store.progress = snapshot.progress
      return
    }
    if (JSON.stringify(store.progress) !== key) store.progress = raw(progress)
  }

  async function run<T>(label: string, command: string, args: Record<string, unknown> = {}) {
    const result = await guard(label, () => invoke<T>(command, args))
    // Refresh whether or not it worked. A git command that fails can still have
    // changed the repository — a branch switch whose stash pop conflicted has
    // switched, a rebase that stopped has moved HEAD — and leaving the window
    // showing the state from before is worse than an extra read.
    await refresh()
    // The home tab counts commits and uncommitted files across every project,
    // and this is the moment those stopped being true. It re-reads when it is
    // next opened rather than now: nothing of it is on screen.
    useHome().stale()
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

  /**
   * Stages files, and moves the viewer on when the open one is what left.
   *
   * Reviewing a change is reading a file, staging it, reading the next one.
   * The file that was just staged is no longer where the viewer is pointing,
   * so it either sat on an empty page or had to be closed by hand before the
   * next one could be opened. Now it walks to the next unstaged file the way
   * the arrow keys would — down first, up when that was the last one, and shut
   * when there is nothing left to read.
   *
   * Only for a single file, and only when it is the one on screen: staging a
   * folder, or a file other than the one being read, is not a step through a
   * review and must not move the page under it.
   */
  async function stageFiles(paths: string[]) {
    const open = openWorkingFile('unstaged')
    const taken = !!open && paths.includes(open.path)
    // The walk is for the one-file gesture. Staging a folder or the whole list
    // is not stepping through a review, so the file on screen simply follows
    // itself across rather than jumping somewhere new.
    const next = taken && paths.length === 1 ? nextToRead(open.path) : null
    const said = await runStatus<string>('Stage', 'stage', { paths })
    // A refusal leaves the file where it was, so the viewer stays on it.
    if (said !== null && taken) {
      store.viewer =
        paths.length === 1 ? (next ? { path: next, side: 'unstaged' } : null) : sideOf(open.path)
    }
    return said
  }

  /**
   * Stages the lot; the file on screen follows itself to the staged side.
   *
   * `expectStaged: false` is the commit button's "stage everything first":
   * the list said nothing was staged, and the backend refuses if something
   * is by now rather than sweep it into the commit unseen.
   */
  async function stageEverything(expectStaged?: boolean) {
    const open = openWorkingFile('unstaged')
    const said = await runStatus<string>(
      'Stage all',
      'stage_all',
      expectStaged === undefined ? {} : { expectStaged }
    )
    if (said !== null && open) store.viewer = sideOf(open.path)
    return said
  }

  /** The working file on screen when it is on `side`, or null. */
  function openWorkingFile(side: 'staged' | 'unstaged') {
    const open = store.viewer
    if (!open || open.commit) return null
    return (open.side ?? 'unstaged') === side ? open : null
  }

  /**
   * Where a path still has something to show, staged side first.
   *
   * A file that has moved between the lists is the same file, and reading it
   * should carry on; one that has nothing left on either side has no page.
   */
  function sideOf(path: string) {
    if ((store.status?.staged ?? []).some((entry) => entry.path === path)) {
      return { path, side: 'staged' as const }
    }
    if ((store.status?.unstaged ?? []).some((entry) => entry.path === path)) {
      return { path, side: 'unstaged' as const }
    }
    return null
  }

  /**
   * The unstaged file to read after `path`, or null when it was the last one.
   *
   * Walked in the order the panel draws rather than the order git lists: a
   * folded-away directory is not somewhere to land, and in tree mode "the next
   * one down" is the next one down on screen. Conflicted files are left out —
   * clicking one opens the resolver, not the viewer, so landing on one would
   * take over the window rather than show the next change.
   */
  function nextToRead(path: string): string | null {
    const conflicted = store.status?.conflicted ?? []
    const files = (store.status?.unstaged ?? []).filter(
      (entry) => !conflicted.includes(entry.path)
    )
    const { state: how } = useFileView()
    const order = walkOrder([{ files, side: 'unstaged' }], how.mode, how.collapsed)
    const from = { path, side: 'unstaged' as const }
    const next = stepFile(order, from, 1) ?? stepFile(order, from, -1)
    return next?.path ?? null
  }

  /**
   * Unstages files, and follows the open one to the side it moved to.
   *
   * A file is unstaged to carry on working on it, not to stop reading it. The
   * viewer was pointed at the staged copy, which no longer exists — so it sat
   * on a page with nothing on it while the file itself was one list down.
   */
  async function unstageFiles(paths: string[]) {
    const open = openWorkingFile('staged')
    const taken = !!open && paths.includes(open.path)
    const said = await runStatus<string>('Unstage', 'unstage', { paths })
    // Only where there is really something left: a staged change the work tree
    // already matches leaves nothing on either side.
    if (said !== null && taken) store.viewer = sideOf(open.path)
    return said
  }

  /**
   * Throws changes away, and shuts the viewer when it was reading one of them.
   *
   * What is discarded has nothing left to show — the file matches HEAD, or is
   * gone from the disk altogether.
   */
  async function throwAway(label: string, command: string, paths: string[]) {
    const open = store.viewer
    const reading = !!open && !open.commit && paths.includes(open.path)
    const said = await run<string>(label, command, { paths })
    if (said !== null && reading) store.viewer = null
    return said
  }

  /** Reports a `git` CLI run in the log, whichever way it went. */
  function report(label: string, out: CmdOutput | null) {
    if (!out) return false
    const text = [out.stdout, out.stderr].filter((s) => s.trim()).join('\n')
    note(`${label}: ${text || (out.ok ? 'done' : `exit ${out.code}`)}`, out.ok ? 'info' : 'error')
    return out.ok
  }

  // --- keeping in step with the remote -------------------------------------

  /**
   * The remote-tracking tips worth watching across a fetch: what the branch
   * you are on follows, and the trunk's copy on its remote.
   */
  function watched(): Record<string, string> {
    const locals = store.refs?.locals ?? []
    const names = new Set<string>()
    const head = locals.find((one) => one.is_head)
    if (head?.upstream) names.add(head.upstream)
    const trunk = store.trunk.name
    if (trunk) {
      const local = locals.find((one) => one.name === trunk)
      if (local?.upstream) names.add(local.upstream)
      else if (!local && trunk.includes('/')) names.add(trunk)
    }
    const tips: Record<string, string> = {}
    for (const name of names) {
      const oid = store.refs?.remotes.find((one) => `${one.remote}/${one.name}` === name)?.oid
      if (oid) tips[name] = oid
    }
    return tips
  }

  /**
   * What every fetch is followed by, once it has worked: the local trunk moved
   * up to its remote when that is a plain fast-forward nobody is standing on,
   * and the list of branches that look finished read again.
   *
   * `path` is the repository the fetch was for. Each step checks it is still
   * the one open: the backend holds one repository at a time, and a tab
   * switched mid-fetch must not have the next call land on the other one.
   */
  async function afterFetch(path: string) {
    // Not while something else is under way: moving a ref in the middle of
    // somebody's merge is not housekeeping.
    if (!store.busy && store.repo?.path === path) {
      const moved = await invoke<TrunkMove | null>('keep_trunk_current').catch((error) => {
        note(`Could not bring the main branch up to date: ${String(error)}`, 'failed')
        return null
      })
      if (moved) {
        note(`${moved.branch} brought up to date with ${moved.upstream} (${commitCount(moved.commits)})`)
      }
    }
    if (store.repo?.path !== path) return
    await findTidy(path)
  }

  /** Reads the tidy list quietly, when the setting allows it. */
  async function findTidy(path = store.repo?.path) {
    if (!path || store.repo?.path !== path) return
    if (useConfig().settings.value?.suggest_tidy === false) {
      store.tidy = []
      return
    }
    const found = await invoke<Tidy[]>('tidy_candidates').catch(() => null)
    if (found && store.repo?.path === path) store.tidy = found
  }

  /**
   * Says when the fetch brought commits for the branch you are on or for the
   * trunk, with the button that brings them in.
   */
  async function announceMoves(path: string, before: Record<string, string>) {
    if (useConfig().settings.value?.announce_remote_moves === false) return
    const locals = store.refs?.locals ?? []
    const head = locals.find((one) => one.is_head) ?? null
    const trunk = trunkLocalName(store.trunk, locals)
    for (const [name, old] of Object.entries(before)) {
      const now = store.refs?.remotes.find((one) => `${one.remote}/${one.name}` === name)?.oid
      if (!now || now === old) continue
      const relation = await invoke<BranchRelation>('branch_relation', {
        source: now,
        target: old
      }).catch(() => null)
      if (store.repo?.path !== path) return
      const count = relation?.ahead ?? 0
      if (!count) continue
      const toasts = useToasts()
      if (head && head.upstream === name) {
        if (head.behind > 0) {
          toasts.offer(`${name} has ${commitCount(count)} you do not have yet`, [
            { label: 'Pull', run: () => pullHere() }
          ])
        }
      } else if (head && trunk && head.name !== trunk) {
        toasts.offer(`${name} moved on by ${commitCount(count)}`, [
          { label: `Update ${head.name}`, run: () => updateFromTrunk(head.name) }
        ])
      } else {
        toasts.info(`${name} moved on by ${commitCount(count)}`)
      }
    }
  }

  /** The pull the toolbar's button does, for a notice to offer. */
  async function pullHere() {
    const outcome = await guard('Pull', () => invoke<MergeOutcome>('pull', {}))
    if (outcome) note(`Pull: ${outcome.message}`, outcome.ok ? 'info' : 'error')
    if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0] ?? null
    await refresh()
    return outcome
  }

  /**
   * The fetch on a timer, and the one when a project opens.
   *
   * Not counted as busy, so nothing on screen locks for the length of a
   * network round trip; and only the repository's own remote, so a fork
   * added for a review and deleted since cannot fail it. A failure is written
   * in the log and nowhere else — nobody asked for this one, and a notice
   * every ten minutes about a remote that is down is a notice to ignore.
   */
  async function fetchQuietly() {
    const path = store.repo?.path
    if (!path) return false
    const before = watched()
    const out = await invoke<CmdOutput>('fetch_primary').catch((error) => {
      note(`Background fetch: ${String(error)}`, 'failed')
      return null
    })
    if (store.repo?.path !== path) return false
    if (!out?.ok) {
      if (out) note(`Background fetch: ${out.stderr.trim() || `exit ${out.code}`}`, 'failed')
      return false
    }
    await afterFetch(path)
    if (store.repo?.path !== path) return true
    // While something else is running its own refresh is on the way, and
    // the watcher has seen the refs move; reading now would only race it.
    if (!store.busy) await refresh()
    if (store.repo?.path !== path) return true
    await announceMoves(path, before)
    const forge = useForge()
    if (forge.usable.value) void forge.loadReviews()
    return true
  }

  /**
   * A fetch the user asked for failed. When a remote it names is one this
   * app added to look at somebody's review, offer to take it off: that is
   * what a fork deleted after its pull request was merged looks like.
   */
  async function offerToDropDeadRemotes(out: CmdOutput, asked?: string) {
    const named = failedRemotes(`${out.stdout}\n${out.stderr}`, asked)
    if (!named.length) return
    const ours = await invoke<string[]>('review_remotes').catch(() => [] as string[])
    for (const remote of named.filter((name) => ours.includes(name))) {
      useToasts().offer(
        `${remote} did not answer. It was added to check out a review.`,
        [
          {
            label: `Remove ${remote}`,
            run: () => run<string>('Remove remote', 'remote_remove', { name: remote })
          }
        ]
      )
    }
  }

  /** Reads whether a refused push only lost to older copies of its own commits. */
  async function markRewritten(branch: string) {
    const preview = await invoke<PushPreview>('push_preview', { branch }).catch(() => null)
    if (preview?.rewritten && store.pushBlocked?.branch === branch) {
      store.pushBlocked = { ...store.pushBlocked, rewritten: true }
    }
  }

  /**
   * Fetches, then rebases `branch` onto the trunk as its remote has it or
   * merges that in, whichever the setting says. Conflicts open the resolver.
   */
  async function updateFromTrunk(branch: string, how?: 'rebase' | 'merge') {
    const outcome = await guard('Update from main', () =>
      invoke<MergeOutcome>('update_from_trunk', { branch, how: how ?? null })
    )
    if (outcome) {
      note(outcome.message, level(outcome))
      if (outcome.ok) useToasts().info(outcome.message.split('\n')[0] ?? outcome.message)
      if (outcome.conflicts.length) store.resolving = outcome.conflicts[0] ?? null
    }
    await refresh()
    return outcome
  }

  /**
   * Deletes local branches, and offers them back.
   *
   * Each goes with the commit it pointed at when it was offered: one that has
   * moved since holds work the list never showed, and the backend leaves it
   * alone. The whole batch is one step in the undo history, and the notice's
   * Undo takes that very step, so ⌘Z and the notice cannot disagree about
   * what is left to undo.
   */
  async function deleteBranches(branches: Doomed[]) {
    const removal = await guard('Delete branches', () =>
      invoke<Removal>('delete_branches', { branches })
    )
    if (removal) {
      for (const refused of removal.refused) note(`Not deleted: ${refused}`, 'error')
      const gone = removal.removed
      if (gone.length) {
        const title =
          gone.length === 1 ? `Deleted ${gone[0]!.name}` : `Deleted ${gone.length} branches`
        note(`${title}: ${gone.map((one) => `${one.name} (was ${one.oid.slice(0, 7)})`).join(', ')}`)
        const step = removal.step
        useToasts().offer(
          title,
          step == null ? [] : [{ label: 'Undo', run: () => undoDeletion(step) }]
        )
      }
    }
    await refresh()
    if (removal?.removed.length) {
      const taken = new Set(removal.removed.map((one) => one.name))
      store.tidy = store.tidy.filter((one) => !taken.has(one.name))
    }
    return removal
  }

  /** Undoes the history step a deletion recorded, from its notice. */
  async function undoDeletion(step: number) {
    const said = await run<string>('Undo', 'undo_step', { id: step })
    if (said) {
      note(said)
      useToasts().info(said.split('\n')[0] ?? said)
    }
    await findTidy()
    return said
  }

  /**
   * Pushes a branch, unless it is one worth a second look first — then the
   * toolbar asks, and `confirmPush` (or the gone-upstream strip) is the
   * answer. Null when it asked rather than pushed.
   *
   * Every push the window offers comes through here: the toolbar, commit and
   * push, a home row, the review dialog. Those used to call `pushBranch`
   * straight, and skipped both questions — so committing and pushing a branch
   * whose pull request had been merged quietly put the deleted remote branch
   * back, and a push to main from the commit box never asked.
   *
   * `recreate` is the gone-upstream strip's own "push it back": that question
   * has been answered, the protected one has not.
   */
  async function requestPush(branch: string, setUpstream: boolean, recreate = false) {
    // The repository this was asked in. The protected check can wait on the
    // forge, and a push sent after a tab switch would go to the other tab.
    const repo = aimedAt()
    const local = store.refs?.locals.find((one) => one.name === branch)
    if (!recreate && local?.gone && !local.upstream) {
      store.pushGone = { branch, gone: local.gone }
      return null
    }
    const reason = await protectedPushWarning(
      branch,
      store.trunk,
      store.refs?.locals ?? [],
      !!useConfig().settings.value?.warn_push_to_trunk,
      (name) => useForge().isProtected(name)
    )
    if (aimedAt() !== repo) {
      note(`Did not push ${branch}: another repository was opened while it was being checked.`)
      return null
    }
    if (reason) {
      store.pushConfirm = { branch, setUpstream, reason }
      return null
    }
    return pushBranch(branch, setUpstream)
  }

  async function pushBranch(branch: string, setUpstream: boolean) {
    // A branch that already tracks something goes back where it came from,
    // whichever remote that is — and so does one whose upstream was deleted,
    // which a push now puts back under the name it had. Only a new branch
    // has to be told, and the answer is the repository's own remote — never
    // a fork added to look at somebody's review.
    const local = store.refs?.locals.find((b) => b.name === branch)
    const upstream = local?.upstream ?? null
    const tracked = (upstream ?? local?.gone ?? null)?.split('/')[0] ?? null
    const primary = tracked ? null : await invoke<string | null>('primary_remote').catch(() => null)
    const target = tracked ?? primary ?? 'origin'
    const out = await guard('Push', () =>
      invoke<CmdOutput>('push', {
        remoteName: target,
        branch,
        force: false,
        setUpstream
      })
    )
    const ok = report('Push', out)
    afterPush(branch, setUpstream, ok)
    store.pushBlocked =
      out && isRejectedForBeingBehind(out)
        ? {
            remote: target,
            branch,
            upstream,
            message: `${out.stdout}\n${out.stderr}`.trim()
          }
        : null
    if (store.pushBlocked) await markRewritten(branch)
    await refresh()
    return ok
  }

  return {
    forget,
    store,
    note,
    typed,
    refresh,
    refreshStatus,
    loadMore,
    soloOn,
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
    renameBranch: async (from: string, to: string) => {
      const repo = store.repo?.path
      const said = await run<string>('Rename branch', 'rename_branch', { from, to })
      // The message being written for it goes along with the name.
      if (said !== null) moveDraft(repo, from, to)
      return said
    },
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
      return runStatus<string>(`${verb} ${some}`, 'apply_hunk', {
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

    stage: stageFiles,
    stageAll: stageEverything,
    unstage: unstageFiles,
    discard: (paths: string[]) => throwAway('Discard', 'discard', paths),
    /** Deletes files git is not tracking, which is what discarding one means. */
    deleteUntracked: (paths: string[]) => throwAway('Delete', 'delete_untracked', paths),
    commit: (message: string, amend = false) =>
      run<string>('Commit', 'commit', { message, amend }),
    /**
     * Commits, and says whether a hook stopped it and what the hook changed,
     * rather than only that it failed. `noVerify` skips the hooks, and is only
     * ever passed from a button that says so.
     */
    commitAttempt: (message: string, amend = false, noVerify = false) =>
      run<CommitAttempt>('Commit', 'commit_attempt', { message, amend, noVerify }),
    /** What in the staged diff is worth a second look before committing. */
    commitGuard: (debug = false) =>
      guard('Check what is staged', () => invoke<GuardFinding[]>('commit_guard', { debug })),
    /** The unstaged file to read after this one; see `stageFiles`. */
    nextToRead,
    /** Which side a path still has something to show on, staged first. */
    sideOf,
    amendDraft: () => guard('Read HEAD', () => invoke<AmendDraft>('amend_draft')),
    rewordCheck: (oid: string) =>
      guard('Read commit', () => invoke<RewordCheck>('reword_check', { oid })),
    /** Gives a commit a new message. Answers with the id it now has. */
    reword: (oid: string, message: string) => run<string>('Reword', 'reword', { oid, message }),
    stashPush: (message?: string) => run<string>('Stash', 'stash_push', { message }),
    stashPop: (index: number) =>
      carrying([index], () => run<string>('Stash pop', 'stash_pop', { index })),
    stashApply: (index: number) =>
      carrying([index], () => run<string>('Stash apply', 'stash_apply', { index })),
    /**
     * Several at once, oldest first. `dropAfter` makes it a pop.
     *
     * Unlike the single ones this hands the outcome back rather than only a
     * sentence: the caller has to be able to say which went on and which one
     * stopped the run.
     */
    stashApplyMany: (indexes: number[], dropAfter = false) =>
      carrying(indexes, () =>
        run<StashRun>(
          `${dropAfter ? 'Pop' : 'Apply'} ${indexes.length} stashes`,
          'stash_apply_many',
          { indexes, dropAfter }
        )
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
    /**
     * What folding these commits into one would do, and what would stop it.
     * Unguarded refusals — a selection that cannot be folded is an answer the
     * dialog shows, not an error.
     */
    squashPreview: (oids: string[]) =>
      guard('Read squash', () => invoke<SquashPreview>('squash_preview', { oids })),
    /** Folds a run of commits into one carrying `message`. */
    squash: (oids: string[], message: string) =>
      run<string>('Squash', 'squash', { oids, message }),
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
      const path = store.repo?.path
      const out = await guard('Fetch', () => invoke<CmdOutput>('fetch', { remote }))
      const ok = report('Fetch', out)
      if (out && !out.ok) void offerToDropDeadRemotes(out, remote)
      // Even a fetch that failed on one remote has usually brought the rest.
      if (out && path) await afterFetch(path)
      await refresh()
      // A fetch asks what moved on the other side, and on a profile signed in
      // to a forge the reviews are part of that answer: one opened, merged or
      // retitled elsewhere is exactly the news a fetch is for. Not awaited —
      // the refs are on screen already, and the forge is a network away.
      const forge = useForge()
      if (forge.usable.value) void forge.loadReviews()
      return ok
    },
    /**
     * Pulls the checked out branch. With `rebase` left out the user's own
     * `pull.rebase` decides, the way a typed `git pull` would.
     */
    pull: async (rebase?: boolean) => {
      const outcome = await guard('Pull', () => invoke<MergeOutcome>('pull', { rebase }))
      if (outcome) note(`Pull: ${outcome.message}`, outcome.ok ? 'info' : 'error')
      await refresh()
      return outcome
    },
    pushPreview: (branch?: string, fetchFirst = false) =>
      guard('Push preview', () =>
        invoke<PushPreview>('push_preview', { branch, fetchFirst })
      ),
    /**
     * `lease` is the upstream commit the force-push preview showed: the push
     * is allowed to replace that and nothing newer.
     */
    push: async (
      remoteName: string,
      branch: string,
      force: boolean,
      setUpstream: boolean,
      lease?: string | null
    ) => {
      const out = await guard('Push', () =>
        invoke<CmdOutput>('push', { remoteName, branch, force, setUpstream, lease })
      )
      const ok = report(force ? 'Force push' : 'Push', out)
      afterPush(branch, setUpstream, ok)
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
      if (store.pushBlocked) await markRewritten(branch)
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
    pullBranch: async (branch: string, rebase?: boolean) => {
      const outcome = await guard('Pull', () =>
        invoke<MergeOutcome>('pull_branch', { branch, rebase })
      )
      if (outcome) note(`Pull ${branch}: ${outcome.message}`, level(outcome))
      await refresh()
      return outcome
    },
    pushBranch,
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
        note(`Merge ${source} into ${target}: ${outcome.message}`, level(outcome))
      }
      await refresh()
      return outcome
    },
    rebaseBranch: async (branch: string, onto: string) => {
      const outcome = await guard('Rebase', () =>
        invoke<MergeOutcome>('rebase_branch', { branch, onto })
      )
      if (outcome) {
        note(`Rebase ${branch} onto ${onto}: ${outcome.message}`, level(outcome))
      }
      await refresh()
      return outcome
    },
    abortMerge: () => run<string>('Abort merge', 'abort_merge'),
    /** The way out of a conflicted auto-stash, which no abort can undo. */
    undoRestore: () => run<string>('Put it back', 'undo_restore'),

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
    /**
     * Ends conflicts by taking the files back to what the branch already had,
     * throwing away whatever the other side was bringing in.
     */
    conflictDiscard: (paths: string[]) =>
      run<string>('Throw away the conflicts', 'conflict_discard', { paths }),
    /** Which conflicted files still have git's markers in them. */
    conflictMarked: () => guard('Read conflicts', () => invoke<string[]>('conflict_marked')),

    /** Takes a step that can no longer be taken out of the undo history. */
    forgetStep: (id: number) => run<string>('Forget step', 'history_forget', { id }),
    /** Where HEAD and the current branch have been, newest first. Read only. */
    reflogMoments: (limit = 200) =>
      guard('Read the reflog', () => invoke<Moment[]>('reflog_moments', { limit })),
    /** Stashes that fell off the list. Runs `git fsck`, so only on request. */
    lostStashes: () =>
      guard('Look for dropped stashes', () => invoke<LostStash[]>('lost_stashes')),
    /** Puts a stash commit back on the stash list. */
    restoreStash: (oid: string) => run<string>('Restore stash', 'restore_stash', { oid }),
    /** Makes a branch at a commit without checking it out: always safe. */
    createBranchAt: (name: string, start: string) =>
      run<string>('Create branch', 'create_branch', { name, start, checkout: false }),
    fetchQuietly,
    findTidy,
    updateFromTrunk,
    deleteBranches,
    undoDeletion,
    requestPush,
    /** The answer to the toolbar's "push straight to main?" question. */
    confirmPush: async (go: boolean) => {
      const asked = store.pushConfirm
      store.pushConfirm = null
      if (go && asked) return pushBranch(asked.branch, asked.setUpstream)
      return null
    },
    /**
     * Fetches, then starts a branch from the trunk as its remote has it, or
     * from `from`, tracking nothing and carrying open changes along.
     */
    startWork: async (name: string, from?: string) => {
      const said = await run<string>('Start work', 'start_work', { name, from: from ?? null })
      if (said) useToasts().info(said.split('\n')[0] ?? said)
      return said
    },
    /** What new work starts from: the trunk as its remote has it. */
    trunkTarget: () => invoke<string | null>('trunk_target').catch(() => null)
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
const bright = ref<string[]>(FALLBACK_LANES.map((one) => lift(one, false)))

/** Reads the lane colours out of the stylesheet. Called when the theme lands. */
export function refreshLanes() {
  if (typeof window === 'undefined') return
  const root = document.documentElement
  const style = getComputedStyle(root)
  const found = FALLBACK_LANES.map((_, at) =>
    style.getPropertyValue(`--lane-${at + 1}`).trim()
  )
  // A stylesheet that has not arrived yet answers with empty strings, and a
  // graph drawn in no colour at all is worse than one drawn in the old ones.
  if (found.every((one) => one.length > 0)) lanes.value = found
  const light = 'light' in root.dataset
  bright.value = lanes.value.map((one) => lift(one, light))
}

/** The modulo keeps the index in range, which the checker cannot work out. */
export const laneColor = (index: number): string =>
  lanes.value[index % lanes.value.length] ?? FALLBACK_LANES[0]!

/**
 * A lane's colour brought forward a step, for the line you are standing on.
 *
 * Slightly: the point is that the eye can find the line among a dozen without
 * the rest of the picture looking switched off. Worked out once per theme
 * rather than per segment, because the graph asks thousands of times a frame.
 */
export const laneBright = (index: number): string =>
  bright.value[index % bright.value.length] ?? laneColor(index)

/**
 * The same hue, one step further from the background.
 *
 * On a dark ground that is towards white; on a light one, where towards white
 * is towards the page, it is a deeper and fuller version of the colour. Both
 * are what "brighter" means when you look at the line.
 */
function lift(hex: string, light: boolean): string {
  const value = parseInt(hex.slice(1), 16)
  if (Number.isNaN(value) || hex.length !== 7) return hex
  const channel = (shift: number) => (value >> shift) & 255
  const mix = (part: number, towards: number, amount: number) =>
    Math.round(part + (towards - part) * amount)
  const [r, g, b] = light
    ? [mix(channel(16), 0, 0.18), mix(channel(8), 0, 0.18), mix(channel(0), 0, 0.18)]
    : [mix(channel(16), 255, 0.22), mix(channel(8), 255, 0.22), mix(channel(0), 255, 0.22)]
  return `#${[r, g, b].map((part) => part!.toString(16).padStart(2, '0')).join('')}`
}

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
