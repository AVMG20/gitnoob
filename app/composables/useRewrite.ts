import { reactive } from 'vue'
import { invoke } from './useInvoke'
import { useGit } from './useGit'
import { useRebase } from './useRebase'

/**
 * Everything that rewrites history from one commit in the graph, and the ways
 * on from a run of picks that stopped.
 *
 * Kept apart from the shared store so the graph menu, the toolbar strips and
 * the dialogs these open can all reach the same few calls and the one piece of
 * state they share: which dialog is up.
 */

/** What a rewrite from one commit would involve, read before asking. */
export interface RewriteCheck {
  oid: string
  short: string
  summary: string
  message: string
  pushed: boolean
  is_head: boolean
  above: number
  staged: number
  refusal: string | null
}

export interface AbsorbTarget {
  oid: string
  short: string
  summary: string
}

export interface AbsorbHunk {
  path: string
  header: string
  preview: string[]
  target: AbsorbTarget | null
  why: string | null
}

export interface AbsorbPlan {
  hunks: AbsorbHunk[]
  kept_files: string[]
  fingerprint: string
  refusal: string | null
}

export interface SequenceOutcome {
  finished: boolean
  message: string
  conflicts: string[]
}

export interface SplitOutcome {
  finished: boolean
  left: string[]
  message: string
}

/** Which dialog is open, and for which commit. */
export type RewriteDialog =
  | { kind: 'fixup'; oid: string }
  | { kind: 'reword'; oid: string }
  | { kind: 'split'; oid: string }
  | { kind: 'absorb' }

const store = reactive({
  dialog: null as RewriteDialog | null,
  /** Files of a commit being split that are still not committed. */
  splitLeft: [] as string[]
})

export function useRewrite() {
  const git = useGit()
  const rebase = useRebase()

  /** Reads the rebase strip again, so a stop shows up straight away. */
  async function settle(said: string | null) {
    if (said) git.note(said)
    await rebase.readProgress()
    return said
  }

  async function sequence(command: string, label: string) {
    const outcome = await git.run<SequenceOutcome>(label, command)
    if (outcome) git.note(outcome.message, outcome.conflicts.length ? 'error' : 'info')
    return outcome
  }

  async function splitContinue(rest: 'ask' | 'commit' | 'leave' = 'ask') {
    const outcome = await git.run<SplitOutcome>('Split', 'split_continue', { rest })
    store.splitLeft = outcome && !outcome.finished ? outcome.left : []
    if (outcome && (outcome.finished || !outcome.left.length)) git.note(outcome.message)
    await rebase.readProgress()
    return outcome
  }

  return {
    store,
    open: (dialog: RewriteDialog) => (store.dialog = dialog),
    close: () => (store.dialog = null),

    check: (oid: string) =>
      invoke<RewriteCheck>('rewrite_check', { oid }).catch((error) => {
        git.note(`Read commit: ${String(error)}`, 'error')
        return null
      }),
    fixup: async (oid: string, fold: boolean, allowPushed: boolean) =>
      settle(await git.run<string>('Fix up', 'fixup_commit', { oid, fold, allowPushed })),
    reword: async (oid: string, message: string, allowPushed: boolean) =>
      settle(await git.run<string>('Reword', 'reword_commit', { oid, message, allowPushed })),
    splitStart: async (oid: string, allowPushed: boolean) =>
      settle(await git.run<string>('Split', 'split_start', { oid, allowPushed })),
    splitContinue,
    absorbPlan: () =>
      invoke<AbsorbPlan>('absorb_plan').catch((error) => {
        git.note(`Absorb: ${String(error)}`, 'error')
        return null
      }),
    absorb: async (fingerprint: string, fold: boolean) =>
      settle(await git.run<string>('Absorb', 'absorb', { fingerprint, fold })),

    sequenceContinue: () => sequence('sequence_continue', 'Continue'),
    sequenceSkip: () => sequence('sequence_skip', 'Skip this commit'),
    sequenceAbort: () => sequence('sequence_abort', 'Abort'),

    resolvedFiles: () =>
      invoke<string[] | null>('conflict_resolved')
        .then((list) => (Array.isArray(list) ? list : []))
        .catch(() => [] as string[]),
    reopen: (path: string) => git.run<string>('Reopen', 'conflict_reopen', { path }),

    rerereEnabled: () =>
      invoke<boolean | null>('rerere_enabled')
        .then((on) => on !== false)
        .catch(() => true),
    setRerere: (enabled: boolean) => git.run<boolean>('Remember resolutions', 'set_rerere', { enabled })
  }
}
