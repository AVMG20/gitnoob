import { describe, expect, it } from 'vitest'
import { keysHere } from '~/composables/useShortcuts'

describe('Mac key glyphs in sentences', () => {
  it('read as Ctrl, Shift and Alt off the Mac', () => {
    expect(keysHere('Go to line (⌘G)', false)).toBe('Go to line (Ctrl+G)')
    expect(keysHere('Previous (↑ or ⇧⌘G)', false)).toBe('Previous (↑ or Shift+Ctrl+G)')
    expect(keysHere('Same as ⌘-click on a name.', false)).toBe('Same as Ctrl-click on a name.')
    expect(keysHere('mod+click marks one more on its own.', false)).toBe('Ctrl-click marks one more on its own.')
  })

  it('stay as they are on the Mac, apart from the raw mod token', () => {
    expect(keysHere('Go to line (⌘G)', true)).toBe('Go to line (⌘G)')
    expect(keysHere('mod+click marks one more', true)).toBe('⌘-click marks one more')
  })
})
