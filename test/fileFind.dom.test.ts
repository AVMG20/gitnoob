// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import DiffViewer from '~/components/DiffViewer.vue'
import FileView from '~/components/FileView.vue'
import DiffView from '~/components/DiffView.vue'
import ChangeRuler from '~/components/ChangeRuler.vue'
import { useGit, type WorkingStatus } from '~/composables/useGit'
import { diffMode } from '~/composables/useDiffMode'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)

const TEXT = 'a store\nnothing here\nstore and Store\n'

const DIFF = {
  path: 'app/Retrier.php',
  binary: false,
  truncated: 0,
  hunks: [
    {
      header: '@@ -1,1 +1,1 @@',
      lines: [
        { origin: '-', old_lineno: 1, new_lineno: null, content: 'a stash' },
        { origin: '+', old_lineno: null, new_lineno: 1, content: 'a store' }
      ]
    }
  ]
}

const status: WorkingStatus = {
  staged: [],
  unstaged: [{ path: 'app/Retrier.php', status: 'modified', staged: false }] as never,
  conflicted: []
}

const git = useGit()
const mod = navigator.userAgent.includes('Mac') ? { metaKey: true } : { ctrlKey: true }

let wrapper: ReturnType<typeof mount> | null = null

async function show(mode: 'file' | 'diff') {
  diffMode.mode = mode
  diffMode.blame = false
  wrapper = mount(DiffViewer, {
    attachTo: document.body,
    global: { components: { FileView, DiffView, ChangeRuler } }
  })
  await flushPromises()
  return wrapper
}

async function press(key: string, extra: KeyboardEventInit = {}, on: EventTarget = window) {
  on.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...extra }))
  await flushPromises()
}

async function search(w: ReturnType<typeof mount>, words: string) {
  await w.find('.find input').setValue(words)
  await flushPromises()
}

const count = (w: ReturnType<typeof mount>) => w.find('.find .count').text()

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'working_file_diff') return DIFF
    if (cmd === 'file_text') return TEXT
    return null
  })
  git.store.status = status
  git.store.viewer = { path: 'app/Retrier.php', side: 'unstaged' }
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('finding text in an open file', () => {
  it('opens a box on the shortcut and marks every match in the file', async () => {
    const w = await show('file')
    expect(w.find('.find').exists()).toBe(false)

    await press('f', mod)
    expect(w.find('.find').exists()).toBe(true)
    expect(document.activeElement).toBe(w.find('.find input').element)

    await search(w, 'store')
    expect(count(w)).toBe('1 of 3')
    expect(w.findAll('mark.find-hit')).toHaveLength(3)
    expect(w.findAll('mark.find-hit.now')).toHaveLength(1)
  })

  it('walks the matches with the arrows and Enter, round the ends', async () => {
    const w = await show('file')
    await press('f', mod)
    await search(w, 'store')
    const input = w.find('.find input').element

    await press('ArrowDown', {}, input)
    expect(count(w)).toBe('2 of 3')
    // The second match is the first one on the third line.
    const lines = w.findAll('.line').filter((one) => !one.classes('gauge'))
    expect(lines[2]!.find('mark.find-hit.now').exists()).toBe(true)

    await press('Enter', {}, input)
    expect(count(w)).toBe('3 of 3')
    await press('ArrowDown', {}, input)
    expect(count(w)).toBe('1 of 3')
    await press('ArrowUp', {}, input)
    expect(count(w)).toBe('3 of 3')
    await press('Enter', { shiftKey: true }, input)
    expect(count(w)).toBe('2 of 3')
  })

  it('leaves the arrows on the box rather than moving to another file', async () => {
    const w = await show('file')
    await press('f', mod)
    await search(w, 'store')
    await press('ArrowDown', {}, w.find('.find input').element)
    expect(git.store.viewer?.path).toBe('app/Retrier.php')
  })

  it('matches case only when asked to', async () => {
    const w = await show('file')
    await press('f', mod)
    await search(w, 'Store')
    expect(count(w)).toBe('1 of 3')
    await w.find('.find .step').trigger('click')
    await flushPromises()
    expect(count(w)).toBe('1 of 1')
  })

  it('says so when nothing matches', async () => {
    const w = await show('file')
    await press('f', mod)
    await search(w, 'missing')
    expect(count(w)).toBe('no matches')
    expect(w.findAll('mark.find-hit')).toHaveLength(0)
  })

  it('searches the patch in the diff view', async () => {
    const w = await show('diff')
    await press('f', mod)
    await search(w, 'st')
    // "a stash" and "a store": both sides of the change are lines of the patch.
    expect(count(w)).toBe('1 of 2')
    expect(w.findAll('mark.find-hit')).toHaveLength(2)
  })

  it('closes the box on Escape before it closes the file', async () => {
    const w = await show('file')
    await press('f', mod)
    await search(w, 'store')

    await press('Escape', {}, w.find('.find input').element)
    expect(w.find('.find').exists()).toBe(false)
    expect(w.findAll('mark.find-hit')).toHaveLength(0)
    expect(git.store.viewer).not.toBeNull()

    await press('Escape')
    expect(git.store.viewer).toBeNull()
  })

  it('stays on a real match when a reload leaves fewer of them', async () => {
    const w = await show('file')
    await press('f', mod)
    await search(w, 'store')
    const input = w.find('.find input').element
    await press('ArrowUp', {}, input)
    expect(count(w)).toBe('3 of 3')

    // A save outside the window: the file now has one match.
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'working_file_diff') return DIFF
      if (cmd === 'file_text') return 'a store\n'
      return null
    })
    git.store.status = { ...status }
    await flushPromises()
    expect(count(w)).toBe('1 of 1')
  })

  it('keeps the words for the next time it is opened', async () => {
    const w = await show('file')
    await press('f', mod)
    await search(w, 'store')
    await press('Escape')
    await press('f', mod)
    expect((w.find('.find input').element as HTMLInputElement).value).toBe('store')
    expect(count(w)).toBe('1 of 3')
  })
})
