import { computed, reactive } from 'vue'

/**
 * The commit message being written, kept per repository and branch.
 *
 * It used to live in the commit box itself, and the box is taken down whenever
 * the panel shows a commit instead — so clicking any row in the graph, or
 * switching tab, threw away whatever had been typed, amend and all. It lives
 * here now, and in the browser's storage behind that, so it also survives a
 * restart. A draft belongs to a branch: switch to another and its own comes
 * back, switch back and this one does.
 *
 * Storage can be missing, full or refused, so every touch of it is wrapped and
 * the store works the same without it — it just forgets on restart.
 *
 * Only the message is kept across a restart, never the amend tick. A box that
 * came back ticked a week later held the message of a commit that had since
 * moved on, maybe been pushed, and one press amended whatever HEAD was by
 * then. An amend is a decision made about the commit in front of you, so it
 * lasts as long as the session; what is stored is what was typed before it.
 */
export interface CommitDraft {
  message: string
  amend: boolean
  /** What was typed before amend was ticked, so unticking gives it back. */
  stashed: string | null
  /** Whether the commit being amended is already on a remote. */
  pushed: boolean
  /** When it was last touched, for throwing away the ones nobody came back to. */
  at: number
}

const KEY = 'gitnoob.commit-drafts'
/** A month: long enough for a holiday, short enough not to pile up forever. */
const TTL = 30 * 24 * 60 * 60 * 1000

const drafts = reactive<Record<string, CommitDraft>>({})
let loaded = false

function blank(): CommitDraft {
  return { message: '', amend: false, stashed: null, pushed: false, at: Date.now() }
}

/** The key a draft is kept under. */
export function draftKey(repo: string | null | undefined, branch: string | null | undefined) {
  return `${repo ?? ''}\u0000${branch ?? ''}`
}

function load() {
  if (loaded) return
  loaded = true
  try {
    const kept = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Record<string, CommitDraft>
    const now = Date.now()
    for (const [key, draft] of Object.entries(kept)) {
      if (draft && typeof draft.message === 'string' && now - (draft.at ?? 0) < TTL) {
        // An older version stored the amend tick too: read without it.
        drafts[key] = { ...blank(), message: draft.message, at: draft.at ?? now }
      }
    }
  } catch {
    // Unreadable or unavailable: start from nothing.
  }
}

function save() {
  try {
    const kept: Record<string, Pick<CommitDraft, 'message' | 'at'>> = {}
    for (const [key, draft] of Object.entries(drafts)) {
      // While amending, the box holds HEAD's message; what was typed is the
      // draft worth keeping.
      const message = draft.amend ? (draft.stashed ?? '') : draft.message
      // An empty one is the same as none, and not worth a line of storage.
      if (message.trim()) kept[key] = { message, at: draft.at }
    }
    localStorage.setItem(KEY, JSON.stringify(kept))
  } catch {
    // Full or refused: the draft is still held for this session.
  }
}

/**
 * Moves a branch's draft along when the branch is renamed. It is the same
 * branch under a new name, and the message being written for it goes with it
 * rather than waiting under a name nothing is called any more. A draft the
 * new name already has is kept: that one was written on purpose.
 */
export function moveDraft(repo: string | null | undefined, from: string, to: string) {
  load()
  const was = draftKey(repo, from)
  const now = draftKey(repo, to)
  const found = drafts[was]
  if (!found || was === now) return
  delete drafts[was]
  if (!drafts[now]?.message.trim()) drafts[now] = { ...found, at: Date.now() }
  save()
}

/** Forgets what was loaded, so a test can start from storage again. */
export function resetDrafts() {
  for (const key of Object.keys(drafts)) delete drafts[key]
  loaded = false
}

/**
 * The draft for whichever repository and branch `where` names, read and
 * written through as if it were the box's own state.
 */
export function useCommitDraft(where: () => { repo: string | null; branch: string | null }) {
  load()
  const key = computed(() => {
    const at = where()
    return draftKey(at.repo, at.branch)
  })

  const current = computed<CommitDraft>(() => drafts[key.value] ?? blank())

  function update(patch: Partial<CommitDraft>) {
    updateAt(key.value, patch)
  }

  /**
   * Writes to the draft under `at`, whichever one is in front now. For an
   * answer that comes back after an await: the tab may have changed, and what
   * came back belongs to the draft it was asked for.
   */
  function updateAt(at: string, patch: Partial<CommitDraft>) {
    drafts[at] = { ...(drafts[at] ?? blank()), ...patch, at: Date.now() }
    save()
  }

  /** The draft under `at`, as it stands. */
  function read(at: string): CommitDraft {
    return drafts[at] ?? blank()
  }

  const message = computed({
    get: () => current.value.message,
    set: (value: string) => update({ message: value })
  })
  const amend = computed({
    get: () => current.value.amend,
    set: (value: boolean) => update({ amend: value })
  })
  const stashed = computed({
    get: () => current.value.stashed,
    set: (value: string | null) => update({ stashed: value })
  })
  const pushed = computed({
    get: () => current.value.pushed,
    set: (value: boolean) => update({ pushed: value })
  })

  /** After a commit went through: the message is in the history now. */
  function clear(at: string = key.value) {
    delete drafts[at]
    save()
  }

  return { key, message, amend, stashed, pushed, update, updateAt, read, clear }
}
