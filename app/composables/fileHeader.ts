/**
 * What the bar above an open file shows: which badge, which buttons.
 *
 * One function decides it from three things — the file, what git says about
 * it, and the tab — so the bar is never a mix of two states, and every state
 * can be checked without drawing anything.
 *
 * - A tracked file with no changes is being read, not reviewed: no counts, no
 *   Stage or Discard, no Diff to switch to. It gets the browsing tools.
 * - A changed file gets Stage and Unstage as they really stand: all staged is
 *   Unstage only, partly staged is both.
 * - A new file gets Add and Delete; the tab and tree already mark it as new.
 * - An ignored file is "ignored", and git has nothing to do with it.
 * - A file the editor will not write to says "read-only" and why.
 * - A dependency (vendor/, node_modules/) says "vendor", and has no buttons.
 * - A commit's copy says which commit, and offers the file as it is now.
 * - A file gone from disk and git has the missing-file page's own buttons.
 */

export type HeaderKind = 'commit' | 'missing' | 'ignored' | 'untracked' | 'modified' | 'unchanged'

export type HeaderAction = 'discard' | 'stage' | 'unstage' | 'add' | 'delete' | 'open-current'

export interface HeaderBadge {
  text: string
  tone: 'new' | 'ignored' | 'readonly' | 'deleted' | 'conflict'
  title?: string
}

export interface HeaderFile {
  path: string
  /** Why the editor will not write to it, as `read_file` says: binary, lfs… */
  readonly?: string | null
  /** Git ignores it (only asked when the status does not list it). */
  ignored?: boolean
  /** Not on disk, not in the status: the missing-file page. */
  missing?: boolean
}

export interface HeaderStatusEntry {
  kind: string
}

export interface HeaderStatus {
  staged?: HeaderStatusEntry | null
  unstaged?: HeaderStatusEntry | null
  conflicted?: boolean
}

export interface HeaderTab {
  commit?: string | null
  /** The commit's subject, for the hash's tooltip. */
  subject?: string | null
}

export interface HeaderState {
  kind: HeaderKind
  badges: HeaderBadge[]
  /** The +/− tally. */
  counts: boolean
  /** Whether Diff and File are both there to switch between. */
  modes: boolean
  /** The git buttons, left to right. */
  actions: HeaderAction[]
  /** The commit's short hash and subject, on a commit tab. */
  commit: { short: string; subject: string } | null
}

/** Why a file is read-only, in a sentence, for the badge's tooltip. */
export const READONLY_REASONS: Record<string, string> = {
  binary: 'Binary file: nothing to edit.',
  lfs: 'Stored with Git LFS: this is the pointer, not the file.',
  encoding: 'Not UTF-8: shown read-only so a save cannot mangle it.',
  endings: 'Mixed line endings: shown read-only so a save cannot change lines you did not touch.',
  large: 'Over 2 MB: shown read-only.',
  locked: 'A committed copy: it cannot be changed.',
  dependency: 'A dependency, installed by Composer or npm: read-only, and git does not track it.'
}

export function headerState(file: HeaderFile, status: HeaderStatus, tab: HeaderTab): HeaderState {
  const readonly = file.readonly ?? null
  const readonlyBadge: HeaderBadge[] = readonly
    ? [{ text: 'read-only', tone: 'readonly', title: READONLY_REASONS[readonly] ?? 'Read-only.' }]
    : []

  if (tab.commit) {
    return {
      kind: 'commit',
      badges: [],
      counts: true,
      modes: true,
      actions: ['open-current'],
      commit: { short: tab.commit.slice(0, 7), subject: tab.subject ?? '' }
    }
  }

  const staged = status.staged ?? null
  const unstaged = status.unstaged ?? null

  if (file.missing && !staged && !unstaged) {
    return { kind: 'missing', badges: [], counts: false, modes: false, actions: [], commit: null }
  }

  // In vendor/ or node_modules/: read, never edited, and nothing for git.
  if (readonly === 'dependency' && !staged && !unstaged) {
    return {
      kind: 'ignored',
      badges: [{ text: 'vendor', tone: 'ignored', title: READONLY_REASONS.dependency }],
      counts: false,
      modes: false,
      actions: [],
      commit: null
    }
  }

  if (unstaged?.kind === 'untracked' && !staged) {
    return {
      kind: 'untracked',
      badges: [...readonlyBadge],
      counts: true,
      modes: true,
      actions: ['add', 'delete'],
      commit: null
    }
  }

  if (staged || unstaged || status.conflicted) {
    const actions: HeaderAction[] = []
    const local = !!unstaged || !!status.conflicted
    if (local) actions.push('discard', 'stage')
    if (staged) actions.push('unstage')
    const badges: HeaderBadge[] = []
    if (status.conflicted) badges.push({ text: 'conflicted', tone: 'conflict' })
    else if ((unstaged ?? staged)?.kind === 'deleted') {
      badges.push({ text: 'deleted', tone: 'deleted', title: 'Deleted: what is shown is the last copy git has' })
    }
    return {
      kind: 'modified',
      badges: [...badges, ...readonlyBadge],
      counts: true,
      modes: true,
      actions,
      commit: null
    }
  }

  if (file.ignored) {
    return {
      kind: 'ignored',
      badges: [{ text: 'ignored', tone: 'ignored', title: 'Git ignores this file' }, ...readonlyBadge],
      counts: false,
      modes: false,
      actions: [],
      commit: null
    }
  }

  return {
    kind: 'unchanged',
    badges: readonlyBadge,
    counts: false,
    modes: false,
    actions: [],
    commit: null
  }
}
