import type { RComment, Thread } from './useReview'

/**
 * Folds a review's flat comment feed into threads.
 *
 * Both forges hand remarks over as one list; the page wants them as
 * conversations — a root and everything answered under it — and the diff
 * additionally wants them by where they stand. Kept apart from the store so
 * the folding itself can be checked without one.
 */

/** The key a line-anchored thread files itself under. */
export function threadKey(path: string, side: string, line: number): string {
  return `${path}\u0000${side}\u0000${line}`
}

export function foldThreads(comments: RComment[]): { talk: Thread[]; byLine: Map<string, Thread[]> } {
  // Every remark by id, replies included, so a chain can be walked to its
  // end however it was pointed.
  const byId = new Map<number, Thread>()
  const raw = new Map(comments.map((comment) => [comment.id, comment]))
  const talk: Thread[] = []
  const byLine = new Map<string, Thread[]>()

  // Roots before replies: the feed arrives sorted by time, but a reply can
  // still outrun its root when clocks disagree, and a thread whose root has
  // not been seen yet is a reply pointing nowhere.
  for (const comment of comments) {
    if (comment.reply_to) continue
    const thread: Thread = { key: String(comment.id), id: comment.id, root: comment, replies: [] }
    byId.set(comment.id, thread)
    // A remark the forge filed under a file but gave no line to stand on
    // still deserves reading; the conversation is the one place left.
    if (comment.kind === 'issue' || comment.line === null) {
      talk.push(thread)
    } else if (comment.path) {
      const key = threadKey(comment.path, comment.side ?? 'new', comment.line)
      byLine.set(key, [...(byLine.get(key) ?? []), thread])
    }
  }
  for (const comment of comments) {
    if (!comment.reply_to) continue
    // Replies chain to their thread's root even when pointed at each other,
    // so a long argument still reads as one conversation. The walk follows
    // the raw pointers — a reply is not in the thread map — and a guard keeps
    // a malformed loop from spinning it forever.
    let parentId = comment.reply_to
    for (let steps = 0; steps < 100; steps++) {
      const parent = raw.get(parentId)
      if (!parent?.reply_to) break
      parentId = parent.reply_to
    }
    byId.get(parentId)?.replies.push(comment)
  }
  return { talk, byLine }
}

/**
 * One remark, marked up as a quotation of it.
 *
 * The body and nothing else, which is what a forge's own quote reply writes:
 * a name and a "said" line are noise above the words they introduce.
 */
export function quoted(body: string): string {
  return `${body
    .trim()
    .split('\n')
    .map((line) => `> ${line}`)
    .join('\n')}\n\n`
}

/**
 * The quotation added to whatever is already being written.
 *
 * Quoting the same remark twice adds nothing the second time: the box is a
 * draft somebody is writing in, not a log of every button they pressed.
 */
export function quotedInto(held: string, body: string): string {
  const quotation = quoted(body)
  if (held.includes(quotation.trim())) return held
  const before = held.trim()
  return before ? `${before}\n\n${quotation}` : quotation
}
