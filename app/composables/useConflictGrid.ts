import { CODE_ROW, type Mark } from './useCode'
import type { ConflictBlock, Resolution } from './useGit'

/**
 * The one row model every pane of the resolver is drawn from.
 *
 * The panes used to be built a side at a time, which is why they drifted: a
 * region where our side has three lines and theirs has nine pushes everything
 * below it nine lines down on the right and three on the left, so the two
 * sides of the same conflict end up on different parts of the screen and
 * scrolling them together is meaningless. Here each row carries what every
 * side has on it — and nothing where a side has run out — so the sides line up
 * by construction, one scroll position means the same thing in all of them,
 * and the strip beside the scrollbar can say where the conflicts are.
 */

/** One line of code: `line-height: 1.5` on the 12px monospace the panes use. */
export const ROW = CODE_ROW

/** The bar above a region, which carries its checkbox. */
export const HEAD = 22

export type Side = 'ours' | 'theirs' | 'base'

/** A line as it is drawn: its number on its own side, its code coloured. */
export interface Cell {
  num: number
  html: string
}

export interface GridRow {
  kind: 'context' | 'head' | 'code'
  /** Which conflict it belongs to, or -1 for the context between them. */
  conflict: number
  top: number
  height: number
  /** What each side has on this row; null where that side has run out. */
  ours: Cell | null
  theirs: Cell | null
  base: Cell | null
  /** Where the line sits in its side of the region, for the line checkboxes. */
  ourAt: number
  theirAt: number
}

/** Where one conflict sits in the grid, so it can be scrolled to and marked. */
export interface Spot {
  index: number
  top: number
  height: number
  /** The row the head sits on, and one past the region's last line. */
  first: number
  last: number
}

export interface Grid {
  rows: GridRow[]
  height: number
  spots: Spot[]
}

/** One side of the file as a plain list of lines, to be coloured in one pass. */
export function sideLines(blocks: ConflictBlock[], side: Side): string[] {
  const out: string[] = []
  for (const block of blocks) {
    out.push(...(block.kind === 'context' ? block.lines : block[side]))
  }
  return out
}

const empty = (): Cell | null => null

/**
 * Lays the blocks out into rows, padding the short side of every region.
 *
 * `painted` holds each side coloured as one piece, in the same order
 * `sideLines` produced — the two walk together, which is the only thing that
 * keeps a colour on the line it belongs to.
 */
export function buildGrid(
  blocks: ConflictBlock[],
  painted: { ours: string[]; theirs: string[]; base: string[] },
  withBase = false
): Grid {
  const rows: GridRow[] = []
  const spots: Spot[] = []
  const cursor = { ours: 0, theirs: 0, base: 0 }
  const number = { ours: 0, theirs: 0, base: 0 }
  let top = 0

  const cell = (side: Side): Cell => ({
    num: ++number[side],
    html: painted[side][cursor[side]++] ?? ''
  })

  for (const block of blocks) {
    if (block.kind === 'context') {
      for (const _ of block.lines) {
        rows.push({
          kind: 'context',
          conflict: -1,
          top,
          height: ROW,
          ours: cell('ours'),
          theirs: cell('theirs'),
          base: withBase ? cell('base') : empty(),
          ourAt: -1,
          theirAt: -1
        })
        top += ROW
      }
      continue
    }

    const first = rows.length
    const at = top
    rows.push({
      kind: 'head',
      conflict: block.index,
      top,
      height: HEAD,
      ours: null,
      theirs: null,
      base: null,
      ourAt: -1,
      theirAt: -1
    })
    top += HEAD

    const tall = Math.max(
      block.ours.length,
      block.theirs.length,
      withBase && block.has_base ? block.base.length : 0
    )
    for (let line = 0; line < tall; line++) {
      rows.push({
        kind: 'code',
        conflict: block.index,
        top,
        height: ROW,
        ours: line < block.ours.length ? cell('ours') : empty(),
        theirs: line < block.theirs.length ? cell('theirs') : empty(),
        base: withBase && line < block.base.length ? cell('base') : empty(),
        ourAt: line < block.ours.length ? line : -1,
        theirAt: line < block.theirs.length ? line : -1
      })
      top += ROW
    }

    spots.push({ index: block.index, top: at, height: top - at, first, last: rows.length })
  }

  return { rows, height: top, spots }
}

/**
 * Which rows are worth drawing, found by search rather than by division.
 *
 * The rows are not all the same height — a region's head is taller than a line
 * — so where one sits is not its index times anything, and the first row on
 * screen is a binary search away.
 */
export function rowWindow(rows: GridRow[], top: number, view: number, overscan = 14) {
  if (!rows.length) return { first: 0, last: 0 }
  const height = view || ROW * 40
  let low = 0
  let high = rows.length - 1
  while (low < high) {
    const mid = (low + high) >> 1
    if (rows[mid]!.top + rows[mid]!.height <= top) low = mid + 1
    else high = mid
  }
  let last = low
  while (last < rows.length && rows[last]!.top < top + height) last++
  return {
    first: Math.max(0, low - overscan),
    last: Math.min(rows.length, last + overscan)
  }
}

/** What the user has decided about one region, and what they typed over it. */
export interface Pick {
  /** One flag per line of that side: whether it goes into the result. */
  ours: boolean[]
  theirs: boolean[]
  /** When both sides are taken whole, which is written first. */
  ours_first: boolean
  /** An AI or hand-written replacement, which wins over the flags. */
  custom: string[] | null
  /** Whether anyone has looked at this region, for the progress count. */
  touched: boolean
}

/** How a region reads at a glance, which is what its colour and chip say. */
export type Stance = 'ours' | 'theirs' | 'both' | 'mixed' | 'dropped' | 'edited'

/**
 * Where a region starts before anyone has answered it.
 *
 * "Keep ours" for anything both sides wrote, which is the same answer as
 * reading the file top to bottom. A region our side has no lines in is not that
 * case: taking ours there means dropping what the other side added, and a
 * default that quietly deletes code is the wrong one — so an addition starts
 * kept, and still counts as undecided until it has been looked at.
 */
export function freshPick(block: Extract<ConflictBlock, { kind: 'conflict' }>): Pick {
  const added = block.ours.length === 0 && block.theirs.length > 0
  return {
    ours: block.ours.map(() => true),
    theirs: block.theirs.map(() => added),
    ours_first: true,
    custom: null,
    touched: false
  }
}

const whole = (flags: boolean[]) => flags.length > 0 && flags.every(Boolean)
const none = (flags: boolean[]) => flags.every((flag) => !flag)

export function stanceOf(pick: Pick): Stance {
  if (pick.custom) return 'edited'
  const ours = { all: whole(pick.ours), nothing: none(pick.ours) }
  const theirs = { all: whole(pick.theirs), nothing: none(pick.theirs) }
  if (ours.nothing && theirs.nothing) return 'dropped'
  if (ours.all && theirs.all) return 'both'
  if (ours.all && theirs.nothing) return 'ours'
  if (theirs.all && ours.nothing) return 'theirs'
  return 'mixed'
}

/**
 * The region turned into something the backend can render.
 *
 * A side taken or dropped whole is sent as the checkbox it looks like, so the
 * file is written by the same path it always was. A region where only some
 * lines were picked has no such shorthand, so it is sent as an explicit list —
 * which is the same door the AI answers come through.
 */
export function resolutionOf(
  pick: Pick,
  block: Extract<ConflictBlock, { kind: 'conflict' }>
): Resolution {
  if (pick.custom) {
    return { take_ours: true, take_theirs: true, ours_first: pick.ours_first, custom: pick.custom }
  }
  const stance = stanceOf(pick)
  if (stance !== 'mixed') {
    return {
      take_ours: whole(pick.ours),
      take_theirs: whole(pick.theirs),
      ours_first: pick.ours_first,
      custom: null
    }
  }
  const ours = block.ours.filter((_, at) => pick.ours[at])
  const theirs = block.theirs.filter((_, at) => pick.theirs[at])
  return {
    take_ours: true,
    take_theirs: true,
    ours_first: pick.ours_first,
    custom: pick.ours_first ? [...ours, ...theirs] : [...theirs, ...ours]
  }
}

/** How many lines the region puts in the file, for the result-pane mapping. */
export function pickedLines(
  pick: Pick,
  block: Extract<ConflictBlock, { kind: 'conflict' }>
): number {
  const choice = resolutionOf(pick, block)
  if (choice.custom) return choice.custom.length
  return (
    (choice.take_ours ? block.ours.length : 0) + (choice.take_theirs ? block.theirs.length : 0)
  )
}

/** Where one line of the result came from, for the marks down its side. */
export interface Origin {
  conflict: number
  from: 'ours' | 'theirs' | 'edit'
}

/**
 * Which lines of the result are the resolution, and which side each came from.
 *
 * The result pane is the one place the file can be read as it will be written,
 * and until now every line in it looked the same — so the lines that are the
 * whole point of the exercise sat unmarked among a few thousand that were never
 * in question. Walked exactly as the backend walks it, so the two agree line
 * for line; anything that falls outside a region is context and gets no mark.
 */
export function resultOrigins(
  blocks: ConflictBlock[],
  pickOf: (index: number) => Pick | undefined
): (Origin | null)[] {
  const out: (Origin | null)[] = []
  for (const block of blocks) {
    if (block.kind === 'context') {
      for (const _ of block.lines) out.push(null)
      continue
    }
    const pick = pickOf(block.index)
    if (!pick) {
      for (const _ of block.ours) out.push({ conflict: block.index, from: 'ours' })
      continue
    }
    const choice = resolutionOf(pick, block)
    if (choice.custom) {
      if (stanceOf(pick) === 'edited') {
        // A hand or AI edit is neither side; it says so.
        for (const _ of choice.custom) out.push({ conflict: block.index, from: 'edit' })
        continue
      }
      // A region picked line by line is sent as a list too, and those lines did
      // come from a side: as many as were ticked on each, in the order asked for.
      const counted = {
        ours: pick.ours.filter(Boolean).length,
        theirs: pick.theirs.filter(Boolean).length
      }
      const order: Origin['from'][] = choice.ours_first ? ['ours', 'theirs'] : ['theirs', 'ours']
      for (const side of order) {
        for (let at = 0; at < counted[side === 'ours' ? 'ours' : 'theirs']; at++) {
          out.push({ conflict: block.index, from: side })
        }
      }
      continue
    }
    const sides: Origin['from'][] = choice.ours_first ? ['ours', 'theirs'] : ['theirs', 'ours']
    for (const side of sides) {
      const taken = side === 'ours' ? choice.take_ours : choice.take_theirs
      if (!taken) continue
      for (const _ of side === 'ours' ? block.ours : block.theirs) {
        out.push({ conflict: block.index, from: side })
      }
    }
  }
  return out
}

/** The bars beside the result: one per region, in fractions of its height. */
export function originMarks(
  origins: (Origin | null)[],
  kindOf: (index: number) => Mark['kind']
): Mark[] {
  const total = origins.length
  if (!total) return []
  const marks: Mark[] = []
  let at = 0
  while (at < total) {
    const origin = origins[at]
    if (!origin) {
      at++
      continue
    }
    let end = at
    while (end < total && origins[end]?.conflict === origin.conflict) end++
    marks.push({
      kind: kindOf(origin.conflict),
      top: at / total,
      height: (end - at) / total
    })
    at = end
  }
  return marks
}

/** The bars beside the scrollbar: one per conflict, coloured by its stance. */
export function conflictMarks(
  spots: Spot[],
  height: number,
  kindOf: (index: number) => Mark['kind']
): Mark[] {
  if (!height) return []
  return spots.map((spot) => ({
    kind: kindOf(spot.index),
    top: spot.top / height,
    height: spot.height / height
  }))
}

/**
 * Where a place in the panes sits in the result, and the other way round.
 *
 * The result is a different length from either side — that is the point of it —
 * so the two cannot be scrolled to the same offset. They can be pinned to each
 * other at every block boundary, which is where both agree about what line is
 * what, and read off proportionally in between.
 */
export interface Anchor {
  from: number
  to: number
}

export function resultAnchors(
  blocks: ConflictBlock[],
  grid: Grid,
  rows: (index: number) => number
): { anchors: Anchor[]; height: number } {
  const anchors: Anchor[] = [{ from: 0, to: 0 }]
  // How tall each region actually came out in the grid, which is not the same
  // as the taller of the two sides once the base pane is showing.
  const tall = new Map(grid.spots.map((spot) => [spot.index, spot.height]))
  let from = 0
  let to = 0
  for (const block of blocks) {
    if (block.kind === 'context') {
      from += block.lines.length * ROW
      to += block.lines.length * ROW
    } else {
      from += tall.get(block.index) ?? HEAD + Math.max(block.ours.length, block.theirs.length) * ROW
      to += rows(block.index) * ROW
    }
    anchors.push({ from, to })
  }
  return { anchors, height: to }
}

/** Reads one side of the anchors off against the other, linearly between them. */
export function mapTop(anchors: Anchor[], value: number, back = false): number {
  const key = back ? 'to' : 'from'
  const out = back ? 'from' : 'to'
  if (anchors.length < 2) return value
  let at = 0
  while (at < anchors.length - 2 && anchors[at + 1]![key] <= value) at++
  const start = anchors[at]!
  const end = anchors[at + 1]!
  const span = end[key] - start[key]
  const reach = end[out] - start[out]
  if (span <= 0) return start[out]
  return start[out] + ((value - start[key]) / span) * reach
}
