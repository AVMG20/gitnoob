import type { BranchDeletion } from './useGit'

/**
 * The answer to "is this safe to delete?", worked out from what the preview
 * found. Kept apart from the dialog so it can be read on its own, and so the
 * cases that used to be got wrong are testable without a window.
 *
 * The dialog title already carries the branch name, so nothing here repeats
 * it — these read as short verdicts, not sentences about a branch.
 */
export interface Verdict {
  /** `safe`: nothing is lost. `careful`: nothing is lost yet something is
   *  worth reading. `danger`: commits go away. */
  tone: 'safe' | 'careful' | 'danger'
  /** The verdict in a handful of words. Usually the whole message. */
  headline: string
  /** The one thing the headline cannot carry, when there is one. Left off
   *  wherever the headline already says it, which is most cases. */
  detail?: string
  /** Whether the tick box has to be ticked before this can be pressed. */
  acknowledge: boolean
}

function commits(count: number) {
  return count === 1 ? '1 commit' : `${count} commits`
}

function list(names: string[]) {
  if (names.length <= 2) return names.join(' and ')
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
}

/**
 * The branch everything is measured against.
 *
 * The repository's trunk, because "has this landed?" is a question about where
 * work lands, not about wherever you happen to be standing. HEAD only when the
 * repository has no trunk at all.
 */
function against(found: BranchDeletion) {
  return found.against ?? found.head ?? 'the commit you are on'
}

/** The remote's own name — `origin`, not `origin/some/long/branch/name`. */
function remoteName(found: BranchDeletion) {
  return found.remote?.remote ?? found.upstream?.split('/')[0] ?? 'the remote'
}

/**
 * Deleting the local branch. Git keeps the commits in the reflog for weeks
 * after the label goes, so even the bad case is not quite the end — but the
 * only way a beginner finds that out is being told.
 */
export function localVerdict(found: BranchDeletion): Verdict {
  if (found.trunk_holds) {
    return { tone: 'safe', headline: `Merged into ${against(found)}`, acknowledge: false }
  }

  // The remote has it all: the branch can be checked out again from there.
  if (found.upstream && found.unpushed === 0) {
    return {
      tone: 'careful',
      headline: `Not on ${against(found)} — ${remoteName(found)} has every commit`,
      acknowledge: false
    }
  }

  // Another local branch holds the work — which is not the same as it being
  // safe. A branch like `staging` holds every commit right up to the morning
  // somebody resets it, and this used to read as "safe to delete" on the
  // strength of exactly that.
  if (found.also_on.length) {
    const many = found.also_on.length > 1
    return {
      tone: 'careful',
      headline: `Not on ${against(found)} — only ${list(found.also_on)}`,
      detail: `A reset on ${many ? 'those branches' : 'that branch'} takes the work with it.`,
      acknowledge: false
    }
  }

  const orphaned = found.only_here
  return {
    tone: 'danger',
    headline: `${commits(orphaned)} left with no branch`,
    detail: `${
      found.upstream ? `Not on ${found.upstream}` : 'No remote copy'
    }. git reflog gets ${orphaned === 1 ? 'it' : 'them'} back for about 30 days.`,
    acknowledge: true
  }
}

/**
 * Deleting the copy on the remote. A different question with a different cost:
 * there is no reflog on the server, and it goes for everyone at once.
 */
export function remoteVerdict(found: BranchDeletion): Verdict | null {
  const remote = found.remote
  if (!remote) return null

  if (remote.unmerged === 0) {
    return {
      tone: 'careful',
      headline: `${remote.remote} holds nothing extra`,
      acknowledge: false
    }
  }

  return {
    tone: 'danger',
    headline: `${commits(remote.unmerged)} only on ${remote.remote}`,
    detail: `Likely somebody else's. Deleting there is for everyone, and no reflog brings ${
      remote.unmerged === 1 ? 'it' : 'them'
    } back.`,
    acknowledge: true
  }
}

/** `git branch -d` refuses anything HEAD cannot reach; `-D` is for those. */
export function needsForce(found: BranchDeletion) {
  return !found.merged
}
