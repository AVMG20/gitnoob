// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import {
  SHORTCUTS,
  clashFor,
  isRebindable,
  keysFor,
  matchesShortcut,
  setShortcutOverrides,
  useShortcuts
} from '~/composables/useShortcuts'

const isMac = navigator.userAgent.includes('Mac')

function press(key: string, mods: { shift?: boolean; alt?: boolean } = {}) {
  const event = new KeyboardEvent('keydown', {
    key,
    metaKey: isMac,
    ctrlKey: !isMac,
    shiftKey: !!mods.shift,
    altKey: !!mods.alt,
    bubbles: true,
    cancelable: true
  })
  window.dispatchEvent(event)
  return event
}

function bind(bindings: Record<string, () => void>) {
  return mount(
    defineComponent({
      setup() {
        useShortcuts(bindings)
        return () => h('div')
      }
    })
  )
}

afterEach(() => setShortcutOverrides({}))

describe('moving a shortcut', () => {
  it('dispatches on the new keys and not the old ones', () => {
    const stage = vi.fn()
    const wrapper = bind({ 'work.stageall': stage })
    press('a', { shift: true })
    expect(stage).toHaveBeenCalledTimes(1)

    setShortcutOverrides({ 'work.stageall': 'mod+alt+s' })
    expect(keysFor('work.stageall')).toBe('mod+alt+s')
    press('a', { shift: true })
    expect(stage).toHaveBeenCalledTimes(1)
    press('s', { alt: true })
    expect(stage).toHaveBeenCalledTimes(2)
    wrapper.unmount()
  })

  it('moves a key a component handles itself, like ⌘↵ in the commit box', () => {
    const enter = new KeyboardEvent('keydown', { key: 'Enter', metaKey: isMac, ctrlKey: !isMac })
    expect(matchesShortcut(enter, 'commit.write')).toBe(true)
    setShortcutOverrides({ 'commit.write': 'mod+j' })
    expect(matchesShortcut(enter, 'commit.write')).toBe(false)
  })

  it('ignores a moved key for a shortcut that does not exist', () => {
    setShortcutOverrides({ 'no.such': 'mod+q' })
    expect(keysFor('no.such')).toBe('')
  })

  it('hands a key back to the handler underneath when the one on top goes', () => {
    const below = vi.fn()
    const above = vi.fn()
    const first = bind({ 'graph.search': below })
    const second = bind({ 'viewer.search': above })
    press('f')
    expect(above).toHaveBeenCalledTimes(1)
    second.unmount()
    press('f')
    expect(below).toHaveBeenCalledTimes(1)
    first.unmount()
  })
})

describe('the keys that ship', () => {
  it('only offers chords to move, not the arrows and clicks the lists own', () => {
    const ids = SHORTCUTS.filter(isRebindable).map((one) => one.id)
    expect(ids).toContain('work.discard')
    expect(ids).toContain('commit.push')
    expect(ids).not.toContain('graph.move')
    expect(ids).not.toContain('project.nth')
    expect(ids).not.toContain('graph.extend')
  })

  it('gives no two shortcuts the same keys in the same place', () => {
    const anywhere = 'Anywhere in the window'
    const chords = SHORTCUTS.filter(isRebindable)
    for (const one of chords) {
      for (const other of chords) {
        if (one === other) continue
        const clash =
          one.keys.toLowerCase() === other.keys.toLowerCase() &&
          (one.where === other.where || one.where === anywhere || other.where === anywhere)
        // ⌘F, ⌘G and ⇧⌘G are the commit list's and an open file's in turn,
        // and the two are never on screen together.
        const knownPair = [one.id, other.id].every((id) => /^(graph|viewer)\./.test(id))
        expect(clash && !knownPair, `${one.id} and ${other.id}`).toBe(false)
      }
    }
  })
})

describe('moving a key onto one already taken', () => {
  const row = (id: string) => SHORTCUTS.find((one) => one.id === id)!
  afterEach(() => setShortcutOverrides({}))

  it('sees the working-tree places as one, since they are on screen together', () => {
    expect(clashFor(row('commit.push'), 'mod+enter')?.id).toBe('commit.write')
    expect(clashFor(row('work.stageall'), 'mod+s')?.id).toBe('work.toggle')
  })

  it('counts the project tabs’ ⌘1…9 as taken', () => {
    expect(clashFor(row('repo.fetch'), 'mod+3')?.id).toBe('project.nth')
  })

  it('lets a key go somewhere that is never live at the same time', () => {
    expect(clashFor(row('work.stageall'), 'mod+shift+y')).toBeUndefined()
  })
})
