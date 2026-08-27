import { describe, expect, it } from 'vitest'
import { parsePatch } from '../app/composables/usePatch'

describe('parsePatch', () => {
  it('reads a hunk with its header and numbers both sides', () => {
    const { hunks } = parsePatch(
      ['@@ -3,4 +3,5 @@ fn main', ' context', '-old line', '+new line', '+another', ' tail'].join('\n')
    )
    expect(hunks).toHaveLength(1)
    expect(hunks[0]!.header).toBe('@@ -3,4 +3,5 @@ fn main')
    const lines = hunks[0]!.lines
    expect(lines.map((line) => line.origin)).toEqual([' ', '-', '+', '+', ' '])
    // Context starts both counters; deletions step only the old, additions
    // only the new.
    expect(lines.map((line) => [line.old_lineno, line.new_lineno])).toEqual([
      [3, 3],
      [4, null],
      [null, 4],
      [null, 5],
      [5, 6]
    ])
  })

  it('keeps several hunks apart, each counting from its own header', () => {
    const { hunks } = parsePatch(
      ['@@ -1,2 +1,2 @@', ' a', '-b', '+B', '@@ -20,2 +20,2 @@', ' x', '-y', '+Y'].join('\n')
    )
    expect(hunks).toHaveLength(2)
    expect(hunks[1]!.lines[0]!.old_lineno).toBe(20)
    expect(hunks[1]!.lines[0]!.new_lineno).toBe(20)
  })

  it('keeps the no-newline remark as a remark, not as a line', () => {
    const { hunks } = parsePatch(['@@ -1,2 +1,2 @@', ' a', '-b', '\\ No newline at end of file', '+B'].join('\n'))
    const lines = hunks[0]!.lines
    expect(lines.map((line) => line.origin)).toEqual([' ', '-', '\\', '+'])

    // It belongs to neither file, so it is numbered in neither and takes no
    // number from the line after it.
    const remark = lines[2]!
    expect(remark.old_lineno).toBeNull()
    expect(remark.new_lineno).toBeNull()
    expect(remark.content).toBe('No newline at end of file')
    expect(lines[3]!.new_lineno).toBe(2)
  })

  it('drops stray file headers a truncated answer may leave behind', () => {
    const { hunks } = parsePatch(['diff --git a/x b/x', 'index 123..456 100644', '--- a/x', '+++ b/x', '@@ -1 +1 @@', '-a', '+b'].join('\n'))
    expect(hunks).toHaveLength(1)
    expect(hunks[0]!.lines.map((line) => line.content)).toEqual(['a', 'b'])
  })

  it('says nothing about an empty or missing patch', () => {
    expect(parsePatch('')).toEqual({ hunks: [], truncated: 0 })
    expect(parsePatch('   \n  ').hunks).toHaveLength(0)
  })

  it('reads a header without counts as starting at that line', () => {
    const { hunks } = parsePatch(['@@ -7 +7 @@', '-gone', '+here'].join('\n'))
    expect(hunks[0]!.lines[0]!.old_lineno).toBe(7)
    expect(hunks[0]!.lines[1]!.new_lineno).toBe(7)
  })
})
