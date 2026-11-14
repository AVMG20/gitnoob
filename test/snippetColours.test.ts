import { describe, expect, it, vi } from 'vitest'
import { tags } from '@lezer/highlight'

vi.mock('~/composables/useCodeMirror', async (original) => ({
  ...(await original<typeof import('~/composables/useCodeMirror')>()),
  syntaxSpecs: () => [
    { tag: [tags.variableName, tags.special(tags.variableName)], color: '#a00' },
    { tag: [tags.keyword, tags.controlKeyword], color: '#00b' },
    { tag: [tags.number, tags.integer], color: '#0c0' }
  ]
}))

const { colourLines, runsOf, snippetTick, resetSnippetColours } = await import('~/composables/snippetColours')

const settle = async () => {
  const before = snippetTick.value
  for (let i = 0; i < 50 && snippetTick.value === before; i++) await new Promise((done) => setTimeout(done, 10))
}

describe('colouring search result lines', () => {
  it('is plain until the language has loaded, then coloured in it', async () => {
    resetSnippetColours()
    const lines = ['$total = 0;', 'return $total;']
    expect(colourLines('app/Cart.php', lines)).toBeNull()
    await settle()
    const coloured = colourLines('app/Cart.php', lines)!
    // A lone PHP line, with no `<?php` before it, is still read as PHP.
    const styleOf = (row: number, text: string) => {
      const at = lines[row]!.indexOf(text)
      return coloured[row]!.find(([from, to]) => from <= at && to >= at + text.length)?.[2]
    }
    expect(styleOf(0, '$total')).toBe('color:#a00')
    expect(styleOf(0, '0')).toBe('color:#0c0')
    expect(styleOf(1, 'return')).toBe('color:#00b')
    // The same lines again are the same answer, not a second parse.
    expect(colourLines('app/Cart.php', lines)).toBe(coloured)
  })

  it('stays plain for a file with no language', async () => {
    resetSnippetColours()
    expect(colourLines('notes.unknownext', ['x'])).toBeNull()
    await new Promise((done) => setTimeout(done, 20))
    expect(colourLines('notes.unknownext', ['x'])).toBeNull()
  })

  it('keeps each match whole over the colours', () => {
    const runs = runsOf('return $total;', [[0, 6, 'k'], [7, 13, 'v']], [[3, 10]])
    expect(runs.map((run) => [run.hit, run.parts.map((part) => [part.text, part.style])])).toEqual([
      [false, [['ret', 'k']]],
      [true, [['urn', 'k'], [' ', null], ['$to', 'v']]],
      [false, [['tal', 'v'], [';', null]]]
    ])
    expect(runsOf('plain', null, []).map((run) => run.parts[0]!.text)).toEqual(['plain'])
  })
})
