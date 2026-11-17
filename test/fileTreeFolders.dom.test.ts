// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import FileTree from '~/components/FileTree.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit, type RepoInfo } from '~/composables/useGit'
import { resetTabs, switchTabs } from '~/composables/useTabs'
import { resetFileTree, treeSession, type DirEntry } from '~/composables/useFileTree'
import { useContextMenu } from '~/composables/useContextMenu'
import { folderColor, resetFolderColors } from '~/composables/useFolderColors'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

/**
 * Folders in the Files tree: shutting a folder shuts what is inside it, a
 * single path of folders down to one file opens in one go, "Open all inside"
 * and `*` open everything below, and a folder can be given a colour.
 */

const asked = vi.mocked(invoke)
const git = useGit()
const menu = useContextMenu()

const entry = (path: string, dir = false, ignored = false): DirEntry => ({
  name: path.slice(path.lastIndexOf('/') + 1),
  path,
  dir,
  ignored,
  link: false
})

const DISK: Record<string, DirEntry[]> = {
  '': [entry('Setup', true), entry('view', true), entry('etc', true), entry('registration.php')],
  Setup: [entry('Setup/Patch', true)],
  'Setup/Patch': [entry('Setup/Patch/Data', true)],
  'Setup/Patch/Data': [entry('Setup/Patch/Data/AddLocationAttributes.php')],
  view: [entry('view/frontend', true), entry('view/cache', true, true)],
  'view/frontend': [entry('view/frontend/layout', true), entry('view/frontend/templates', true)],
  'view/frontend/layout': [entry('view/frontend/layout/catalog_product_view.xml')],
  'view/frontend/templates': [entry('view/frontend/templates/locations.phtml')],
  'view/cache': [entry('view/cache/one.bin')],
  etc: [entry('etc/sub', true)],
  'etc/sub': [entry('etc/sub/a.xml'), entry('etc/sub/b.xml')]
}

let wrapper: ReturnType<typeof mount> | null = null

async function show() {
  wrapper = mount(FileTree, { attachTo: document.body, global: { components: { AppModal } } })
  await flushPromises()
  return wrapper
}

const row = (w: ReturnType<typeof mount>, path: string) => w.find(`.row[data-path="${path}"]`)
const listed = (w: ReturnType<typeof mount>) =>
  w.findAll('.row').map((one) => one.attributes('data-path'))

async function click(w: ReturnType<typeof mount>, path: string) {
  await row(w, path).trigger('click')
  await flushPromises()
}

async function press(w: ReturnType<typeof mount>, key: string) {
  const list = w.find('.list').element
  list.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
  await flushPromises()
}

beforeEach(() => {
  resetTabs()
  resetFileTree()
  resetFolderColors()
  menu.close()
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'list_dir') {
      const found = DISK[String(args?.path ?? '')]
      if (!found) throw new Error('not a folder')
      return found
    }
    return null
  })
  git.store.repo = { path: '/repos/module', name: 'module', head: 'main' } as RepoInfo
  switchTabs('/repos/module')
  git.store.status = { staged: [], unstaged: [], conflicted: [] }
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
  git.store.status = null
})

describe('folders in the files tree', () => {
  it('opens a single path of folders all the way to its one file', async () => {
    const w = await show()
    await click(w, 'Setup')
    expect(listed(w)).toContain('Setup/Patch/Data/AddLocationAttributes.php')
    expect(Object.keys(treeSession().open).sort()).toEqual(['Setup', 'Setup/Patch', 'Setup/Patch/Data'])
  })

  it('stops where a folder holds more than one thing', async () => {
    const w = await show()
    await click(w, 'view')
    expect(listed(w)).toContain('view/frontend')
    expect(listed(w)).not.toContain('view/frontend/layout')
    // A path of folders that ends in two files is not opened either.
    await click(w, 'etc')
    expect(listed(w)).toContain('etc/sub')
    expect(listed(w)).not.toContain('etc/sub/a.xml')
  })

  it('shuts every folder inside one that is shut', async () => {
    const w = await show()
    await click(w, 'Setup')
    await click(w, 'Setup')
    expect(treeSession().open).toEqual({})
    // Opened again, it opens down the path afresh rather than remembering.
    await click(w, 'view')
    await click(w, 'view/frontend')
    await click(w, 'view')
    expect(treeSession().open).toEqual({})
    await click(w, 'view')
    expect(listed(w)).not.toContain('view/frontend/layout')
  })

  it('opens everything inside from the menu, leaving ignored folders shut', async () => {
    const w = await show()
    await row(w, 'view').trigger('contextmenu')
    await menu.state.items.find((one) => one.label === 'Open all inside')!.action!()
    await flushPromises()
    expect(listed(w)).toContain('view/frontend/layout/catalog_product_view.xml')
    expect(listed(w)).toContain('view/frontend/templates/locations.phtml')
    expect(listed(w)).toContain('view/cache')
    expect(listed(w)).not.toContain('view/cache/one.bin')

    await row(w, 'view').trigger('contextmenu')
    await menu.state.items.find((one) => one.label === 'Close all inside')!.action!()
    await flushPromises()
    expect(treeSession().open).toEqual({})
  })

  it('opens everything inside the selected folder on *', async () => {
    const w = await show()
    await click(w, 'view')
    await click(w, 'view')
    expect(treeSession().open).toEqual({})
    await press(w, '*')
    expect(listed(w)).toContain('view/frontend/templates/locations.phtml')
    // And none of it went to the speed search.
    expect(w.find('.speed').exists()).toBe(false)
  })

  it('gives a folder a colour from its menu and takes it off again', async () => {
    const w = await show()
    await row(w, 'etc').trigger('contextmenu')
    const colours = menu.state.items.find((one) => one.label === 'Colour')!.children!
    await colours.find((one) => one.label === 'Blue')!.action!()
    await flushPromises()
    expect(folderColor('/repos/module', 'etc')?.id).toBe('blue')
    expect(row(w, 'etc').classes()).toContain('tinted')
    expect(row(w, 'etc').attributes('style')).toContain('--folder-color')
    expect(JSON.parse(localStorage.getItem('gitnoob.folderColors') ?? '{}')).toEqual({
      '/repos/module': { etc: 'blue' }
    })

    await row(w, 'etc').trigger('contextmenu')
    const again = menu.state.items.find((one) => one.label === 'Colour')!.children!
    expect(again.find((one) => one.label === 'Blue')!.hint).toBe('✓')
    await again.find((one) => one.label === 'No colour')!.action!()
    await flushPromises()
    expect(row(w, 'etc').classes()).not.toContain('tinted')
    expect(folderColor('/repos/module', 'etc')).toBeNull()
  })
})
