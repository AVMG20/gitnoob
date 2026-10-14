import type { LocalBranch, PushBlock, Tidy, Trunk } from './useGit'

/**
 * The small decisions behind keeping branches in step with their remotes,
 * kept apart from the components that act on them so each can be read, and
 * tested, without a window.
 */

/**
 * Whether the refused-push strip should lead with force-with-lease rather than
 * a pull: only when the backend found that everything the push would drop has
 * a copy here.
 *
 * It used to lead with force too whenever the last step on the branch was a
 * rebase or an amend. That was a guess, and a wrong one exactly when it
 * mattered: a branch rebased here while a colleague pushed to it is refused
 * for the colleague's commits, and leading with force put the button that
 * drops them first. The backend measures it; the strip goes by that alone.
 */
export function leadWithForce(block: PushBlock | null) {
  return !!block?.rewritten
}

/**
 * The local name of the trunk: `main` for a trunk named `main` or, when there
 * is no local branch of that name, `origin/main`.
 */
export function trunkLocalName(trunk: Trunk | null | undefined, locals: LocalBranch[]) {
  const name = trunk?.name
  if (!name) return null
  if (locals.some((one) => one.name === name)) return name
  const slash = name.indexOf('/')
  return slash > 0 ? name.slice(slash + 1) : name
}

/**
 * Why a push straight to `branch` deserves a second look, or null when it
 * does not.
 *
 * Only the local half of the question: the setting, and whether `branch` is
 * the trunk. The forge knows which branches are protected; this is the one
 * function to extend when that is asked too.
 */
export async function protectedPushWarning(
  branch: string,
  trunk: Trunk | null | undefined,
  locals: LocalBranch[],
  enabled: boolean,
  isProtected?: (branch: string) => Promise<boolean | null>
): Promise<string | null> {
  if (!enabled) return null
  const local = trunkLocalName(trunk, locals)
  if (local && branch === local) {
    return `${branch} is the main branch. Pushing straight to it skips any review.`
  }
  // Any other branch is only worth asking about when the forge says it is
  // protected. No answer — no forge, no token, no network — is not a yes.
  if (isProtected && (await isProtected(branch).catch(() => null)) === true) {
    return `${branch} is protected on the forge. A push straight to it is likely to be refused, and skips any review.`
  }
  return null
}

/**
 * The remotes a failed fetch names.
 *
 * `git fetch --all` carries on past a remote it cannot reach and says
 * "error: could not fetch <name>" at the end; a fetch of one remote names it
 * in the "fatal:" line instead, so that one is passed in.
 */
export function failedRemotes(text: string, asked?: string | null): string[] {
  const found = new Set<string>()
  for (const match of text.matchAll(/could not fetch '?([^'\s]+?)'?(?:\s|$)/gi)) {
    if (match[1]) found.add(match[1])
  }
  if (asked && text.trim()) found.add(asked)
  return [...found]
}

/** Why a branch is on the tidy list, in a few words. */
export function tidyReason(one: Tidy): string {
  const into = one.into ?? 'main'
  switch (one.reason) {
    case 'merged':
      return `merged into ${into}`
    case 'squash':
      return `squash-merged into ${into}`
    case 'rebase':
      return `rebased onto ${into}`
    default:
      return one.gone ? `${one.gone} was deleted; its work is on no main branch` : 'upstream deleted'
  }
}

/** The branches a deletion should start with ticked: only the proven ones. */
export function tickedAtFirst(list: Tidy[]): Set<string> {
  return new Set(list.filter((one) => one.proven).map((one) => one.name))
}

/** Identifies one tidy list, so a dismissed strip stays dismissed until it changes. */
export function tidyKey(list: Tidy[]): string {
  return list
    .map((one) => `${one.name}@${one.oid}`)
    .sort()
    .join(' ')
}

/** "3 commits" and "1 commit". */
export function commitCount(count: number) {
  return count === 1 ? '1 commit' : `${count} commits`
}
