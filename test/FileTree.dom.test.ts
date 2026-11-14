// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import FileTree from '~/components/FileTree.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit, type RepoInfo } from '~/composables/useGit'
import { openTab, resetTabs, switchTabs, tabId, useTabs } from '~/composables/useTabs'
import { resetFileTree, revealInFiles, treeSession, type DirEntry } from '~/composables/useFileTree'
import { useContextMenu } from '~/composables/useContextMenu'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const asked = vi.mocked(invoke)
const git = useGit()
const tabs = useTabs()
const menu = useContextMenu()

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
  'app/Billing': [
    entry('app/Billing/InvoiceService.php'),
    entry('app/Billing/Money.php'),
    entry('app/Billing/TaxRates.php')
  ],
  'app/Http': [entry('app/Http/Kernel.php')],
  vendor: [entry('vendor/autoload.php', false, true)]
}

let calls: { cmd: string; args: Record<string, unknown> }[] = []
let wrapper: ReturnType<typeof mount> | null = null

async function show() {
  wrapper = mount(FileTree, { attachTo: document.body, global: { components: { AppModal } } })
  await flushPromises()
  return wrapper
}

const row = (w: ReturnType<typeof mount>, path: string) => w.find(`.row[data-path="${path}"]`)
const listed = (w: ReturnType<typeof mount>) =>
  w.findAll('.row').map((one) => one.attributes('data-path'))
const readDirs = () => calls.filter((one) => one.cmd === 'list_dir').map((one) => one.args.path)

beforeEach(() => {
  resetTabs()
  resetFileTree()
  menu.close()
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'list_dir') {
      const found = DISK[String(args?.path ?? '')]
      if (!found) throw new Error('not a folder')
      return found
    }
    if (cmd === 'find_files') {
      return [
        { path: 'app/Billing/InvoiceService.php', score: 90, indices: [12, 13, 14, 19, 20] },
        { path: 'app/Providers/InvoiceServiceProvider.php', score: 70, indices: [14] }
      ]
    }
    return null
  })
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as RepoInfo
  switchTabs('/repos/shop')
  git.store.status = {
    staged: [
      { path: 'app/Billing/TaxRates.php', kind: 'added' },
      { path: 'app/Billing/Money.php', kind: 'modified' }
    ],
    unstaged: [
      { path: 'app/Billing/InvoiceService.php', kind: 'modified' },
      { path: 'app/Billing/Money.php', kind: 'modified' },
      { path: 'app/Billing/LegacyTax.php', kind: 'deleted' }
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

describe('the files tree', () => {
  it('reads only the top folder to start, folders first', async () => {
    const w = await show()
    expect(readDirs()).toEqual([''])
    expect(listed(w)).toEqual(['app', 'vendor', 'composer.json', 'debug.log'])
  })

  it('dims what git ignores and does not read an ignored folder until it is opened', async () => {
    const w = await show()
    expect(row(w, 'vendor').classes()).toContain('ignored')
    expect(row(w, 'vendor').text()).toContain('ignored')
    expect(row(w, 'debug.log').classes()).toContain('ignored')
    expect(row(w, 'app').classes()).not.toContain('ignored')
    expect(readDirs()).not.toContain('vendor')

    await row(w, 'vendor').trigger('click')
    await flushPromises()
    expect(readDirs()).toContain('vendor')
    expect(row(w, 'vendor/autoload.php').classes()).toContain('ignored')
  })

  it('colours a folder with changes inside it without opening it', async () => {
    const w = await show()
    expect(row(w, 'app').classes()).toContain('modified')
    expect(row(w, 'composer.json').classes()).not.toContain('modified')
    expect(readDirs()).toEqual([''])
  })

  it('colours files like the right panel, with a dot for where the change is', async () => {
    const w = await show()
    await row(w, 'app').trigger('click')
    await flushPromises()
    await row(w, 'app/Billing').trigger('click')
    await flushPromises()

    const invoice = row(w, 'app/Billing/InvoiceService.php')
    expect(invoice.classes()).toContain('modified')
    expect(invoice.find('.dot').classes()).toContain('unstaged')

    const rates = row(w, 'app/Billing/TaxRates.php')
    expect(rates.classes()).toContain('added')
    expect(rates.find('.dot').classes()).toContain('staged')

    expect(row(w, 'app/Billing/Money.php').find('.dot').classes()).toContain('both')
    // Unchanged files have no colour and no dot.
    expect(row(w, 'app/Http').classes()).not.toContain('modified')

    // A deleted file is still in its folder, struck through.
    const legacy = row(w, 'app/Billing/LegacyTax.php')
    expect(legacy.classes()).toContain('gone')
    expect(legacy.classes()).toContain('deleted')
  })

  it('follows the status after a stage or an unstage', async () => {
    const w = await show()
    revealInFiles('app/Billing/InvoiceService.php')
    await flushPromises()
    expect(row(w, 'app/Billing/InvoiceService.php').find('.dot').classes()).toContain('unstaged')

    git.store.status = {
      staged: [{ path: 'app/Billing/InvoiceService.php', kind: 'modified' }],
      unstaged: [],
      conflicted: []
    }
    await flushPromises()
    expect(row(w, 'app/Billing/InvoiceService.php').find('.dot').classes()).toContain('staged')

    git.store.status = { staged: [], unstaged: [], conflicted: [] }
    await flushPromises()
    expect(row(w, 'app/Billing/InvoiceService.php').find('.dot').exists()).toBe(false)
    expect(row(w, 'app').classes()).not.toContain('modified')
  })

  it('opens a file in File mode: a preview on one click, kept on a double click', async () => {
    const w = await show()
    revealInFiles('app/Billing/Money.php')
    await flushPromises()

    await row(w, 'app/Billing/Money.php').trigger('click')
    expect(tabs.state.tabs).toHaveLength(1)
    expect(tabs.state.tabs[0]).toMatchObject({ path: 'app/Billing/Money.php', mode: 'file', preview: true })

    await row(w, 'app/Billing/TaxRates.php').trigger('click')
    expect(tabs.state.tabs.map((tab) => tab.path)).toEqual(['app/Billing/TaxRates.php'])

    await row(w, 'app/Billing/TaxRates.php').trigger('dblclick')
    expect(tabs.state.tabs[0]!.preview).toBe(false)
  })

  it('reveals and highlights the file that is open', async () => {
    const w = await show()
    openTab({ path: 'app/Billing/InvoiceService.php', side: 'unstaged' })
    await flushPromises()
    expect(readDirs()).toEqual(expect.arrayContaining(['app', 'app/Billing']))
    expect(row(w, 'app/Billing/InvoiceService.php').classes()).toContain('on')
    expect(treeSession().open).toMatchObject({ app: true, 'app/Billing': true })
  })

  it('reveals a path that is no longer on disk without an error, shutting the folders it lacks', async () => {
    const w = await show()
    revealInFiles('app/Removed/Old.php')
    await flushPromises()
    expect(treeSession().mode).toBe('files')
    expect(treeSession().open).toEqual({ app: true })
    expect(row(w, 'app/Removed/Old.php').exists()).toBe(false)
    expect(w.text()).not.toMatch(/error|not a folder/i)
  })

  it('narrows to matching files, shown flat with their folder', async () => {
    const w = await show()
    await w.find('.filter input').setValue('invsvc')
    await new Promise((done) => setTimeout(done, 120))
    await flushPromises()
    expect(calls.some((one) => one.cmd === 'find_files' && one.args.query === 'invsvc')).toBe(true)
    expect(listed(w)).toEqual([
      'app/Billing/InvoiceService.php',
      'app/Providers/InvoiceServiceProvider.php'
    ])
    const first = row(w, 'app/Billing/InvoiceService.php')
    expect(first.find('.folder-of').text()).toBe('app/Billing')
    expect(first.findAll('.name b').map((one) => one.text()).join('')).toBe('InvSe')

    await w.find('.filter input').trigger('keydown', { key: 'Enter' })
    expect(tabs.state.active).toBe(tabId('app/Billing/InvoiceService.php'))
    expect(tabs.state.tabs[0]!.preview).toBe(false)

    await w.find('.filter input').trigger('keydown', { key: 'Escape' })
    await flushPromises()
    expect(listed(w)).toContain('app')
  })

  it('offers the working-tree actions on a changed file, and asks before a discard', async () => {
    const w = await show()
    revealInFiles('app/Billing/Money.php')
    await flushPromises()
    await row(w, 'app/Billing/Money.php').trigger('contextmenu')
    const labels = menu.state.items.filter((one) => !one.separator).map((one) => one.label)
    expect(labels).toEqual(
      expect.arrayContaining(['Open', 'Open diff', 'Stage', 'Unstage', 'Discard changes…', 'File history', 'Blame', 'Copy path'])
    )

    await menu.state.items.find((one) => one.label === 'Stage')!.action!()
    expect(calls.find((one) => one.cmd === 'stage')?.args).toMatchObject({
      paths: ['app/Billing/Money.php']
    })

    await row(w, 'app/Billing/Money.php').trigger('contextmenu')
    await menu.state.items.find((one) => one.label === 'Discard changes…')!.action!()
    await flushPromises()
    expect(calls.some((one) => one.cmd === 'discard')).toBe(false)
    await w.find('.btn-danger').trigger('click')
    await flushPromises()
    expect(calls.find((one) => one.cmd === 'discard')?.args).toMatchObject({
      paths: ['app/Billing/Money.php']
    })
  })

  it('offers a discard on a file that was added, staged, and edited since', async () => {
    git.store.status = {
      staged: [{ path: 'app/Billing/TaxRates.php', kind: 'added' }],
      unstaged: [{ path: 'app/Billing/TaxRates.php', kind: 'modified' }],
      conflicted: []
    }
    const w = await show()
    revealInFiles('app/Billing/TaxRates.php')
    await flushPromises()
    await row(w, 'app/Billing/TaxRates.php').trigger('contextmenu')
    const labels = menu.state.items.filter((one) => !one.separator).map((one) => one.label)
    expect(labels).toContain('Discard changes…')
  })

  it('leaves a folder as it is on the second click of a double click', async () => {
    const w = await show()
    await row(w, 'app').trigger('click', { detail: 1 })
    await flushPromises()
    expect(listed(w)).toContain('app/Billing')
    await row(w, 'app').trigger('click', { detail: 2 })
    await flushPromises()
    expect(listed(w)).toContain('app/Billing')
  })

  it('says so when the top folder cannot be read, rather than showing nothing', async () => {
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_dir') throw new Error('gone')
      return null
    })
    const w = await show()
    expect(w.text()).toContain('Could not read this folder')
  })

  it('opens the diff of a changed file from its menu', async () => {
    const w = await show()
    revealInFiles('app/Billing/InvoiceService.php')
    await flushPromises()
    await row(w, 'app/Billing/InvoiceService.php').trigger('contextmenu')
    await menu.state.items.find((one) => one.label === 'Open diff')!.action!()
    expect(tabs.activeTab()).toMatchObject({ mode: 'diff', side: 'unstaged' })
  })

  it('keeps each project’s open folders apart', async () => {
    const w = await show()
    await row(w, 'app').trigger('click')
    await flushPromises()
    expect(treeSession().open).toEqual({ app: true })

    git.store.repo = { path: '/repos/other', name: 'other', head: 'main' } as RepoInfo
    await flushPromises()
    expect(treeSession().open).toEqual({})

    git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as RepoInfo
    await flushPromises()
    expect(treeSession().open).toEqual({ app: true })
  })
})
