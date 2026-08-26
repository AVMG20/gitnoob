import { computed, reactive } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { Label, Member, Review, ReviewDetail } from './useForge'
import { useForge } from './useForge'
import { parsePatch } from './usePatch'
import { foldThreads, threadKey } from './reviewThreads'
import type { DiffHunk } from './useGit'

/** Somebody who spoke or was named: the shape both forges flatten to. */
export interface Person {
  login: string
  name: string
  avatar: string | null
}

/**
 * One remark under a review.
 *
 * `issue` comments belong to the conversation; `diff` ones are anchored to a
 * line of a file. The forge-specific shapes — GitHub's two comment tables,
 * GitLab's one notes list with positions — arrive here already flattened.
 */
export interface RComment {
  id: number
  author: Person
  body: string
  created_at: string
  updated_at: string
  kind: 'issue' | 'diff'
  path: string | null
  line: number | null
  /** `new` unless the thread is anchored to something that has since gone. */
  side: 'old' | 'new' | null
  reply_to: number | null
  /** What the forge wants back to settle this thread; empty when it cannot. */
  thread: string
  resolvable: boolean
  resolved: boolean
  /** Whether the lines it was written against have since moved on. */
  outdated: boolean
}

/** A root remark and everything said in answer to it. */
export interface Thread {
  key: string
  id: number
  root: RComment
  replies: RComment[]
}

/** One check a forge ran against the review's head. */
export interface ReviewCheck {
  name: string
  state: 'success' | 'failure' | 'pending' | 'cancelled' | 'skipped'
  description: string
  url: string
}

/** What one person has said about the review as a whole. */
export interface ReviewVerdict {
  author: Person
  state: 'approved' | 'changes_requested' | 'commented' | 'dismissed'
  submitted_at: string
  body: string
}

/** Whether the review can land, and what stands in the way of it. */
export interface ReviewStatus {
  checks: ReviewCheck[]
  checks_state: 'success' | 'failure' | 'pending' | 'skipped' | 'none'
  verdicts: ReviewVerdict[]
  approvals: number
  approvals_required: number
  mergeable: boolean | null
  merge_status: string | null
  conflicts: boolean
}

export type FileStatus = 'added' | 'deleted' | 'modified' | 'renamed'

/** One file a review touches across its commits, with its unified patch. */
export interface RFile {
  path: string
  old_path: string | null
  status: FileStatus
  additions: number
  deletions: number
  binary: boolean
  patch: string
}

/** A file as the reading side wants it: the patch already in hunks. */
export interface RFileWithDiff extends RFile {
  hunks: DiffHunk[]
}

export interface RCommit {
  sha: string
  message: string
  author: string
  created_at: string
}

/** Where a diff-anchored draft sits, so it can be opened and sent by place. */
export interface Draft {
  path: string
  line: number
  side: 'old' | 'new'
}

/**
 * A remark written on a line and held back until the verdict goes with it.
 *
 * This is what a review is on GitHub — remarks pending under one review, sent
 * when it is submitted — and what a reader wants everywhere: reading a
 * hundred files should not fire a notification per line.
 */
export interface Pending extends Draft {
  body: string
}

const store = reactive({
  /** The review this page is open on; null when no page is open at all. */
  current: null as Review | null,
  detail: null as ReviewDetail | null,
  loadingDetail: false,
  detailError: null as string | null,

  comments: [] as RComment[],
  loadingComments: false,
  commentsError: null as string | null,

  files: [] as RFileWithDiff[],
  loadingFiles: false,

  commits: [] as RCommit[],
  loadingCommits: false,

  /** How the review stands: what ran, who said what, whether it can land. */
  status: null as ReviewStatus | null,
  loadingStatus: false,

  /** Which pane of the review page is showing. */
  tab: 'conversation' as 'conversation' | 'files' | 'commits' | 'checks',
  /** The file selected in the rail, which the files pane scrolls to. */
  selectedPath: null as string | null,
  /** Files ticked read along the way. */
  viewed: new Set<string>(),

  /** An inline comment being written on a diff line, when one is. */
  draft: null as Draft | null,
  /** Remarks written on lines and waiting for the verdict to carry them. */
  pending: [] as Pending[],
  /** Replying straight into an existing thread, keyed by the thread's root. */
  replyingTo: null as number | null,

  /**
   * What has been typed but not sent: the conversation composer, and each
   * line a remark was started on. Kept for a while after the page closes, so
   * leaving by accident — or meaning to come back — costs nothing.
   */
  drafts: {
    talk: '',
    lines: {} as Record<string, string>
  },

  sending: false,
  /** Whatever verdict/merge/state action is in flight, named for the buttons. */
  acting: null as string | null
})

// --- drafts kept for a while
//
// Remarks are written in passing and sent when they are done; anything between
// the two belongs on disk rather than in luck. Kept per review for a week —
// held-back remarks are finished work, and a review started on a Friday is
// still a review on Monday.

const DRAFTS_KEY = 'gitnoob.review-drafts'
/** A week: long enough that a review left over a weekend is still there. */
const DRAFT_TTL = 7 * 24 * 60 * 60 * 1000

interface KeptDrafts {
  at: number
  talk: string
  lines: Record<string, string>
  /** Remarks finished but held back for the verdict that will carry them. */
  pending?: Pending[]
}

function draftsId(review: Review): string {
  const status = useForge().store.status
  const where = status?.slug ? `${status.kind}@${status.host}/${status.slug.owner}/${status.slug.name}` : 'unknown'
  return `${where}#${review.number}`
}

/** Reads what was kept for this review, when it was kept recently enough. */
function recallDrafts(review: Review) {
  store.drafts = { talk: '', lines: {} }
  store.pending = []
  try {
    const all = JSON.parse(localStorage.getItem(DRAFTS_KEY) ?? '{}') as Record<string, KeptDrafts>
    const kept = all[draftsId(review)]
    if (kept && Date.now() - kept.at < DRAFT_TTL) {
      store.drafts = { talk: kept.talk ?? '', lines: kept.lines ?? {} }
      store.pending = kept.pending ?? []
    }
  } catch {
    // Nothing kept is not a problem; nothing readable is neither.
  }
}

/** Puts the drafts away, dropping the ones already sent. */
function keepDrafts() {
  const current = store.current
  try {
    const all = JSON.parse(localStorage.getItem(DRAFTS_KEY) ?? '{}') as Record<string, KeptDrafts>
    const id = current ? draftsId(current) : null
    // Sweeps the expired on the way through, so the store never grows past
    // the reviews somebody is actually circling.
    const fresh: Record<string, KeptDrafts> = {}
    for (const [key, kept] of Object.entries(all)) {
      if (Date.now() - kept.at < DRAFT_TTL) fresh[key] = kept
    }
    if (
      id &&
      (store.drafts.talk.trim() || Object.keys(store.drafts.lines).length || store.pending.length)
    ) {
      fresh[id] = {
        at: Date.now(),
        talk: store.drafts.talk,
        lines: store.drafts.lines,
        pending: store.pending
      }
    } else if (id) {
      delete fresh[id]
    }
    localStorage.setItem(DRAFTS_KEY, JSON.stringify(fresh))
  } catch {
    // A window that cannot remember still shows the review this session.
  }
}

/** Where a line's half-written remark is kept. */
function lineDraftKey(path: string, side: string, line: number): string {
  return `${path}\u0000${side}\u0000${line}`
}

/** Everything above the pure store, rebuilt per call like the other composables. */
export function useReview() {
  const forge = useForge()

  /** The folded feed, rebuilt whenever a remark arrives or leaves. */
  const folded = computed(() => foldThreads(store.comments))

  function threadsAt(path: string, side: 'old' | 'new', line: number): Thread[] {
    return folded.value.byLine.get(threadKey(path, side, line)) ?? []
  }

  function talkThreads(): Thread[] {
    return folded.value.talk
  }

  /**
   * Where one remark lives on the forge.
   *
   * Neither forge sends the address of a comment with it, but both build it
   * the same way every time: the review's own page, and an anchor naming the
   * remark. Worked out here so copying a link to one costs no request.
   */
  function commentUrl(comment: RComment): string {
    const url = store.current?.url ?? store.detail?.url ?? ''
    if (!url) return ''
    if (forge.store.status?.kind === 'gitlab') return `${url}#note_${comment.id}`
    return comment.kind === 'diff'
      ? `${url}#discussion_r${comment.id}`
      : `${url}#issuecomment-${comment.id}`
  }

  /** How many remarks a file carries, for the rail and the headers. */
  function countFor(path: string): number {
    let count = 0
    for (const [key, threads] of folded.value.byLine) {
      if (!key.startsWith(`${path}\u0000`)) continue
      count += threads.reduce((sum, thread) => sum + 1 + thread.replies.length, 0)
    }
    return count
  }

  /**
   * Whether an answer that has just arrived is still the one being waited on.
   *
   * Four reads go out per review and none of them is fast; clicking through
   * three reviews in a second is how a page ends up showing one review's files
   * under another's title. A late answer to a page nobody is on any more is
   * dropped rather than drawn.
   */
  function stillOpen(number: number) {
    return store.current?.number === number
  }

  async function loadAll(number: number) {
    store.loadingDetail = true
    store.loadingComments = true
    store.loadingFiles = true
    store.loadingCommits = true
    store.detailError = null
    store.commentsError = null

    void forge.loadReviewDetail(number).finally(() => {
      if (!stillOpen(number)) return
      store.detail = forge.store.details[number] ?? null
      store.loadingDetail = false
      if (forge.store.detailError) store.detailError = forge.store.detailError
    })

    void invoke<RComment[]>('forge_review_comments', { number })
      .then((comments) => stillOpen(number) && (store.comments = comments))
      .catch((error) => stillOpen(number) && (store.commentsError = String(error)))
      .finally(() => stillOpen(number) && (store.loadingComments = false))

    void invoke<RFile[]>('forge_review_files', { number })
      .then(
        (files) =>
          stillOpen(number) &&
          (store.files = files.map((file) => ({
            ...file,
            hunks: file.binary ? [] : parsePatch(file.patch).hunks
          })))
      )
      .catch(() => stillOpen(number) && (store.files = []))
      .finally(() => stillOpen(number) && (store.loadingFiles = false))

    void invoke<RCommit[]>('forge_review_commits', { number })
      .then((commits) => stillOpen(number) && (store.commits = commits))
      .catch(() => stillOpen(number) && (store.commits = []))
      .finally(() => stillOpen(number) && (store.loadingCommits = false))

    void loadStatus(number)
  }

  /**
   * How the review stands: checks, verdicts, whether it can land.
   *
   * Several requests on the other side, and the half of a review that changes
   * while it is being read — so it is asked for apart from the description and
   * again whenever the reader does something that could have moved it.
   */
  async function loadStatus(number: number) {
    store.loadingStatus = true
    try {
      const standing = await invoke<ReviewStatus>('forge_review_status', { number })
      if (stillOpen(number)) store.status = standing
    } catch {
      // A forge that will not answer leaves the merge box saying what it
      // knows — the branches, the conversation — rather than nothing at all.
      if (stillOpen(number)) store.status = null
    } finally {
      if (stillOpen(number)) store.loadingStatus = false
    }
  }

  /** Opens the page on one review and starts every read it needs. */
  function show(review: Review) {
    store.current = review
    store.detail = null
    store.comments = []
    store.files = []
    store.commits = []
    store.status = null
    store.viewed = new Set()
    store.tab = 'conversation'
    store.selectedPath = null
    store.draft = null
    store.pending = []
    store.replyingTo = null
    recallDrafts(review)
    void loadAll(review.number)
  }

  /** Back to the graph. */
  function close() {
    store.current = null
    store.draft = null
    store.replyingTo = null
  }

  /** Reloads what time changes: the conversation, the detail, the standing. */
  async function refreshConversation() {
    const number = store.current?.number
    if (!number) return
    await forge.loadReviewDetail(number, true)
    store.detail = forge.store.details[number] ?? null
    void loadStatus(number)
    try {
      const comments = await invoke<RComment[]>('forge_review_comments', { number })
      store.comments = comments
    } catch (error) {
      store.commentsError = String(error)
    }
  }

  /**
   * Settles a thread, or opens it again.
   *
   * Answered on screen first and asked of the forge after: a tick that waits
   * for a round trip reads as a click that did nothing, and a refusal puts it
   * back where it was.
   */
  async function resolveThread(thread: Thread, resolved: boolean): Promise<boolean> {
    const number = store.current?.number
    const id = thread.root.thread
    if (!number || !id) return false
    const touched = [thread.root, ...thread.replies]
    for (const comment of touched) comment.resolved = resolved
    try {
      await invoke('forge_resolve_thread', { number, thread: id, resolved })
      return true
    } catch (error) {
      for (const comment of touched) comment.resolved = !resolved
      forge.store.error = String(error)
      return false
    }
  }

  /** Hands the review to somebody, or asks somebody to look at it. */
  async function setPeople(assignees: Member[], reviewers: Member[]): Promise<boolean> {
    const number = store.current?.number
    if (!number) return false
    store.acting = 'people'
    try {
      await invoke('forge_set_review_people', { number, assignees, reviewers })
      await refreshConversation()
      return true
    } catch (error) {
      forge.store.error = String(error)
      return false
    } finally {
      store.acting = null
    }
  }

  /** Sets the review's labels to exactly these. */
  async function setLabels(names: string[]): Promise<boolean> {
    const number = store.current?.number
    if (!number) return false
    store.acting = 'labels'
    try {
      await invoke('forge_set_labels', { number, labels: names })
      await refreshConversation()
      return true
    } catch (error) {
      forge.store.error = String(error)
      return false
    } finally {
      store.acting = null
    }
  }

  /** Every label the project has, asked for when a picker opens. */
  async function projectLabels(): Promise<Label[]> {
    try {
      const known = await invoke<Label[]>('forge_project_labels')
      // A forge that answers with something other than a list is a forge with
      // no labels as far as the picker is concerned.
      return Array.isArray(known) ? known : []
    } catch {
      return []
    }
  }

  /** Rewrites the title and description. */
  async function updateReview(title: string, body: string): Promise<boolean> {
    const number = store.current?.number
    if (!number || !title.trim()) return false
    store.acting = 'edit'
    try {
      await invoke('forge_update_review', { number, title: title.trim(), body })
      await refreshConversation()
      if (store.current) store.current.title = title.trim()
      return true
    } catch (error) {
      forge.store.error = String(error)
      return false
    } finally {
      store.acting = null
    }
  }

  /** Marks the review ready to be read, or puts it back to a draft. */
  async function setDraft(draft: boolean): Promise<boolean> {
    const number = store.current?.number
    if (!number) return false
    store.acting = draft ? 'draft' : 'ready'
    try {
      await invoke('forge_set_draft', { number, draft })
      await Promise.all([forge.loadReviews(), refreshConversation()])
      return true
    } catch (error) {
      forge.store.error = String(error)
      return false
    } finally {
      store.acting = null
    }
  }

  function beginDraft(path: string, line: number, side: 'old' | 'new') {
    store.draft = { path, line, side }
    store.tab = 'files'
    store.selectedPath = path
  }

  function cancelDraft() {
    // A cancelled remark is a remark taken back, not one to be kept: its text
    // goes so it is not handed back the next time the page opens.
    if (store.draft) delete store.drafts.lines[lineDraftKey(store.draft.path, store.draft.side, store.draft.line)]
    keepDrafts()
    store.draft = null
  }

  /**
   * Holds the line draft back for the verdict instead of sending it.
   *
   * The remark is finished — it just is not anybody else's business until the
   * reading is. Kept on disk with the half-written ones, so closing the page
   * mid-review does not throw the review away.
   */
  function queueDraft(body: string): boolean {
    const where = store.draft
    if (!where || !body.trim()) return false
    // One remark per line: writing again on a line already spoken for replaces
    // what was there, which is what editing a pending remark means.
    store.pending = [
      ...store.pending.filter(
        (one) => !(one.path === where.path && one.line === where.line && one.side === where.side)
      ),
      { ...where, body }
    ]
    delete store.drafts.lines[lineDraftKey(where.path, where.side, where.line)]
    store.draft = null
    keepDrafts()
    return true
  }

  /**
   * Posts one held-back remark as its own thread, and takes it off the queue.
   *
   * The GitLab half of finishing a review: it has no pending review to submit,
   * so each remark is sent as the thread it will become and dropped from the
   * queue the moment it lands — nothing is ever sent twice.
   */
  async function sendPending(one: Pending): Promise<boolean> {
    const current = store.current
    if (!current) return false
    const shas = store.detail
    try {
      await invoke('forge_add_diff_comment', {
        number: current.number,
        headSha: shas?.head_sha || current.head_sha || '',
        baseSha: shas?.base_sha || '',
        startSha: shas?.start_sha || '',
        path: one.path,
        line: one.line,
        side: one.side,
        body: one.body
      })
      dropPending(one)
      return true
    } catch (error) {
      forge.store.error = String(error)
      return false
    }
  }

  /** Takes a held-back remark off the list again. */
  function dropPending(one: Pending) {
    store.pending = store.pending.filter(
      (kept) =>
        !(kept.path === one.path && kept.line === one.line && kept.side === one.side)
    )
    keepDrafts()
  }

  /** What is waiting on one line, when anything is. */
  function pendingAt(path: string, side: 'old' | 'new', line: number): Pending | null {
    return (
      store.pending.find((one) => one.path === path && one.side === side && one.line === line) ??
      null
    )
  }

  /** How many held-back remarks a file carries, for the list's badges. */
  function pendingFor(path: string): number {
    return store.pending.filter((one) => one.path === path).length
  }

  /** Sends the line draft, then clears it whatever happened. */
  async function sendDraft(body: string): Promise<boolean> {
    const current = store.current
    const where = store.draft
    if (!current || !where || !body.trim()) return false
    const shas = store.detail
    store.sending = true
    try {
      await invoke('forge_add_diff_comment', {
        number: current.number,
        headSha: shas?.head_sha || current.head_sha || '',
        baseSha: shas?.base_sha || '',
        startSha: shas?.start_sha || '',
        path: where.path,
        line: where.line,
        side: where.side,
        body
      })
      delete store.drafts.lines[lineDraftKey(where.path, where.side, where.line)]
      await refreshConversation()
      return true
    } catch (error) {
      forge.store.error = String(error)
      return false
    } finally {
      store.sending = false
      store.draft = null
    }
  }

  async function post(body: string): Promise<boolean> {
    const current = store.current
    if (!current || !body.trim()) return false
    store.sending = true
    try {
      await invoke('forge_post_comment', { number: current.number, body })
      store.drafts.talk = ''
      await refreshConversation()
      return true
    } catch (error) {
      forge.store.error = String(error)
      return false
    } finally {
      store.sending = false
    }
  }

  async function reply(parentId: number, body: string): Promise<boolean> {
    const current = store.current
    if (!current || !body.trim()) return false
    store.sending = true
    try {
      await invoke('forge_reply_comment', { number: current.number, parentId, body })
      await refreshConversation()
      return true
    } catch (error) {
      forge.store.error = String(error)
      return false
    } finally {
      store.sending = false
      store.replyingTo = null
    }
  }

  /**
   * Approve, request changes or plain comment-with-verdict.
   *
   * Answers with whether it landed; the summary that went with it is spent
   * either way, having been sent rather than kept.
   */
  async function verdict(
    event: 'approve' | 'request_changes' | 'comment',
    body: string
  ): Promise<boolean> {
    const current = store.current
    if (!current) return false
    store.acting = event
    try {
      // GitHub takes the held-back remarks in the same request as the verdict,
      // which is one review and one notification. GitLab has no such request,
      // so they go one at a time and each leaves the queue as it lands: a
      // failure halfway through then leaves exactly what did not go, rather
      // than a queue that would post the first half twice on the next try.
      const held = store.pending.map((one) => ({
        path: one.path,
        line: one.line,
        side: one.side,
        body: one.body
      }))
      if (forge.store.status?.kind === 'gitlab') {
        for (const one of store.pending.slice()) {
          const sent = await sendPending(one)
          if (!sent) return false
        }
      }
      await invoke('forge_submit_review', {
        number: current.number,
        event,
        body,
        comments: forge.store.status?.kind === 'gitlab' ? [] : held
      })
      store.pending = []
      keepDrafts()
      await refreshConversation()
      return true
    } catch (error) {
      forge.store.error = String(error)
      return false
    } finally {
      store.acting = null
    }
  }

  async function merge(squash = false, deleteBranch = false) {
    const current = store.current
    if (!current) return
    store.acting = 'merge'
    try {
      store.detail && (store.detail.state = 'merged')
      const note = await invoke<string>('forge_merge_review', {
        number: current.number,
        squash,
        deleteBranch
      })
      forge.store.error = null
      await Promise.all([
        forge.loadReviewDetail(current.number, true),
        forge.loadReviews(),
        refreshConversation()
      ])
      store.detail = forge.store.details[current.number] ?? store.detail
      return note
    } catch (error) {
      forge.store.error = String(error)
      store.detail && store.detail.state !== 'merged' && (store.detail.state = 'open')
      return null
    } finally {
      store.acting = null
    }
  }

  async function setState(action: 'close' | 'reopen') {
    const current = store.current
    if (!current) return
    store.acting = action
    try {
      await invoke('forge_set_review_state', { number: current.number, action })
      await Promise.all([forge.loadReviews(), refreshConversation()])
    } catch (error) {
      forge.store.error = String(error)
    } finally {
      store.acting = null
    }
  }

  function toggleViewed(path: string) {
    const next = new Set(store.viewed)
    if (next.has(path)) next.delete(path)
    else next.add(path)
    store.viewed = next
  }

  /** How much of the review has been read, for the toolbar's progress. */
  const viewedCount = computed(() => store.files.filter((f) => store.viewed.has(f.path)).length)

  /** Every thread standing on a line, whatever file it belongs to. */
  const diffThreads = computed(() => [...folded.value.byLine.values()].flat())

  /** How many diff threads are still open, which is what is left to answer. */
  const openThreads = computed(
    () => diffThreads.value.filter((thread) => !thread.root.resolved).length
  )

  const resolvedThreads = computed(
    () => diffThreads.value.filter((thread) => thread.root.resolved).length
  )

  /** How many open threads a file carries, for the list's badges. */
  function openFor(path: string): number {
    let count = 0
    for (const [key, threads] of folded.value.byLine) {
      if (!key.startsWith(`${path}\u0000`)) continue
      count += threads.filter((thread) => !thread.root.resolved).length
    }
    return count
  }

  /** The verdict the signed-in account has standing, when it has one. */
  const myVerdict = computed(() => {
    const me = forge.store.me?.login
    if (!me) return null
    return store.status?.verdicts.find((verdict) => verdict.author.login === me) ?? null
  })

  /**
   * Whether the forge would take the merge right now.
   *
   * `null` means it has not said — a check still running, a mergeability it
   * has not worked out — and the button says so rather than guessing.
   */
  const canMerge = computed(() => {
    const state = store.detail?.state ?? store.current?.state ?? ''
    if (state === 'merged' || state === 'closed') return false
    if (store.detail?.draft) return false
    return store.status?.mergeable !== false
  })

  /** Puts the drafts away now rather than waiting for the next send. */
  function saveDrafts() {
    keepDrafts()
  }

  return {
    store,
    threadsAt,
    talkThreads,
    commentUrl,
    countFor,
    openFor,
    diffThreads,
    openThreads,
    resolvedThreads,
    myVerdict,
    canMerge,
    viewedCount,
    lineDraftKey,
    saveDrafts,
    show,
    close,
    beginDraft,
    cancelDraft,
    sendDraft,
    queueDraft,
    dropPending,
    pendingAt,
    pendingFor,
    post,
    reply,
    verdict,
    merge,
    setState,
    setPeople,
    setLabels,
    projectLabels,
    updateReview,
    setDraft,
    resolveThread,
    loadStatus,
    toggleViewed,
    refreshConversation
  }
}
