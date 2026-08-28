import { computed, reactive } from 'vue'
import { invoke } from './useInvoke'
import { useGit } from './useGit'

/**
 * The interactive rebase: the plan being built, and the one being run.
 *
 * Module scope like the rest, because two components read it — the pane that
 * builds the plan, and the strip under the toolbar that stands while git is
 * part-way through one.
 */

/** What to do with one commit. The names are git's own. */
export type RebaseAction = 'pick' | 'reword' | 'squash' | 'fixup' | 'edit' | 'drop'

export const ACTIONS: RebaseAction[] = ['pick', 'reword', 'squash', 'fixup', 'edit', 'drop']

/** What each action means, in the words the menu uses. */
export const ACTION_WORDS: Record<RebaseAction, { label: string; note: string }> = {
  pick: { label: 'Keep it', note: 'as it is' },
  reword: { label: 'Keep it, new message', note: 'stops to ask' },
  squash: { label: 'Fold into the one above', note: 'join the messages' },
  fixup: { label: 'Fold into the one above', note: 'drop this message' },
  edit: { label: 'Stop here', note: 'to change the files' },
  drop: { label: 'Throw it away', note: 'the changes go too' }
}

export interface Candidate {
  oid: string
  short: string
  summary: string
  author: string
  email: string
  time: number
  /** Already on a remote, so rewriting it means a force push afterwards. */
  pushed: boolean
}

/** One row of the plan: a commit and what is to become of it. */
export interface PlanRow extends Candidate {
  action: RebaseAction
}

/** Where a running rebase has got to. */
export interface RebaseProgress {
  at: number
  total: number
  stopped: string | null
  summary: string | null
  /** True when this stop is one the plan asked for as a reword. */
  rewording: boolean
  message: string | null
}

const store = reactive({
  /** The commit being rebased onto, while the pane is open. */
  onto: null as string | null,
  /** What to call it on screen. */
  ontoLabel: '',
  rows: [] as PlanRow[],
  open: false,
  loading: false,
  starting: false,
  /** Where a running rebase is, or null when none is. */
  progress: null as RebaseProgress | null
})

export function useRebase() {
  const git = useGit()

  /** What the plan leaves behind, for the column beside it. */
  const outcome = computed(() => {
    const kept: { row: PlanRow; folded: number }[] = []
    for (const row of store.rows) {
      if (row.action === 'drop') continue
      if ((row.action === 'squash' || row.action === 'fixup') && kept.length) {
        kept[kept.length - 1]!.folded += 1
        continue
      }
      kept.push({ row, folded: 0 })
    }
    return kept
  })

  const dropped = computed(() => store.rows.filter((row) => row.action === 'drop').length)
  const stops = computed(
    () => store.rows.filter((row) => row.action === 'edit' || row.action === 'reword').length
  )
  const rewriting = computed(() => store.rows.some((row) => row.pushed))

  /**
   * Why the plan cannot be run, or null when it can.
   *
   * The same two rules the backend refuses on, said here so the button can be
   * disabled with a reason rather than the press being answered by an error.
   */
  const refusal = computed(() => {
    const first = store.rows.find((row) => row.action !== 'drop')
    if (!first) return 'Every commit is dropped. Use reset instead.'
    if (first.action === 'squash' || first.action === 'fixup') {
      return 'The first commit has nothing above it to fold into.'
    }
    return null
  })

  /** Opens the pane on the commits above `onto`. */
  async function planFrom(onto: string, label: string) {
    store.open = true
    store.onto = onto
    store.ontoLabel = label
    store.loading = true
    store.rows = []
    try {
      const found = await invoke<Candidate[]>('rebase_plan', { onto })
      store.rows = found.map((one) => ({ ...one, action: 'pick' as RebaseAction }))
    } catch (error) {
      git.note(`Rebase: ${String(error)}`, 'error')
      close()
    } finally {
      store.loading = false
    }
  }

  function close() {
    store.open = false
    store.onto = null
    store.rows = []
  }

  function setAction(at: number, action: RebaseAction) {
    const row = store.rows[at]
    if (row) row.action = action
  }

  /** Moves a row, which is the whole point of the list. */
  function move(from: number, to: number) {
    if (from === to || from < 0 || from >= store.rows.length) return
    const [row] = store.rows.splice(from, 1)
    if (!row) return
    store.rows.splice(Math.max(0, Math.min(to, store.rows.length)), 0, row)
  }

  /** Puts every row back to being kept, in the order git listed them. */
  function reset() {
    store.rows.sort((a, b) => a.time - b.time)
    for (const row of store.rows) row.action = 'pick'
  }

  async function start() {
    if (!store.onto || refusal.value || store.starting) return
    store.starting = true
    const steps = store.rows.map((row) => ({ oid: row.oid, action: row.action }))
    const said = await git.run<string>('Rebase', 'rebase_start', { onto: store.onto, steps })
    store.starting = false
    if (said === null) return
    git.note(said)
    await readProgress()
    // A rebase that ran straight through has nothing left to show a plan for.
    if (!store.progress) close()
  }

  /** Where the running rebase is, read when git says one is running. */
  async function readProgress() {
    try {
      store.progress = await invoke<RebaseProgress | null>('rebase_progress')
    } catch {
      // Not being able to read it is the same as there not being one: the
      // strip goes, and git's own state is still whatever it is.
      store.progress = null
    }
  }

  async function carryOn(command: string, label: string) {
    const said = await git.run<string>(label, command, {})
    if (said !== null) git.note(said)
    await readProgress()
    if (!store.progress) close()
  }

  const resume = () => carryOn('rebase_continue', 'Continue rebase')
  const skip = () => carryOn('rebase_skip', 'Skip this commit')
  const abort = () => carryOn('rebase_abort', 'Abort rebase')

  async function reword(message: string) {
    const said = await git.run<string>('Reword', 'rebase_reword', { message })
    if (said !== null) git.note(said)
    await readProgress()
    if (!store.progress) close()
  }

  return {
    store,
    outcome,
    dropped,
    stops,
    rewriting,
    refusal,
    planFrom,
    close,
    setAction,
    move,
    reset,
    start,
    readProgress,
    resume,
    skip,
    abort,
    reword
  }
}
