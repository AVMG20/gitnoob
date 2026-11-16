// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import FileTree from '~/components/FileTree.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit, type RepoInfo } from '~/composables/useGit'
import { resetTabs, switchTabs, useTabs } from '~/composables/useTabs'
import { resetFileTree, type DirEntry } from '~/composables/useFileTree'
import { speedMatch } from '~/composables/speedSearch'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

/**
 * The Files tree from the keyboard: walking, folding and opening, and the
 * speed search that typing over it starts.
 */

const asked = vi.mocked(invoke)
const git = useGit()
const tabs = useTabs()

const entry = (path: string, dir = false): DirEntry => ({
  name: path.slice(path.lastIndexOf('/') + 1),
  path,
  dir,
  ignored: false,
  link: false
})

const DISK: Record<string, DirEntry[]> = {
  '': [entry('app', true), entry('docs', true), entry('big.txt'), entry('readme.md')],
  app: [entry('app/BigBridge.php'), entry('app/AccountLoggedIn.php'), entry('app/Ambig.php')],
  docs: [entry('docs/guide.md')]
}

let wrapper: ReturnType<typeof mount> | null = null
const reachedWindow: string[] = []
const onWindow = (event: KeyboardEvent) => reachedWindow.push(event.key)

beforeEach(() => {
  resetTabs()
  resetFileTree()
  reachedWindow.length = 0
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'list_dir') return DISK[String(args?.path ?? '')] ?? []
    return null
  })
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as RepoInfo
  switchTabs('/repos/shop')
  git.store.status = { staged: [], unstaged: [], conflicted: [] }
  window.addEventListener('keydown', onWindow)
})

afterEach(() => {
  window.removeEventListener('keydown', onWindow)
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
  git.store.status = null
})

async function show() {
  wrapper = mount(FileTree, { attachTo: document.body, global: { components: { AppModal } } })
  await flushPromises()
  const list = wrapper.find('.list').element as HTMLElement
  list.focus()
  list.dispatchEvent(new FocusEvent('focus'))
  await flushPromises()
  return wrapper
}

async function press(w: ReturnType<typeof mount>, key: string) {
  const list = w.find('.list').element
  list.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  await flushPromises()
}

async function type(w: ReturnType<typeof mount>, text: string) {
  for (const key of text) await press(w, key)
}

const selectedPath = (w: ReturnType<typeof mount>) => w.find('.row.selected').attributes('data-path')
const listed = (w: ReturnType<typeof mount>) => w.findAll('.row').map((one) => one.attributes('data-path'))

describe('the files tree from the keyboard', () => {
  it('walks with ↑ ↓, opens a folder with →, steps in, and goes back out with ←', async () => {
    const w = await show()
    expect(selectedPath(w)).toBe('app')
    await press(w, 'ArrowDown')
    expect(selectedPath(w)).toBe('docs')
    await press(w, 'ArrowUp')
    expect(selectedPath(w)).toBe('app')
    // → opens it, → again steps into it.
    await press(w, 'ArrowRight')
    expect(listed(w)).toContain('app/BigBridge.php')
    expect(selectedPath(w)).toBe('app')
    await press(w, 'ArrowRight')
    expect(selectedPath(w)).toBe('app/BigBridge.php')
    // ← on a file goes to its folder; ← on an open folder shuts it.
    await press(w, 'ArrowLeft')
    expect(selectedPath(w)).toBe('app')
    await press(w, 'ArrowLeft')
    expect(listed(w)).not.toContain('app/BigBridge.php')
    await press(w, 'End')
    expect(selectedPath(w)).toBe('readme.md')
    await press(w, 'Home')
    expect(selectedPath(w)).toBe('app')
    // None of it reached the window: the viewer's ↑ ↓ never heard it.
    expect(reachedWindow).toEqual([])
  })

  it('opens a file pinned on ↵ and as a preview on Space, and folds a folder on ↵', async () => {
    const w = await show()
    await press(w, 'End')
    await press(w, ' ')
    expect(tabs.activeTab()).toMatchObject({ path: 'readme.md', preview: true })
    await press(w, 'ArrowUp')
    await press(w, 'Enter')
    expect(tabs.activeTab()).toMatchObject({ path: 'big.txt', preview: false })
    await press(w, 'Home')
    await press(w, 'Enter')
    expect(listed(w)).toContain('app/Ambig.php')
  })

  it('takes a click as the keyboard’s row too', async () => {
    const w = await show()
    await w.find('.row[data-path="docs"]').trigger('click')
    expect(selectedPath(w)).toBe('docs')
  })
})

describe('speed search in the files tree', () => {
  it('marks every row on screen that has what was typed, and goes to the best', async () => {
    const w = await show()
    await press(w, 'ArrowRight')
    await type(w, 'big')
    expect(w.find('.speed').text()).toBe('big')
    const marked = w.findAll('.row.matched').map((one) => one.attributes('data-path'))
    expect(marked).toEqual(['app/BigBridge.php', 'app/Ambig.php', 'big.txt'])
    expect(w.find('.row[data-path="app/Ambig.php"] mark.speed-hit').text()).toBe('big')
    // A name that starts with it beats one that only has it.
    expect(selectedPath(w)).toBe('app/BigBridge.php')
    // Not dimmed, nothing narrowed: every row is still there.
    expect(listed(w)).toContain('readme.md')
    // ↑ ↓ go from match to match only, round the end.
    await press(w, 'ArrowDown')
    expect(selectedPath(w)).toBe('app/Ambig.php')
    await press(w, 'ArrowDown')
    expect(selectedPath(w)).toBe('big.txt')
    await press(w, 'ArrowDown')
    expect(selectedPath(w)).toBe('app/BigBridge.php')
    await press(w, 'ArrowUp')
    expect(selectedPath(w)).toBe('big.txt')
    // Typing reached nothing else in the window.
    expect(reachedWindow).toEqual([])
  })

  it('matches word starts and camel humps', async () => {
    expect(speedMatch('BigBridge.php', 'bb')).toMatchObject({ tier: 2, indices: [0, 3] })
    expect(speedMatch('AccountLoggedIn.php', 'acl')).toMatchObject({ tier: 2, indices: [0, 1, 7] })
    expect(speedMatch('readme.md', 'md')).toMatchObject({ tier: 2 })
    expect(speedMatch('Ambig.php', 'big')).toMatchObject({ tier: 1 })
    expect(speedMatch('big.txt', 'big')).toMatchObject({ tier: 3 })
    expect(speedMatch('guide.md', 'xyz')).toBeNull()
    const w = await show()
    await press(w, 'ArrowRight')
    await type(w, 'acl')
    expect(selectedPath(w)).toBe('app/AccountLoggedIn.php')
  })

  it('clears on Esc, and leaves the next Esc to the window', async () => {
    const w = await show()
    await type(w, 'doc')
    expect(selectedPath(w)).toBe('docs')
    await press(w, 'Backspace')
    expect(w.find('.speed').text()).toBe('do')
    await press(w, 'Escape')
    expect(w.find('.speed').exists()).toBe(false)
    expect(reachedWindow).toEqual([])
    await press(w, 'Escape')
    expect(reachedWindow).toEqual(['Escape'])
  })

  it('turns red on a letter nothing has, and stays where it was', async () => {
    const w = await show()
    await type(w, 'rea')
    expect(selectedPath(w)).toBe('readme.md')
    await press(w, 'q')
    expect(w.find('.speed').classes()).toContain('missed')
    expect(w.find('.speed').text()).toBe('reaq')
    expect(selectedPath(w)).toBe('readme.md')
  })

  it('ends on ↵, opening what it is on, and on losing the focus', async () => {
    const w = await show()
    await type(w, 'big.')
    await press(w, 'Enter')
    expect(tabs.activeTab()).toMatchObject({ path: 'big.txt', preview: false })
    expect(w.find('.speed').exists()).toBe(false)
    await type(w, 'rea')
    w.find('.list').element.dispatchEvent(new FocusEvent('blur'))
    await flushPromises()
    expect(w.find('.speed').exists()).toBe(false)
  })

  it('leaves the filter box’s typing alone', async () => {
    const w = await show()
    const box = w.find('.filter input').element
    box.dispatchEvent(new KeyboardEvent('keydown', { key: 'b', bubbles: true, cancelable: true }))
    await flushPromises()
    expect(w.find('.speed').exists()).toBe(false)
  })
})
