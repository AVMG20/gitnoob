import { describe, expect, it } from 'vitest'
import { stepFile, walkOrder, type FileEntry, type FileStep } from '../app/composables/useFileView'

const file = (path: string): FileEntry => ({ path, kind: 'modified' })

const NONE = new Set<string>()

describe('walkOrder', () => {
  it('lists a flat view in the order it was given', () => {
    const order = walkOrder([{ files: [file('b.ts'), file('a.ts')] }], 'path', NONE)
    expect(order.map((one) => one.path)).toEqual(['b.ts', 'a.ts'])
  })

  it('lists a tree in the order the tree draws it', () => {
    const order = walkOrder(
      [{ files: [file('z.ts'), file('src/b.ts'), file('src/a.ts')] }],
      'tree',
      NONE
    )
    // Directories first and sorted, files within them sorted, loose files last.
    expect(order.map((one) => one.path)).toEqual(['src/a.ts', 'src/b.ts', 'z.ts'])
  })

  it('leaves out the files inside a folded directory', () => {
    const order = walkOrder(
      [{ files: [file('src/a.ts'), file('z.ts')] }],
      'tree',
      new Set(['src'])
    )
    expect(order.map((one) => one.path)).toEqual(['z.ts'])
  })

  it('runs the groups together, each carrying its own side', () => {
    const order = walkOrder(
      [
        { files: [file('a.ts')], side: 'unstaged' },
        { files: [file('b.ts')], side: 'staged' }
      ],
      'path',
      NONE
    )
    expect(order).toEqual([
      { path: 'a.ts', side: 'unstaged' },
      { path: 'b.ts', side: 'staged' }
    ])
  })

  it('tells the same path apart on either side', () => {
    const order = walkOrder(
      [
        { files: [file('a.ts')], side: 'unstaged' },
        { files: [file('a.ts')], side: 'staged' }
      ],
      'path',
      NONE
    )
    expect(stepFile(order, { path: 'a.ts', side: 'unstaged' }, 1)).toEqual({
      path: 'a.ts',
      side: 'staged'
    })
  })
})

describe('stepFile', () => {
  const order: FileStep[] = [{ path: 'a.ts' }, { path: 'b.ts' }, { path: 'c.ts' }]

  it('moves one either way', () => {
    expect(stepFile(order, { path: 'b.ts' }, 1)).toEqual({ path: 'c.ts' })
    expect(stepFile(order, { path: 'b.ts' }, -1)).toEqual({ path: 'a.ts' })
  })

  it('stops at the ends rather than wrapping', () => {
    expect(stepFile(order, { path: 'c.ts' }, 1)).toBeNull()
    expect(stepFile(order, { path: 'a.ts' }, -1)).toBeNull()
  })

  it('starts at the end the key was heading away from when nothing is open', () => {
    expect(stepFile(order, null, 1)).toEqual({ path: 'a.ts' })
    expect(stepFile(order, null, -1)).toEqual({ path: 'c.ts' })
  })

  it('starts over when the open file has left the list', () => {
    expect(stepFile(order, { path: 'gone.ts' }, 1)).toEqual({ path: 'a.ts' })
  })

  it('has nowhere to go in an empty list', () => {
    expect(stepFile([], { path: 'a.ts' }, 1)).toBeNull()
    expect(stepFile([], null, 1)).toBeNull()
  })
})
