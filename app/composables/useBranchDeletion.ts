import type { BranchDeletion } from './useGit'

/**
 * The answer to "is this safe to delete?", worked out from what the preview
 * found. Kept apart from the dialog so it can be read on its own, and so the
 * cases that used to be got wrong are testable without a window.
 */
export interface Verdict {
  /** `safe`: nothing is lost. `careful`: nothing is lost yet something is
   *  worth reading. `danger`: commits go away. */
  tone: 'safe' | 'careful' | 'danger'
  /** The answer in a handful of words, for the top of the dialog. */
  headline: string
  /** Why, in a sentence or two. */
  detail: string
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

/** The branch the local half is measured against, named the way git would. */
function here(found: BranchDeletion) {
  return found.head ?? 'the commit you are on'
}

/**
 * Deleting the local branch. Git keeps the commits in the reflog for weeks
 * after the label goes, so even the bad case is not quite the end — but the
 * only way a beginner finds that out is being told.
 */
export function localVerdict(found: BranchDeletion): Verdict {
  if (found.merged) {
    return {
      tone: 'safe',
      headline: 'Safe to delete',
      detail: `${here(found)} already holds every commit on ${found.name}, so deleting the branch here loses nothing.`,
      acknowledge: false
    }
  }

  // Not merged into HEAD, but another local branch holds the work. Git's own
  // `-d` refuses this; nothing is lost by it all the same.
  if (found.also_on.length) {
    return {
      tone: 'safe',
      headline: 'Safe to delete',
      detail: `${found.name} is not merged into ${here(found)}, but ${list(found.also_on)} ${
        found.also_on.length === 1 ? 'holds' : 'hold'
      } every commit on it.`,
      acknowledge: false
    }
  }

  // The remote has it all: the branch can be checked out again from there.
  if (found.upstream && found.unpushed === 0) {
    return {
      tone: 'careful',
      headline: 'Safe here — the remote keeps a copy',
      detail: `${found.name} is not merged into ${here(found)}, but ${found.upstream} holds every commit on it. You can check it out again from there.`,
      acknowledge: false
    }
  }

  const orphaned = found.only_here
  const where = found.upstream
    ? `on ${found.name} and not on ${found.upstream}`
    : `only on ${found.name}, which has no remote copy`
  return {
    tone: 'danger',
    headline: `${commits(orphaned)} would be left with no branch`,
    detail: `${commits(orphaned)} ${orphaned === 1 ? 'is' : 'are'} ${where}. Deleting it leaves ${
      orphaned === 1 ? 'that commit' : 'those commits'
    } reachable from nothing. Git keeps ${
      orphaned === 1 ? 'it' : 'them'
    } in the reflog for about 30 days, so \`git reflog\` can still bring ${
      orphaned === 1 ? 'it' : 'them'
    } back until then.`,
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
      headline: `${remote.name} holds nothing new`,
      detail: `Every commit on ${remote.name} is already on ${here(found)}. Deleting it there removes the branch for everyone, but no work goes with it.`,
      acknowledge: false
    }
  }

  return {
    tone: 'danger',
    headline: `${commits(remote.unmerged)} exist only on ${remote.name}`,
    detail: `${commits(remote.unmerged)} on ${remote.name} ${
      remote.unmerged === 1 ? 'is' : 'are'
    } not on ${here(found)} — most likely somebody else's work. Deleting the branch there removes ${
      remote.unmerged === 1 ? 'it' : 'them'
    } for everyone, and no reflog here brings ${remote.unmerged === 1 ? 'it' : 'them'} back.`,
    acknowledge: true
  }
}

/** `git branch -d` refuses anything HEAD cannot reach; `-D` is for those. */
export function needsForce(found: BranchDeletion) {
  return !found.merged
}
