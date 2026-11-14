import type { DiffHunk, DiffLine } from './useGit'

/**
 * The changes behind the bars in the editor's gutter, one per run of touched
 * lines, as the card that opens on a bar shows them: what was there before,
 * and how to put it back.
 *
 * Read from the same hunks as the bars (the work tree against HEAD), and
 * split the same way `markedLines` splits them, so a bar and its card always
 * talk about the same lines.
 */

export interface Change {
  /** Its place among all the file's changes, from 0. */
  index: number
  kind: 'added' | 'removed' | 'modified'
  /** What the lines were, before; empty for an addition. */
  removed: string[]
  /** What they are now; empty for a deletion. */
  added: string[]
  /** The first and last line of it in the file now. A deletion has none of
      its own: both are the line its wedge sits on, the one after the gap. */
  from: number
  to: number
  /** The line the old lines came after, for putting a deletion back. */
  after: number
}

const real = (line: DiffLine) => line.origin !== '\\'

/** Every change in the file, top to bottom. */
export function changesOf(hunks: DiffHunk[]): Change[] {
  const out: Change[] = []
  for (const hunk of hunks) {
    const lines = hunk.lines.filter(real)
    // The new-side line before the run, from the header when the run opens it.
    const header = /\+(\d+)(?:,(\d+))?/.exec(hunk.header)
    let previous = Math.max(0, Number(header?.[1] ?? 1) - 1)
    if (header?.[2] === '0') previous = Number(header[1])
    let index = 0
    while (index < lines.length) {
      const line = lines[index]!
      if (line.origin === ' ') {
        previous = line.new_lineno ?? previous
        index++
        continue
      }
      const removed: string[] = []
      const added: string[] = []
      let first: number | null = null
      let last: number | null = null
      while (index < lines.length && lines[index]!.origin !== ' ') {
        const one = lines[index]!
        if (one.origin === '-') removed.push(one.content)
        else if (one.origin === '+') {
          added.push(one.content)
          first ??= one.new_lineno
          last = one.new_lineno ?? last
        }
        index++
      }
      const kind = !added.length ? 'removed' : !removed.length ? 'added' : 'modified'
      const seam = previous + 1
      out.push({
        index: out.length,
        kind,
        removed,
        added,
        from: first ?? seam,
        to: last ?? seam,
        after: previous
      })
      if (last !== null) previous = last
    }
  }
  return out
}

/** The change a bar on `line` belongs to, or null. */
export function changeAt(changes: Change[], line: number): Change | null {
  return changes.find((one) => line >= one.from && line <= one.to) ?? null
}

/** The same change in a newer list, by what it is rather than where: typing
    above it moves it down without making it a different change. */
export function sameChange(changes: Change[], was: Change): Change | null {
  const same = (one: Change) =>
    one.kind === was.kind &&
    one.removed.join('\n') === was.removed.join('\n') &&
    one.added.join('\n') === was.added.join('\n')
  const matching = changes.filter(same)
  if (!matching.length) return null
  // Of several identical ones, the nearest.
  return matching.reduce((best, one) =>
    Math.abs(one.from - was.from) < Math.abs(best.from - was.from) ? one : best
  )
}

/**
 * The edit that puts a change back the way it was, against a document of
 * `lines` lines whose line starts and ends `at` gives. One edit, so it is one
 * step for undo.
 */
export function revertEdit(
  change: Change,
  doc: { lines: number; length: number; line: (n: number) => { from: number; to: number } }
): { from: number; to: number; insert: string } | null {
  const old = change.removed.join('\n')
  if (change.kind === 'removed') {
    if (change.after <= 0) return { from: 0, to: 0, insert: `${old}\n` }
    if (change.after > doc.lines) return null
    const at = doc.line(change.after).to
    return { from: at, to: at, insert: `\n${old}` }
  }
  if (change.from < 1 || change.to > doc.lines || change.from > change.to) return null
  if (change.kind === 'modified') {
    return { from: doc.line(change.from).from, to: doc.line(change.to).to, insert: old }
  }
  // An addition goes, with the line breaks that came with it.
  if (change.to < doc.lines) {
    return { from: doc.line(change.from).from, to: doc.line(change.to + 1).from, insert: '' }
  }
  const from = change.from > 1 ? doc.line(change.from - 1).to : 0
  return { from, to: doc.length, insert: '' }
}

/**
 * Where an old line differs from its new one, as [from, to) in the old line,
 * widened to whole words. Lines are paired in order; an old line with no
 * partner is changed all through.
 */
export function wordChanges(change: Change): [number, number][][] {
  return change.removed.map((old, at) => {
    const now = change.kind === 'modified' ? change.added[at] : undefined
    if (now === undefined) return old.length ? [[0, old.length]] : []
    let start = 0
    while (start < old.length && start < now.length && old[start] === now[start]) start++
    let end = 0
    while (
      end < old.length - start &&
      end < now.length - start &&
      old[old.length - 1 - end] === now[now.length - 1 - end]
    ) {
      end++
    }
    const word = /[\p{L}\p{N}_$]/u
    while (start > 0 && word.test(old[start - 1]!)) start--
    let stop = old.length - end
    while (stop < old.length && word.test(old[stop]!)) stop++
    return stop > start ? [[start, stop]] : []
  })
}

export interface StagePick {
  hunk: number
  lines: { added: number[]; removed: number[] }
}

/**
 * The same change in the unstaged diff (the work tree against the index),
 * for staging it with the hunk-staging path the Diff view uses.
 *
 * The card reads against HEAD, as the bars do; staging moves what is between
 * the index and the work tree. Lines are matched by their number in the file
 * now, which both diffs share. Null when none of it is unstaged: it is all
 * staged already.
 */
export function stagePick(unstaged: DiffHunk[], change: Change): StagePick | null {
  for (const [hunkAt, hunk] of unstaged.entries()) {
    const lines = hunk.lines.filter(real)
    const header = /\+(\d+)(?:,(\d+))?/.exec(hunk.header)
    let previous = Math.max(0, Number(header?.[1] ?? 1) - 1)
    if (header?.[2] === '0') previous = Number(header[1])
    let index = 0
    const added: number[] = []
    const removed: number[] = []
    while (index < lines.length) {
      const line = lines[index]!
      if (line.origin === ' ') {
        previous = line.new_lineno ?? previous
        index++
        continue
      }
      // One run of this diff: it belongs to the change when it touches the
      // change's lines, or sits in the change's seam.
      const run: DiffLine[] = []
      while (index < lines.length && lines[index]!.origin !== ' ') run.push(lines[index++]!)
      const news = run.filter((one) => one.origin === '+').map((one) => one.new_lineno!)
      const inside = news.length
        ? news.some((number) => number >= change.from && number <= change.to)
        : change.kind === 'removed'
          ? previous === change.after
          : previous >= change.from - 1 && previous <= change.to
      if (inside) {
        for (const one of run) {
          if (one.origin === '+' && one.new_lineno !== null) added.push(one.new_lineno)
          if (one.origin === '-' && one.old_lineno !== null) removed.push(one.old_lineno)
        }
      }
      if (news.length) previous = news[news.length - 1]!
    }
    if (added.length || removed.length) return { hunk: hunkAt, lines: { added, removed } }
  }
  return null
}
