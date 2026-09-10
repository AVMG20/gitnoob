import type { DiffHunk, DiffLine, FileDiff } from './useGit'

/**
 * Drawing only the code that is on screen.
 *
 * Both views used to render every line of the file — a couple of thousand rows
 * of four or five elements each, plus a span per coloured token inside them.
 * Nothing about that is wrong until it is scrolled: the engine has the whole
 * document to keep laid out and painted, and a fast flick of the wheel leaves
 * the text visibly trailing the scrollbar. So each view draws the rows the box
 * can actually show, with the space above and below held open so the scrollbar
 * still measures the whole file.
 *
 * The arithmetic lives here rather than in the two components because it is the
 * part that is easy to get subtly wrong — a window an item short leaves a strip
 * of blank at one edge, and only while scrolling — and it is the part a test
 * can reach without a browser.
 */

/** One line of code: `line-height: 1.5` on the 12px monospace both views use. */
export const CODE_ROW = 18

/** A hunk heading: one line of text, its padding, and the rule above and below. */
export const HUNK_HEAD = 24

/**
 * Rows drawn beyond each edge of the box.
 *
 * Enough that a normal scroll never outruns them between frames, and few enough
 * that the window stays a fraction of a long file.
 */
export const OVERSCAN = 12

export interface Window {
  first: number
  last: number
}

/** Which rows of a list of equal-height ones are worth drawing. */
export function windowOf(total: number, top: number, view: number, row = CODE_ROW): Window {
  if (total <= 0) return { first: 0, last: 0 }
  const first = Math.max(0, Math.floor(top / row) - OVERSCAN)
  // `view` is 0 until the box has been measured, which is before the first
  // paint: a window of nothing then would render an empty page and never be
  // asked again, because nothing it did would make the box scroll.
  const height = view || row * 40
  const last = Math.min(total, Math.ceil((top + height) / row) + OVERSCAN)
  return { first, last }
}

/** A heading or a line of a patch, and where down the view it sits. */
export interface DiffRow {
  kind: 'head' | 'line'
  /** Which hunk it belongs to, which is what the stage and discard buttons act on. */
  hunk: number
  line: DiffLine | null
  /** Where in that hunk's lines it sits, for picking a run of them. `-1` on a
      heading, which is not one of them. */
  at: number
  top: number
  height: number
}

/**
 * A patch flattened into the rows that draw it.
 *
 * Headings and lines are not the same height, so where a row sits cannot be its
 * index times anything; the run is walked once and each row carries its own
 * offset. Walked once per patch, not once per frame.
 */
export function diffRows(hunks: DiffHunk[]): { rows: DiffRow[]; height: number } {
  const rows: DiffRow[] = []
  let top = 0
  hunks.forEach((hunk, at) => {
    rows.push({ kind: 'head', hunk: at, line: null, at: -1, top, height: HUNK_HEAD })
    top += HUNK_HEAD
    hunk.lines.forEach((line, within) => {
      rows.push({ kind: 'line', hunk: at, line, at: within, top, height: CODE_ROW })
      top += CODE_ROW
    })
  })
  return { rows, height: top }
}

/**
 * Which of those rows to draw, found by search rather than by division.
 *
 * The rows are in order of `top` and each knows its own height, so the first
 * one on screen is a binary search away — which matters, because the
 * alternative is walking a patch of ten thousand rows on every frame.
 */
export function diffWindow(rows: DiffRow[], top: number, view: number): Window {
  if (!rows.length) return { first: 0, last: 0 }
  const height = view || CODE_ROW * 40
  let low = 0
  let high = rows.length - 1
  while (low < high) {
    const mid = (low + high) >> 1
    if (rows[mid]!.top + rows[mid]!.height <= top) low = mid + 1
    else high = mid
  }
  const first = Math.max(0, low - OVERSCAN)
  let last = low
  while (last < rows.length && rows[last]!.top < top + height) last++
  return { first, last: Math.min(rows.length, last + OVERSCAN) }
}

/**
 * Whether this is a line of a file at all.
 *
 * Git's "\\ No newline at end of file" rides in the hunk as though it were
 * one, with an origin of its own. It is a remark about the line above it, so
 * everything that counts, marks or measures lines steps over it.
 */
const real = (line: DiffLine) => line.origin !== '\\'

/**
 * The line of the new file the first change sits on, or null when nothing in
 * the patch has one.
 *
 * A deletion has no line of its own in the new file, so it answers with the
 * line it used to sit above — the seam, which is where the file view draws its
 * mark and so where the eye is being sent.
 */
export function firstChangedLine(diff: FileDiff | null): number | null {
  for (const hunk of diff?.hunks ?? []) {
    const lines = hunk.lines.filter(real)
    for (let at = 0; at < lines.length; at++) {
      const line = lines[at]!
      if (line.origin === ' ') continue
      if (line.new_lineno !== null) return line.new_lineno
      // A run of deletions: the seam is the next line that is in the new file.
      for (let next = at + 1; next < lines.length; next++) {
        const after = lines[next]!
        if (after.new_lineno !== null) return after.new_lineno
      }
      return null
    }
  }
  return null
}

/** What happened to a line of the file as it now stands. */
export type LineMark = 'added' | 'changed' | null

export interface Line {
  number: number
  mark: LineMark
  /** What this line said before it was changed, when it replaced something. */
  was: string[]
  /** Lines deleted immediately above this one, with nothing put in their place. */
  removed: string[]
}


/**
 * The file, line by line, with what changed marked against it.
 *
 * An editor's gutter distinguishes three things, and so does this: a line that
 * is new, a line that replaced one, and a place where lines were taken out and
 * nothing put back. Git's diff does not name the middle one — it is a deletion
 * and an insertion sitting together — so a run of the two is read as a change
 * to the lines that survived it, which is what someone reading the file sees.
 */
export function markedLines(text: string | null, hunks: DiffHunk[]): Line[] {
  if (text === null) return []
  const source = text.split('\n')
  // A file that ends in a newline splits into a last empty piece that is not a
  // line of the file.
  if (source.length && source[source.length - 1] === '') source.pop()

  const marks = new Map<number, LineMark>()
  // The text of what went, not just how much of it: a gutter mark that can be
  // asked what it replaced is worth more than one that can only say something
  // happened here.
  const before = new Map<number, string[]>()
  const gaps = new Map<number, string[]>()

  for (const hunk of hunks) {
    const lines = hunk.lines.filter(real)
    // Walk each run of touched lines together: what a run is made of decides
    // whether it reads as an addition or as a change.
    let index = 0
    while (index < lines.length) {
      if (lines[index]!.origin === ' ') {
        index++
        continue
      }
      let end = index
      const deleted: string[] = []
      const added: number[] = []
      while (end < lines.length && lines[end]!.origin !== ' ') {
        const line = lines[end]!
        if (line.origin === '-') deleted.push(line.content)
        else if (line.new_lineno) added.push(line.new_lineno)
        end++
      }
      const deletions = deleted.length

      if (added.length) {
        // As many added lines as were deleted are the replacements; anything
        // beyond that is genuinely new.
        for (const [at, number] of added.entries()) {
          marks.set(number, at < deletions ? 'changed' : 'added')
          if (at >= deletions) continue
          // Where more went than came back, the surplus has no line of its own
          // to hang from, so it joins the last of the replacements: the run
          // still reads as one change, and none of it goes unaccounted for.
          const replaced =
            at === added.length - 1 ? deleted.slice(at) : deleted.slice(at, at + 1)
          before.set(number, replaced)
        }
      } else if (deletions) {
        // Nothing replaced them, so the mark belongs to the seam: the line the
        // deleted ones used to sit above.
        const next = lines[end]?.new_lineno ?? source.length + 1
        gaps.set(next, [...(gaps.get(next) ?? []), ...deleted])
      }
      index = end
    }
  }

  return source.map((_, at) => ({
    number: at + 1,
    mark: marks.get(at + 1) ?? null,
    was: before.get(at + 1) ?? [],
    removed: gaps.get(at + 1) ?? []
  }))
}

/** A run of lines that were changed together, and everything they replaced. */
export interface WasRun {
  /** The first line of the run. */
  start: number
  /** The last line of the run, which is what the panel hangs below. */
  end: number
  /** What the whole run replaced, in order. */
  was: string[]
}

/**
 * Consecutive changed lines folded into the one change they are.
 *
 * Git reports a rewritten block as a deletion and an insertion sitting
 * together, and the line-by-line reading of that gives every line its own mark
 * to click and its own one-line answer. That is not what happened: three lines
 * were rewritten once. So a run of lines that each replaced something is read
 * as a single change — one bar down the gutter, and one panel showing the
 * three lines as they were, together, which is the only form in which they can
 * be compared with the three that took their place.
 *
 * A seam breaks a run: lines deleted with nothing put in their place are their
 * own change, and they have their own mark on that boundary to say so.
 */
export function wasRuns(lines: Line[]): Map<number, WasRun> {
  const runs = new Map<number, WasRun>()
  let current: WasRun | null = null
  for (const line of lines) {
    if (!line.was.length) {
      current = null
      continue
    }
    if (current && line.number === current.end + 1 && !line.removed.length) {
      current.end = line.number
      current.was.push(...line.was)
    } else {
      current = { start: line.number, end: line.number, was: [...line.was] }
    }
    runs.set(line.number, current)
  }
  return runs
}

/**
 * One bar on the strip beside the scrollbar, in fractions of the whole.
 *
 * The first four say what happened to a line, which is what a diff has to
 * report. The last three are the conflict resolver's, where the question is not
 * what changed but what still wants an answer: a region nobody has looked at,
 * one that has been decided, and one set to be dropped entirely.
 */
export interface Mark {
  kind: 'added' | 'changed' | 'removed' | 'gone' | 'open' | 'settled' | 'dropped'
  top: number
  height: number
}

/**
 * Folds a run of marked rows into one bar.
 *
 * Twenty changed lines in a row are one thing to look at, and twenty bars a
 * pixel apart are a smear. Rows arrive in order, so a run is a row whose kind
 * matches the bar above it and which starts where that bar ended.
 */
function fold(found: { kind: Mark['kind']; top: number; height: number }[], total: number): Mark[] {
  if (!total) return []
  return found.map((mark) => ({
    kind: mark.kind,
    top: mark.top / total,
    height: mark.height / total
  }))
}

function run(
  into: { kind: Mark['kind']; top: number; height: number }[],
  kind: Mark['kind'],
  top: number,
  height: number
) {
  const last = into[into.length - 1]
  if (last && last.kind === kind && top <= last.top + last.height + 1.5) {
    last.height = top + height - last.top
  } else {
    into.push({ kind, top, height })
  }
}

/** Where the changes are in the whole file, for the strip beside the file view. */
export function fileMarks(lines: Line[]): Mark[] {
  const found: { kind: Mark['kind']; top: number; height: number }[] = []
  lines.forEach((line, at) => {
    const top = at * CODE_ROW
    // The seam is drawn on the boundary above the line, so it is its own bar
    // rather than part of whatever the line itself is.
    if (line.removed.length) run(found, 'gone', top - 2, 4)
    if (line.mark) run(found, line.mark, top, CODE_ROW)
  })
  return fold(found, lines.length * CODE_ROW)
}

/** The same, for the patch view, whose rows are not all the same height. */
export function patchMarks(rows: DiffRow[], height: number): Mark[] {
  const found: { kind: Mark['kind']; top: number; height: number }[] = []
  for (const row of rows) {
    if (row.kind !== 'line' || !row.line) continue
    if (row.line.origin === '+') run(found, 'added', row.top, row.height)
    else if (row.line.origin === '-') run(found, 'removed', row.top, row.height)
  }
  return fold(found, height)
}

/**
 * The file as it was before the change, rebuilt from the file as it is now.
 *
 * A deleted line is not in the new file at all, so the one place a diff could
 * never colour properly was its own `-` lines: they fell back to being read one
 * at a time, and a line at a time cannot see the `<script>` tag that made it
 * TypeScript or the `/*` that made it a comment. Reading the old copy off disk
 * would be another round trip per file; it is not needed, because the diff and
 * the new file together already say what the old one was.
 *
 * Every line of the diff carries the number it had on each side, so the old
 * file is the new one with each hunk's new-side lines swapped back for its
 * old-side lines, and the untouched stretches between hunks copied across.
 *
 * Rebuilt optimistically and then checked: every line the diff claims for the
 * old side has to be found at its own number in the result. The diff and the
 * file are read separately, so a write landing between the two would otherwise
 * produce a plausible file that is not the one being shown. A null return means
 * the caller should keep doing what it did before.
 */
export function oldText(hunks: DiffHunk[], now: string[]): string[] | null {
  const old: string[] = []
  // 1-based, into `now`: how far through the new file the walk has come.
  let at = 1

  for (const hunk of hunks) {
    const sides = hunk.lines.filter((line) => line.origin !== '\\')
    const firstNew = sides.find((line) => line.new_lineno !== null)?.new_lineno ?? null
    // A hunk that only deletes has no new-side line to sit against, and so it
    // sits exactly where the walk has got to.
    if (firstNew !== null) {
      if (firstNew < at) return null
      while (at < firstNew) old.push(now[at++ - 1] ?? '')
    }
    for (const line of sides) if (line.old_lineno !== null) old.push(line.content)
    const lastNew = sides.reduce<number | null>(
      (last, line) => line.new_lineno ?? last,
      null
    )
    if (lastNew !== null) at = lastNew + 1
  }
  while (at <= now.length) old.push(now[at++ - 1] ?? '')

  // The check, and it is the new side that carries it. Every old-side line was
  // copied out of the diff, so checking those against the rebuild only asks
  // whether the splice put them back where it found them. What says the two
  // were read of the same version of the file is the new side: each line the
  // diff claims has to be at that number in the text on disk. Where it is not,
  // a write landed between the two reads and every offset here is out.
  for (const hunk of hunks) {
    for (const line of hunk.lines) {
      if (line.new_lineno !== null && now[line.new_lineno - 1] !== line.content) return null
      if (line.old_lineno !== null && old[line.old_lineno - 1] !== line.content) return null
    }
  }
  return old
}
