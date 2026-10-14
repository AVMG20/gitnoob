/**
 * Small decisions the commit box makes, kept apart from the component so they
 * can be read, and tested, on their own.
 */

/**
 * The ticket a branch is named after, if it is: `ABC-123` out of
 * `ABC-123-fix-login`, `feature/ABC-123/login` or `jo/ABC-123`.
 *
 * Upper case only, the way Jira and Linear write a key. `fix-123-thing` is a
 * branch about a fix, not ticket FIX-123, and a prefix guessed wrong is worse
 * than none — it goes into the history.
 */
export function ticketFrom(branch: string | null | undefined): string | null {
  if (!branch) return null
  const found = branch.match(/(?:^|[/_-])([A-Z][A-Z0-9]{1,9}-\d+)(?=$|[/_.-])/)
  return found?.[1] ?? null
}

/** What goes in front of the message for a ticket. */
export function ticketPrefix(branch: string | null | undefined): string | null {
  const ticket = ticketFrom(branch)
  return ticket ? `${ticket}: ` : null
}

/**
 * Whether the box holds nothing but what was put there for the user — the
 * ticket prefix — and so still counts as empty: for the merge's own message
 * to go in, and for the commit button, which would otherwise commit "ABC-123:".
 */
export function onlyPrefix(message: string, prefix: string | null): boolean {
  const typed = message.trim()
  return !typed || (!!prefix && typed === prefix.trim())
}

/**
 * Whether the branch checked out is the repository's trunk.
 *
 * The trunk may be named as its remote copy — `origin/main` in a clone that
 * only ever pulls — in which case the local branch of the same name is it.
 * Only a real remote's name is taken off the front: `feature/x` chosen as the
 * trunk is not a remote called `feature`.
 */
export function onTrunk(
  head: string | null | undefined,
  trunk: string | null | undefined,
  remotes: string[]
): boolean {
  if (!head || !trunk) return false
  if (head === trunk) return true
  const slash = trunk.indexOf('/')
  if (slash < 0) return false
  return remotes.includes(trunk.slice(0, slash)) && trunk.slice(slash + 1) === head
}

/**
 * The message as git should get it: a blank line between the subject and the
 * body, which every tool that shows them apart expects.
 */
export function composeMessage(message: string): string {
  const lines = message.split('\n')
  const head = (lines.shift() ?? '').trim()
  const rest = lines.join('\n').replace(/^\s*\n/, '').trimEnd()
  return rest ? `${head}\n\n${rest}` : head
}
