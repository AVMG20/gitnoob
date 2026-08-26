import { describe, expect, it } from 'vitest'
import {
  CODE_ROW,
  HUNK_HEAD,
  OVERSCAN,
  diffRows,
  diffWindow,
  fileMarks,
  firstChangedLine,
  markedLines,
  patchMarks,
  windowOf
} from '../app/composables/useCode'
import type { DiffHunk, DiffLine } from '../app/composables/useGit'

const ctx = (old: number, now: number, content = 'ctx'): DiffLine => ({
  origin: ' ',
  old_lineno: old,
  new_lineno: now,
  content
})
const add = (now: number, content = 'new'): DiffLine => ({
  origin: '+',
  old_lineno: null,
  new_lineno: now,
  content
})
const del = (old: number, content = 'old'): DiffLine => ({
  origin: '-',
  old_lineno: old,
  new_lineno: null,
  content
})
const hunk = (...lines: DiffLine[]): DiffHunk => ({ header: '@@', lines })

describe('windowOf', () => {
  it('covers the rows on screen and a margin either side', () => {
    const { first, last } = windowOf(1000, 100 * CODE_ROW, 10 * CODE_ROW)
    expect(first).toBe(100 - OVERSCAN)
    expect(last).toBe(110 + OVERSCAN)
  })

  it('never runs past either end of the list', () => {
    expect(windowOf(1000, 0, 10 * CODE_ROW).first).toBe(0)
    expect(windowOf(20, 0, 1000 * CODE_ROW).last).toBe(20)
  })

  it('draws something before the box has been measured', () => {
    // Nothing drawn is nothing to scroll, and nothing to scroll never asks again.
    expect(windowOf(1000, 0, 0).last).toBeGreaterThan(0)
  })

  it('has no window over an empty file', () => {
    expect(windowOf(0, 0, 500)).toEqual({ first: 0, last: 0 })
  })
})

describe('diffRows', () => {
  it('puts a heading before each hunk and stacks the rows', () => {
    const { rows, height } = diffRows([hunk(ctx(1, 1), add(2)), hunk(del(9))])
    expect(rows.map((row) => row.kind)).toEqual(['head', 'line', 'line', 'head', 'line'])
    expect(rows[0]!.top).toBe(0)
    expect(rows[1]!.top).toBe(HUNK_HEAD)
    expect(rows[2]!.top).toBe(HUNK_HEAD + CODE_ROW)
    expect(rows[3]!.top).toBe(HUNK_HEAD + CODE_ROW * 2)
    expect(height).toBe(HUNK_HEAD * 2 + CODE_ROW * 3)
  })

  it('keeps each row pointing at the hunk it can be staged from', () => {
    const { rows } = diffRows([hunk(add(1)), hunk(add(2))])
    expect(rows.map((row) => row.hunk)).toEqual([0, 0, 1, 1])
  })
})

describe('diffWindow', () => {
  const { rows } = diffRows([hunk(...Array.from({ length: 200 }, (_, i) => add(i + 1)))])

  it('finds the rows at a scroll position', () => {
    const { first, last } = diffWindow(rows, 50 * CODE_ROW, 10 * CODE_ROW)
    expect(rows[first]!.top).toBeLessThanOrEqual(50 * CODE_ROW)
    expect(rows[last - 1]!.top).toBeGreaterThanOrEqual(60 * CODE_ROW)
  })

  it('starts at the top when the view is at the top', () => {
    expect(diffWindow(rows, 0, 10 * CODE_ROW).first).toBe(0)
  })

  it('has no window over an empty patch', () => {
    expect(diffWindow([], 0, 500)).toEqual({ first: 0, last: 0 })
  })
})

describe('firstChangedLine', () => {
  it('finds an addition', () => {
    expect(firstChangedLine({ path: 'a', binary: false, truncated: 0, hunks: [hunk(ctx(1, 1), add(2))] })).toBe(2)
  })

  it('sends a deletion to the seam it left behind', () => {
    expect(
      firstChangedLine({
        path: 'a',
        binary: false,
        truncated: 0,
        hunks: [hunk(ctx(1, 1), del(2), del(3), ctx(4, 2))]
      })
    ).toBe(2)
  })

  it('has nothing to say about a patch with no changes', () => {
    expect(firstChangedLine({ path: 'a', binary: false, truncated: 0, hunks: [] })).toBeNull()
    expect(firstChangedLine(null)).toBeNull()
  })
})

describe('markedLines', () => {
  it('calls a line that replaced one changed, and a fresh one added', () => {
    const lines = markedLines('a\nb\nc', [hunk(ctx(1, 1), del(2), add(2), add(3))])
    expect(lines.map((line) => line.mark)).toEqual([null, 'changed', 'added'])
    expect(lines[1]!.was).toEqual(['old'])
  })

  it('marks the seam where lines went and nothing replaced them', () => {
    const lines = markedLines('a\nb', [hunk(ctx(1, 1), del(2), ctx(3, 2))])
    expect(lines[1]!.removed).toEqual(['old'])
    expect(lines[1]!.mark).toBeNull()
  })

  it('does not count the empty piece after a trailing newline as a line', () => {
    expect(markedLines('a\nb\n', []).length).toBe(2)
  })

  it('has no lines without the file', () => {
    expect(markedLines(null, [])).toEqual([])
  })
})

describe('marks', () => {
  it('folds a run of changed lines into one bar', () => {
    const lines = markedLines(
      'a\nb\nc\nd',
      [hunk(ctx(1, 1), add(2), add(3), ctx(4, 4))]
    )
    const marks = fileMarks(lines)
    expect(marks).toHaveLength(1)
    expect(marks[0]!.kind).toBe('added')
    expect(marks[0]!.height).toBeCloseTo(2 / 4)
  })

  it('measures a patch against the height of the whole patch', () => {
    const { rows, height } = diffRows([hunk(ctx(1, 1), add(2))])
    const marks = patchMarks(rows, height)
    expect(marks).toHaveLength(1)
    expect(marks[0]!.top).toBeCloseTo((HUNK_HEAD + CODE_ROW) / height)
  })

  it('has nothing to draw for a file with no changes', () => {
    expect(fileMarks(markedLines('a\nb', []))).toEqual([])
    expect(patchMarks([], 0)).toEqual([])
  })
})
