// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import FileTree from '~/components/FileTree.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit, type RepoInfo } from '~/composables/useGit'
import { resetTabs, switchTabs, useTabs } from '~/composables/useTabs'
import { resetFileTree, revealInFiles, treeView, type DirEntry } from '~/composables/useFileTree'
import { useContextMenu, type MenuItem } from '~/composables/useContextMenu'
import { useDragDrop } from '~/composables/useDragDrop'
import { clearClip, treeClipboard } from '~/composables/useFileOps'
import { useToasts } from '~/composables/useToasts'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => true) }))

const asked = vi.mocked(invoke)
const git = useGit()
const tabs = useTabs()
const menu = useContextMenu()
const drag = useDragDrop()

const entry = (path: string, dir = false, ignored = false): DirEntry => ({
  name: path.slice(path.lastIndexOf('/') + 1),
  path,
  dir,
  ignored,
  link: false
})

const DISK: Record<string, DirEntry[]> = {
  '': [entry('app', true), entry('vendor', true, true), entry('composer.json'), entry('debug.log', false, true)],
  app: [entry('app/Billing', true), entry('app/Http', true)],
  'app/Billing': [entry('app/Billing/Money.php'), entry('app/Billing/TaxRates.php')],
  'app/Http': [entry('app/Http/Kernel.php')],
  vendor: []
}

let calls: { cmd: string; args: Record<string, unknown> }[] = []
let wrapper: ReturnType<typeof mount> | null = null

async function show() {
  wrapper = mount(FileTree, { attachTo: document.body, global: { components: { AppModal } } })
  await flushPromises()
  return wrapper
}

const row = (w: ReturnType<typeof mount>, path: string) => w.find(`.row[data-path="${path}"]`)
const listed = (w: ReturnType<typeof mount>) => w.findAll('.row').map((one) => one.attributes('data-path'))
const called = (cmd: string) => calls.filter((one) => one.cmd === cmd)

/** A menu item by its label, looking inside submenus too. */
function item(label: string, items: MenuItem[] = menu.state.items): MenuItem | undefined {
  for (const one of items) {
    if (one.label === label) return one
    const inside = one.children && item(label, one.children)
    if (inside) return inside
  }
  return undefined
}

function key(w: ReturnType<typeof mount>, key: string, extra: KeyboardEventInit = {}) {
  const list = w.find('.list').element as HTMLElement
  list.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...extra }))
}

beforeEach(() => {
  resetTabs()
  resetFileTree()
  clearClip()
  menu.close()
  drag.end()
  useToasts().clear()
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'list_dir') {
      const found = DISK[String(args?.path ?? '')]
      if (!found) throw new Error('not a folder')
      return found
    }
    if (cmd === 'create_path') return args?.path
    if (cmd === 'rename_path') return args?.to
    if (cmd === 'move_paths') {
      const from = args?.paths as string[]
      return {
        from,
        to: from.map((one) => `${args?.dest ? `${args.dest}/` : ''}${one.split('/').pop()}`),
        error: null
      }
    }
    if (cmd === 'copy_paths') return []
    if (cmd === 'trash_paths') return 'Moved composer.json to the Trash'
    if (cmd === 'add_to_gitignore') return `Added ${args?.pattern} to .gitignore`
    if (cmd === 'file_history') return []
    return null
  })
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as RepoInfo
  switchTabs('/repos/shop')
  git.store.status = {
    staged: [{ path: 'app/Billing/TaxRates.php', kind: 'added' }],
    unstaged: [
      { path: 'app/Billing/Money.php', kind: 'modified' },
      { path: 'app/Billing/Old.php', kind: 'deleted' },
      { path: 'app/Http/New.php', kind: 'untracked' }
    ],
    conflicted: []
  }
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
  git.store.status = null
})

describe('making and renaming files in the tree', () => {
  it('names a new file in a box under the folder and opens it', async () => {
    const w = await show()
    await row(w, 'app').trigger('contextmenu')
    await item('New file…')!.action!()
    await flushPromises()

    const box = w.find('input.edit')
    expect(box.exists()).toBe(true)
    // The box is the first row inside the folder, which opened for it.
    const paths = listed(w)
    expect(paths.indexOf('app')).toBeLessThan(paths.indexOf('app/Billing'))
    expect(w.find('.row.draft').exists()).toBe(true)

    await box.setValue('Http/Controllers/Home.php')
    await box.trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(called('create_path')[0]?.args).toMatchObject({ path: 'app/Http/Controllers/Home.php', dir: false })
    expect(w.find('input.edit').exists()).toBe(false)
    expect(tabs.activeTab()?.path).toBe('app/Http/Controllers/Home.php')
  })

  it('makes a folder at the root from the empty space under the rows', async () => {
    const w = await show()
    await w.find('.list').trigger('contextmenu')
    await item('New folder…')!.action!()
    await flushPromises()
    const box = w.find('input.edit')
    await box.setValue('docs')
    await box.trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(called('create_path')[0]?.args).toMatchObject({ path: 'docs', dir: true })
  })

  it('gives up on Escape, and on an empty name, without asking the backend', async () => {
    const w = await show()
    await row(w, 'app').trigger('contextmenu')
    await item('New file…')!.action!()
    await flushPromises()
    await w.find('input.edit').setValue('x.ts')
    await w.find('input.edit').trigger('keydown', { key: 'Escape' })
    await flushPromises()
    expect(w.find('input.edit').exists()).toBe(false)

    await row(w, 'app').trigger('contextmenu')
    await item('New file…')!.action!()
    await flushPromises()
    await w.find('input.edit').trigger('blur')
    await flushPromises()
    expect(called('create_path')).toHaveLength(0)
  })

  it('renames with F2, the name picked out without its extension', async () => {
    const w = await show()
    await row(w, 'composer.json').trigger('click')
    key(w, 'F2')
    await flushPromises()
    const box = w.find('input.edit').element as HTMLInputElement
    expect(box.value).toBe('composer.json')
    expect([box.selectionStart, box.selectionEnd]).toEqual([0, 'composer'.length])

    await w.find('input.edit').setValue('package.json')
    await w.find('input.edit').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(called('rename_path')[0]?.args).toMatchObject({ from: 'composer.json', to: 'package.json' })
  })

  it('keeps the box with what was typed when the backend refuses the name', async () => {
    asked.mockImplementationOnce(async () => DISK['']!)
    const w = await show()
    const fallback = asked.getMockImplementation()!
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'rename_path') {
        calls.push({ cmd, args: args ?? {} })
        throw new Error('app is already there')
      }
      return fallback(cmd, args)
    })
    await row(w, 'composer.json').trigger('contextmenu')
    await item('Rename…')!.action!()
    await flushPromises()
    await w.find('input.edit').setValue('app')
    await w.find('input.edit').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(called('rename_path')).toHaveLength(1)
    expect((w.find('input.edit').element as HTMLInputElement).value).toBe('app')
  })
})

describe('moving, copying and deleting', () => {
  it('asks before a delete, sends it to the Trash and offers an undo', async () => {
    const w = await show()
    await row(w, 'composer.json').trigger('contextmenu')
    await item('Delete…')!.action!()
    await flushPromises()
    expect(called('trash_paths')).toHaveLength(0)
    expect(w.text()).toContain('Trash')
    await w.find('.btn-danger').trigger('click')
    await flushPromises()
    expect(called('trash_paths')[0]?.args).toMatchObject({ paths: ['composer.json'] })
    expect(useToasts().items.value.some((one) => one.title.includes('Trash'))).toBe(true)
  })

  it('cuts and pastes with the keyboard, which moves', async () => {
    const w = await show()
    await row(w, 'composer.json').trigger('click')
    key(w, 'x', { metaKey: true })
    expect(treeClipboard).toMatchObject({ mode: 'cut', paths: ['composer.json'] })
    await flushPromises()
    expect(row(w, 'composer.json').classes()).toContain('cut')

    await row(w, 'app').trigger('click', { detail: 1 })
    await flushPromises()
    key(w, 'v', { metaKey: true })
    await flushPromises()
    expect(called('move_paths')[0]?.args).toMatchObject({ paths: ['composer.json'], dest: 'app' })
    expect(treeClipboard.mode).toBe(null)
  })

  it('moves a file dragged onto a folder, and copies it with ⌥', async () => {
    const w = await show()
    await row(w, 'composer.json').trigger('dragstart')
    await row(w, 'app').trigger('dragover')
    expect(row(w, 'app').classes()).toContain('drop')
    await row(w, 'app').trigger('drop')
    await flushPromises()
    expect(called('move_paths')[0]?.args).toMatchObject({ paths: ['composer.json'], dest: 'app' })

    await row(w, 'composer.json').trigger('dragstart')
    await row(w, 'app').trigger('dragover', { altKey: true })
    await row(w, 'app').trigger('drop', { altKey: true })
    await flushPromises()
    expect(called('copy_paths')[0]?.args).toMatchObject({ paths: ['composer.json'], dest: 'app' })
  })

  it('does not take a folder dropped into itself', async () => {
    const w = await show()
    await row(w, 'app').trigger('click', { detail: 1 })
    await flushPromises()
    await row(w, 'app').trigger('dragstart')
    await row(w, 'app/Billing').trigger('dragover')
    expect(row(w, 'app/Billing').classes()).not.toContain('drop')
    await row(w, 'app/Billing').trigger('drop')
    await flushPromises()
    expect(called('move_paths')).toHaveLength(0)
  })

  it('does nothing when a row is dropped on itself or a sibling, rather than moving it to the root', async () => {
    const w = await show()
    revealInFiles('app/Billing/Money.php')
    await flushPromises()
    await row(w, 'app/Billing/Money.php').trigger('dragstart')
    await row(w, 'app/Billing/Money.php').trigger('dragover')
    expect(w.find('.list').classes()).not.toContain('drop-root')
    await row(w, 'app/Billing/TaxRates.php').trigger('dragover')
    await row(w, 'app/Billing/TaxRates.php').trigger('drop')
    await flushPromises()
    expect(called('move_paths')).toHaveLength(0)
    expect(called('copy_paths')).toHaveLength(0)
  })

  it('says why a move stopped and keeps what did not move where it was', async () => {
    const w = await show()
    const fallback = asked.getMockImplementation()!
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'move_paths') {
        calls.push({ cmd, args: args ?? {} })
        return { from: [], to: [], error: 'app/composer.json is already there' }
      }
      return fallback(cmd, args)
    })
    await row(w, 'composer.json').trigger('dragstart')
    await row(w, 'app').trigger('dragover')
    await row(w, 'app').trigger('drop')
    await flushPromises()
    expect(called('move_paths')).toHaveLength(1)
    expect(useToasts().items.value.some((one) => one.level === 'error')).toBe(true)
  })

  it('duplicates into the same folder', async () => {
    const w = await show()
    await row(w, 'composer.json').trigger('contextmenu')
    await item('Duplicate')!.action!()
    await flushPromises()
    expect(called('copy_paths')[0]?.args).toMatchObject({ paths: ['composer.json'], dest: '' })
  })
})

describe('git on files and folders', () => {
  it('ignores a file anchored at its path, or only on this machine', async () => {
    const w = await show()
    await row(w, 'composer.json').trigger('contextmenu')
    expect(item('Add to .gitignore')!.children!.map((one) => one.hint)).toEqual(
      expect.arrayContaining(['/composer.json', '*.json'])
    )
    await item('This file (composer.json)')!.action!()
    await flushPromises()
    expect(called('add_to_gitignore')[0]?.args).toMatchObject({ pattern: '/composer.json', local: false })

    await row(w, 'composer.json').trigger('contextmenu')
    await item('Only on this machine')!.action!()
    await flushPromises()
    expect(called('add_to_gitignore')[1]?.args).toMatchObject({ pattern: '/composer.json', local: true })
  })

  it('offers no ignore rule for what git already ignores', async () => {
    const w = await show()
    await row(w, 'debug.log').trigger('contextmenu')
    expect(item('Add to .gitignore')).toBeUndefined()
    expect(item('Stop tracking')).toBeUndefined()
  })

  it('stops tracking a tracked file and not an untracked one', async () => {
    const w = await show()
    await row(w, 'composer.json').trigger('contextmenu')
    await item('Stop tracking')!.action!()
    await flushPromises()
    expect(called('untrack_paths')[0]?.args).toMatchObject({ paths: ['composer.json'] })

    DISK['app/Http'] = [entry('app/Http/Kernel.php'), entry('app/Http/New.php')]
    try {
      revealInFiles('app/Http/New.php')
      await flushPromises()
      await row(w, 'app/Http/New.php').trigger('contextmenu')
      expect(item('Stop tracking')).toBeUndefined()
    } finally {
      DISK['app/Http'] = [entry('app/Http/Kernel.php')]
    }
  })

  it('stages, unstages and discards what is inside a folder', async () => {
    const w = await show()
    await row(w, 'app').trigger('contextmenu')
    await item('Stage 3 files')!.action!()
    await flushPromises()
    expect(called('stage')[0]?.args.paths).toEqual([
      'app/Billing/Money.php',
      'app/Billing/Old.php',
      'app/Http/New.php'
    ])

    await row(w, 'app').trigger('contextmenu')
    await item('Unstage 1 file')!.action!()
    await flushPromises()
    expect(called('unstage')[0]?.args.paths).toEqual(['app/Billing/TaxRates.php'])

    await row(w, 'app').trigger('contextmenu')
    await item('Discard changes…')!.action!()
    await flushPromises()
    await w.find('.btn-danger').trigger('click')
    await flushPromises()
    // The new file is left alone: discarding never deletes.
    expect(called('discard')[0]?.args.paths).toEqual(['app/Billing/Money.php', 'app/Billing/Old.php'])
  })

  it('reads a folder history without following one file', async () => {
    const w = await show()
    await row(w, 'app').trigger('contextmenu')
    await item('Folder history')!.action!()
    await flushPromises()
    expect(called('file_history')[0]?.args).toMatchObject({ path: 'app', folder: true })
  })
})

describe('how the tree is drawn', () => {
  it('counts the changed files in a folder', async () => {
    const w = await show()
    expect(row(w, 'app').find('.count').text()).toBe('4')
    expect(row(w, 'composer.json').find('.count').exists()).toBe(false)
  })

  it('rings the row whose menu is open', async () => {
    const w = await show()
    await row(w, 'composer.json').trigger('contextmenu')
    expect(row(w, 'composer.json').classes()).toContain('menu-on')
    menu.close()
    await flushPromises()
    expect(row(w, 'composer.json').classes()).not.toContain('menu-on')
  })

  it('hides what git ignores when asked', async () => {
    const w = await show()
    expect(listed(w)).toContain('debug.log')
    await w.find('button[aria-label="Show ignored files"]').trigger('click')
    await flushPromises()
    expect(listed(w)).not.toContain('debug.log')
    expect(listed(w)).not.toContain('vendor')
    expect(treeView.showIgnored).toBe(false)
  })

  it('shows only changed files, folders open, without reading the disk', async () => {
    const w = await show()
    const before = called('list_dir').length
    await w.find('button[aria-label="Changed files only"]').trigger('click')
    await flushPromises()
    expect(listed(w)).toEqual([
      'app',
      'app/Billing',
      'app/Billing/Money.php',
      'app/Billing/Old.php',
      'app/Billing/TaxRates.php',
      'app/Http',
      'app/Http/New.php'
    ])
    expect(called('list_dir').length).toBe(before)

    await row(w, 'app/Billing').trigger('click', { detail: 1 })
    await flushPromises()
    expect(listed(w)).not.toContain('app/Billing/Money.php')
  })
})
