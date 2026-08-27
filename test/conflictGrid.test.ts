import { describe, expect, it } from 'vitest'
import {
  HEAD,
  ROW,
  buildGrid,
  conflictMarks,
  freshPick,
  mapTop,
  pickedLines,
  resolutionOf,
  resultAnchors,
  rowWindow,
  sideLines,
  stanceOf,
  type Pick
} from '../app/composables/useConflictGrid'
import type { ConflictBlock } from '../app/composables/useGit'

const context = (...lines: string[]): ConflictBlock => ({ kind: 'context', lines })

const conflict = (
  index: number,
  ours: string[],
  theirs: string[],
  base: string[] = []
): Extract<ConflictBlock, { kind: 'conflict' }> => ({
  kind: 'conflict',
  index,
  ours,
  theirs,
  base,
  has_base: base.length > 0,
  ours_label: 'HEAD',
  theirs_label: 'feature'
})

/** Each side coloured is stood in for by the lines themselves. */
const paint = (blocks: ConflictBlock[]) => ({
  ours: sideLines(blocks, 'ours'),
  theirs: sideLines(blocks, 'theirs'),
  base: sideLines(blocks, 'base')
})

describe('sideLines', () => {
  it('reads one side of the file, context included', () => {
    const blocks = [context('a'), conflict(0, ['ours'], ['theirs', 'more']), context('b')]
    expect(sideLines(blocks, 'ours')).toEqual(['a', 'ours', 'b'])
    expect(sideLines(blocks, 'theirs')).toEqual(['a', 'theirs', 'more', 'b'])
  })
})

describe('buildGrid', () => {
  const blocks = [context('one'), conflict(0, ['mine'], ['yours', 'and yours']), context('two')]
  const grid = buildGrid(blocks, paint(blocks))

  it('gives every side the same rows, so the panes cannot drift', () => {
    // context, head, two region rows, context.
    expect(grid.rows).toHaveLength(5)
    expect(grid.rows.map((row) => row.kind)).toEqual([
      'context',
      'head',
      'code',
      'code',
      'context'
    ])
  })

  it('leaves a hole where a side has run out rather than closing the gap', () => {
    const short = grid.rows[3]!
    expect(short.theirs?.html).toBe('and yours')
    expect(short.ours).toBeNull()
    expect(short.ourAt).toBe(-1)
  })

  it('numbers each side as that side of the file, not as the grid', () => {
    // Our side is one, mine, two; theirs is one line longer through the region.
    expect(grid.rows.map((row) => row.ours?.num ?? null)).toEqual([1, null, 2, null, 3])
    expect(grid.rows.map((row) => row.theirs?.num ?? null)).toEqual([1, null, 2, 3, 4])
  })

  it('measures where each conflict sits, for the strip and the arrows', () => {
    expect(grid.spots).toHaveLength(1)
    expect(grid.spots[0]).toMatchObject({ index: 0, top: ROW, height: HEAD + 2 * ROW })
    expect(grid.height).toBe(ROW + HEAD + 2 * ROW + ROW)
  })

  it('only lays out the base side when it is asked for', () => {
    const withBase = [conflict(0, ['a'], ['b'], ['before'])]
    const off = buildGrid(withBase, paint(withBase))
    expect(off.rows[1]!.base).toBeNull()
    const on = buildGrid(withBase, paint(withBase), true)
    expect(on.rows[1]!.base?.html).toBe('before')
  })
})

describe('rowWindow', () => {
  const blocks = [context(...Array.from({ length: 400 }, (_, at) => `line ${at}`))]
  const grid = buildGrid(blocks, paint(blocks))

  it('covers the rows on screen and a margin either side', () => {
    const { first, last } = rowWindow(grid.rows, 100 * ROW, 10 * ROW, 5)
    expect(first).toBe(95)
    expect(last).toBe(115)
  })

  it('never runs past either end', () => {
    expect(rowWindow(grid.rows, 0, 10 * ROW).first).toBe(0)
    expect(rowWindow(grid.rows, 0, 10000 * ROW).last).toBe(400)
    expect(rowWindow([], 0, 100)).toEqual({ first: 0, last: 0 })
  })

  it('draws something before the box has been measured', () => {
    expect(rowWindow(grid.rows, 0, 0).last).toBeGreaterThan(0)
  })
})

describe('what a region has been decided to be', () => {
  const block = conflict(0, ['a', 'b'], ['c', 'd'])

  it('starts on our side, the same answer as reading the file top to bottom', () => {
    const pick = freshPick(block)
    expect(stanceOf(pick)).toBe('ours')
    expect(pick.touched).toBe(false)
    expect(resolutionOf(pick, block)).toEqual({
      take_ours: true,
      take_theirs: false,
      ours_first: true,
      custom: null
    })
  })

  it('sends whole sides as the checkboxes they look like', () => {
    const both: Pick = { ours: [true, true], theirs: [true, true], ours_first: false, custom: null, touched: true }
    expect(stanceOf(both)).toBe('both')
    expect(resolutionOf(both, block)).toEqual({
      take_ours: true,
      take_theirs: true,
      ours_first: false,
      custom: null
    })

    const gone: Pick = { ours: [false, false], theirs: [false, false], ours_first: true, custom: null, touched: true }
    expect(stanceOf(gone)).toBe('dropped')
    expect(pickedLines(gone, block)).toBe(0)
  })

  it('spells out a region picked line by line, in the order asked for', () => {
    const some: Pick = { ours: [true, false], theirs: [false, true], ours_first: true, custom: null, touched: true }
    expect(stanceOf(some)).toBe('mixed')
    expect(resolutionOf(some, block).custom).toEqual(['a', 'd'])
    expect(pickedLines(some, block)).toBe(2)

    const flipped: Pick = { ...some, ours_first: false }
    expect(resolutionOf(flipped, block).custom).toEqual(['d', 'a'])
  })

  it('lets an edit win over the lines, and counts as its own stance', () => {
    const edited: Pick = { ours: [true, false], theirs: [false, false], ours_first: true, custom: ['merged'], touched: true }
    expect(stanceOf(edited)).toBe('edited')
    expect(resolutionOf(edited, block).custom).toEqual(['merged'])
    expect(pickedLines(edited, block)).toBe(1)
  })

  it('keeps what one side added rather than dropping it by default', () => {
    const added = conflict(0, [], ['new one'])
    const pick = freshPick(added)
    expect(pick.ours).toEqual([])
    expect(stanceOf(pick)).toBe('theirs')
    expect(pick.touched).toBe(false)
    expect(resolutionOf(pick, added)).toMatchObject({ take_ours: false, take_theirs: true })
  })

  it('reads an emptied region as dropped', () => {
    const added = conflict(0, [], ['new one'])
    const gone: Pick = { ...freshPick(added), theirs: [false], touched: true }
    expect(stanceOf(gone)).toBe('dropped')
    expect(pickedLines(gone, added)).toBe(0)
  })
})

describe('conflictMarks', () => {
  it('places one bar per conflict, in fractions of the file', () => {
    const blocks = [conflict(0, ['a'], ['b']), context('x'), conflict(1, ['c'], ['d'])]
    const grid = buildGrid(blocks, paint(blocks))
    const marks = conflictMarks(grid.spots, grid.height, (index) =>
      index === 0 ? 'settled' : 'open'
    )
    expect(marks).toHaveLength(2)
    expect(marks[0]!.kind).toBe('settled')
    expect(marks[0]!.top).toBe(0)
    expect(marks[1]!.top).toBeCloseTo((HEAD + ROW + ROW) / grid.height)
    expect(conflictMarks(grid.spots, 0, () => 'open')).toEqual([])
  })
})

describe('pinning the result to the panes', () => {
  // Our side keeps one line where theirs has three, and the region is answered
  // with theirs — so the result runs three lines where the panes run four rows
  // of region plus a head.
  const blocks = [context('top'), conflict(0, ['mine'], ['a', 'b', 'c']), context('end')]
  const grid = buildGrid(blocks, paint(blocks))
  const { anchors, height } = resultAnchors(blocks, grid, () => 3)

  it('agrees with both at every block boundary', () => {
    expect(anchors[0]).toEqual({ from: 0, to: 0 })
    expect(anchors[1]).toEqual({ from: ROW, to: ROW })
    expect(anchors[2]).toEqual({ from: ROW + HEAD + 3 * ROW, to: ROW + 3 * ROW })
    expect(height).toBe(ROW + 3 * ROW + ROW)
  })

  it('reads one off against the other, and back again', () => {
    expect(mapTop(anchors, 0)).toBe(0)
    expect(mapTop(anchors, ROW)).toBe(ROW)
    expect(mapTop(anchors, anchors[2]!.from)).toBe(anchors[2]!.to)
    // Halfway through the region lands halfway through what it produced.
    const half = ROW + (HEAD + 3 * ROW) / 2
    expect(mapTop(anchors, half)).toBeCloseTo(ROW + (3 * ROW) / 2)
    expect(mapTop(anchors, mapTop(anchors, half), true)).toBeCloseTo(half)
  })

  it('answers before there is anything to answer with', () => {
    expect(mapTop([], 40)).toBe(40)
    expect(mapTop([{ from: 0, to: 0 }], 40)).toBe(40)
  })
})
