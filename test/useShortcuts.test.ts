import { describe, expect, it } from 'vitest'
import {
  SHORTCUTS,
  chordOf,
  clashFor,
  keysFor,
  reservedChord,
  setShortcutOverrides
} from '~/composables/useShortcuts'
import { TIPS } from '~/composables/useTips'

/** A keydown as the web view hands it over, without needing a DOM. */
function key(init: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    key: '',
    code: '',
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...init
  } as KeyboardEvent
}

describe('reading a chord on macOS', () => {
  const mac = true

  it('reads ⌘⌥F as mod+alt+f, though Option typed ƒ', () => {
    expect(chordOf(key({ key: 'ƒ', code: 'KeyF', metaKey: true, altKey: true }), mac)).toBe(
      'mod+alt+f'
    )
  })

  it('reads a dead key under Option by where it sits', () => {
    // ⌥E starts an accent on a Mac rather than typing anything.
    expect(chordOf(key({ key: 'Dead', code: 'KeyE', altKey: true }), mac)).toBe('alt+e')
  })

  it('reads ⌥⇧ with a digit by the digit', () => {
    expect(
      chordOf(key({ key: '›', code: 'Digit4', metaKey: true, altKey: true, shiftKey: true }), mac)
    ).toBe('mod+alt+shift+4')
  })

  it('leaves the arrows alone under Option', () => {
    expect(chordOf(key({ key: 'ArrowDown', code: 'ArrowDown', metaKey: true, altKey: true }), mac)).toBe(
      'mod+alt+ArrowDown'
    )
  })

  it('keeps a letter Option did not change', () => {
    expect(chordOf(key({ key: 'f', code: 'KeyF', metaKey: true, altKey: true }), mac)).toBe(
      'mod+alt+f'
    )
  })

  it('reads ⌘ alone without the physical key', () => {
    expect(chordOf(key({ key: '[', code: 'BracketLeft', metaKey: true }), mac)).toBe('mod+[')
  })

  it('spells Control as ctrl, which never matches ⌘', () => {
    expect(chordOf(key({ key: 'Tab', code: 'Tab', ctrlKey: true }), mac)).toBe('ctrl+Tab')
    expect(chordOf(key({ key: 'f', code: 'KeyF', ctrlKey: true }), mac)).not.toBe('mod+f')
  })
})

describe('reading a chord on Windows and Linux', () => {
  const mac = false

  it('keeps the letter the layout gives under Alt', () => {
    // On an AZERTY board the A key sits where QWERTY has Q.
    expect(chordOf(key({ key: 'a', code: 'KeyQ', ctrlKey: true, altKey: true }), mac)).toBe(
      'mod+alt+a'
    )
  })

  it('does not take a character typed with AltGr for a chord', () => {
    const altGr = { getModifierState: (name: string) => name === 'AltGraph' } as Partial<KeyboardEvent>
    // German: AltGr+Q types @. Polish: AltGr+A types ą.
    expect(chordOf(key({ key: '@', code: 'KeyQ', ctrlKey: true, altKey: true, ...altGr }), mac)).toBe('')
    expect(chordOf(key({ key: 'ą', code: 'KeyA', ctrlKey: true, altKey: true, ...altGr }), mac)).toBe('')
    // Ctrl+Alt+F held on purpose is still Fetch.
    expect(chordOf(key({ key: 'f', code: 'KeyF', ctrlKey: true, altKey: true }), mac)).toBe('mod+alt+f')
  })

  it('reads Ctrl as mod and ignores the Windows key', () => {
    expect(chordOf(key({ key: 'Tab', ctrlKey: true }), mac)).toBe('mod+Tab')
    expect(chordOf(key({ key: 'f', metaKey: true }), mac)).toBe('')
  })
})

describe('the keys that moved for 0.10', () => {
  it('gives the brackets to back and forward, and moves file stepping to the arrows', () => {
    expect(keysFor('work.next')).toBe('mod+alt+ArrowDown')
    expect(keysFor('work.previous')).toBe('mod+alt+ArrowUp')
    expect(keysFor('nav.back')).toBe('mod+[')
    expect(keysFor('nav.forward')).toBe('mod+]')
  })

  it('leaves ⌘G with the graph only', () => {
    expect(keysFor('graph.next')).toBe('mod+g')
    expect(keysFor('viewer.next')).toBe('')
    expect(keysFor('viewer.previous')).toBe('')
  })

  it('lists the new keys under their own heading', () => {
    const files = SHORTCUTS.filter((one) => one.group === 'Files and editing').map((one) => one.id)
    expect(files).toEqual(
      expect.arrayContaining(['nav.back', 'nav.forward', 'tab.next', 'tab.previous', 'file.mode', 'graph.show'])
    )
  })
})

describe('keys that cannot be taken', () => {
  const row = (id: string) => SHORTCUTS.find((one) => one.id === id)!

  it('counts a shortcut’s second chord as taken too', () => {
    const back = row('nav.back')
    // Only Windows and Linux give back a second chord.
    if (back.also) expect(clashFor(row('files.recent'), back.also)?.id).toBe('nav.back')
  })

  it('refuses the chords the macOS menu owns, and only on a Mac', () => {
    expect(reservedChord('mod+q', true)).toBe(true)
    expect(reservedChord('mod+H', true)).toBe(true)
    expect(reservedChord('mod+shift+q', true)).toBe(false)
    expect(reservedChord('mod+q', false)).toBe(false)
  })

  it('sees the clash a reset would walk into', () => {
    setShortcutOverrides({ 'files.recent': 'mod+alt+e', 'repo.fetch': 'mod+e' })
    const recent = row('files.recent')
    expect(clashFor(recent, recent.keys)?.id).toBe('repo.fetch')
    setShortcutOverrides({})
  })
})

describe('a tip about a key that was moved', () => {
  it('names the key it is on now', () => {
    const tip = TIPS.find((one) => one.id && one.keys === keysFor('project.switch'))!
    setShortcutOverrides({ 'project.switch': 'mod+shift+p' })
    expect(tip.keys).toBe('mod+shift+p')
    setShortcutOverrides({})
    expect(tip.keys).toBe(row_default('project.switch'))
  })
})

function row_default(id: string) {
  return SHORTCUTS.find((one) => one.id === id)!.keys
}
