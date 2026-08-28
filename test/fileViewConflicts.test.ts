import { describe, expect, it } from 'vitest'
import { buildRows, CONFLICTED } from '~/composables/useFileView'

const files = [
  { path: 'app/store.ts', kind: CONFLICTED },
  { path: 'app/main.ts', kind: 'modified' },
  { path: 'docs/readme.md', kind: 'added' }
]

describe('a conflict inside the file tree', () => {
  it('counts on the folders above it, so a folded one still says so', () => {
    const rows = buildRows(files, 'tree', new Set(['app']))
    const app = rows.find((row) => row.kind === 'dir' && row.path === 'app')
    expect(app?.conflicts).toBe(1)

    const docs = rows.find((row) => row.kind === 'dir' && row.path === 'docs')
    expect(docs?.conflicts).toBe(0)
  })

  it('is not counted as an edit — it is not one yet', () => {
    const rows = buildRows(files, 'tree', new Set(['app']))
    const app = rows.find((row) => row.kind === 'dir' && row.path === 'app')
    expect(app?.tally).toEqual({ added: 0, modified: 1, deleted: 0, renamed: 0 })
  })

  it('keeps its own kind on the file row, in either mode', () => {
    for (const mode of ['tree', 'path'] as const) {
      const rows = buildRows(files, mode, new Set())
      const file = rows.find((row) => row.path === 'app/store.ts')
      expect(file?.entry?.kind).toBe(CONFLICTED)
    }
  })
})
