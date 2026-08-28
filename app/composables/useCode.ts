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
