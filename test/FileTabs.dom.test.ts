// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import FileTabs from '~/components/FileTabs.vue'
import { useGit, WIP, type RepoInfo } from '~/composables/useGit'
import { openTab, resetTabs, switchTabs, tabId, useTabs } from '~/composables/useTabs'
import { useContextMenu } from '~/composables/useContextMenu'
import { resetFileTree, treeSession } from '~/composables/useFileTree'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const git = useGit()
const tabs = useTabs()
const menu = useContextMenu()

let wrapper: ReturnType<typeof mount> | null = null

async function show() {
  wrapper = mount(FileTabs, { attachTo: document.body })
  await flushPromises()
  return wrapper
}

const fileTabs = (w: ReturnType<typeof mount>) => w.findAll('.ftab:not(.graph)')

beforeEach(() => {
  resetTabs()
  resetFileTree()
  menu.close()
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as RepoInfo
  switchTabs('/repos/shop')
  git.store.status = {
    staged: [{ path: 'app/TaxRates.php', kind: 'added' }],
    unstaged: [{ path: 'app/InvoiceService.php', kind: 'modified' }],
    conflicted: []
  }
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
})

describe('the file tab strip', () => {
  it('is not there while only the graph is open', async () => {
    const w = await show()
    expect(w.find('.file-tabs').exists()).toBe(false)
  })

  it('shows the graph first, then each file with its status letter', async () => {
    openTab({ path: 'app/InvoiceService.php', side: 'unstaged' })
    openTab({ path: 'app/TaxRates.php', side: 'staged' })
    const w = await show()
    const all = w.findAll('.ftab')
    expect(all[0]!.classes()).toContain('graph')
    expect(all[1]!.find('.name').text()).toBe('InvoiceService.php')
    expect(all[1]!.find('.letter').text()).toBe('M')
    expect(all[2]!.find('.letter').text()).toBe('A')
    expect(all[2]!.classes()).toContain('on')
  })

  it('draws a preview tab in italics and keeps it on a double click', async () => {
    openTab({ path: 'app/InvoiceService.php' }, { preview: true })
    const w = await show()
    expect(fileTabs(w)[0]!.classes()).toContain('preview')
    await fileTabs(w)[0]!.trigger('dblclick')
    expect(fileTabs(w)[0]!.classes()).not.toContain('preview')
  })

  it('names the commit a file was opened at', async () => {
    openTab({ path: 'app/InvoiceService.php', commit: 'a41c9e7f00' })
    const w = await show()
    expect(fileTabs(w)[0]!.find('.hash').text()).toBe('@ a41c9e')
  })

  it('shows a dot instead of the cross while a file has unsaved edits', async () => {
    const tab = openTab({ path: 'app/InvoiceService.php' })
    tab.dirty = true
    const w = await show()
    expect(fileTabs(w)[0]!.find('.dirty').exists()).toBe(true)
    // The cross stays in the tab, under the dot, so the keyboard and a hover
    // can still reach it.
    expect(fileTabs(w)[0]!.classes()).toContain('unsaved')
    expect(fileTabs(w)[0]!.find('.close').exists()).toBe(true)
  })

  it('takes a tab from the keyboard', async () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    const w = await show()
    expect(fileTabs(w)[0]!.attributes('tabindex')).toBe('0')
    await fileTabs(w)[0]!.trigger('keydown', { key: 'Enter' })
    expect(useTabs().activeTab()?.path).toBe('a.php')
  })

  it('switches tabs on a click, and to the graph from the graph tab', async () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    const w = await show()
    await fileTabs(w)[0]!.trigger('click')
    expect(tabs.state.active).toBe(tabId('a.php'))
    await w.find('.ftab.graph').trigger('click')
    expect(tabs.state.active).toBeNull()
    // The files stay open behind the graph.
    expect(fileTabs(w)).toHaveLength(2)
    expect(w.find('.ftab.graph').classes()).toContain('on')
  })

  it('closes a tab on a middle click', async () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    const w = await show()
    await fileTabs(w)[0]!.trigger('mouseup', { button: 1 })
    expect(tabs.state.tabs.map((tab) => tab.path)).toEqual(['b.php'])
  })

  it('closes a tab on a ⇧-click without going to it first, and leaves the graph tab alone', async () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    const w = await show()
    expect(fileTabs(w)[0]!.attributes('title')).toContain('Middle-click or ⇧-click to close')
    expect(tabs.state.active).toBe(tabId('b.php'))
    const back = tabs.state.back.length
    await fileTabs(w)[0]!.trigger('click', { shiftKey: true })
    await flushPromises()
    expect(tabs.state.back).toHaveLength(back)
    expect(tabs.state.tabs.map((tab) => tab.path)).toEqual(['b.php'])
    // The tab on screen stayed the one on screen.
    expect(tabs.state.active).toBe(tabId('b.php'))

    await w.find('.ftab.graph').trigger('click', { shiftKey: true })
    expect(tabs.state.tabs.map((tab) => tab.path)).toEqual(['b.php'])
    expect(tabs.state.active).toBe(tabId('b.php'))
  })

  it('goes when the last file closes, and back brings the file back', async () => {
    openTab({ path: 'a.php' })
    const w = await show()
    await fileTabs(w)[0]!.find('.close').trigger('click')
    expect(tabs.state.active).toBeNull()
    expect(w.find('.file-tabs').exists()).toBe(false)

    // The strip is gone, so back comes from the keyboard or the mouse here.
    tabs.goBack()
    await flushPromises()
    expect(w.find('.file-tabs').exists()).toBe(true)
    expect(tabs.state.active).toBe(tabId('a.php'))
  })

  it('steps back and forward with the arrows, which are off when there is nowhere to go', async () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    const w = await show()
    const back = w.find('[data-testid="nav-back"]')
    const forward = w.find('[data-testid="nav-forward"]')
    expect(forward.attributes('disabled')).toBeDefined()
    await back.trigger('click')
    expect(tabs.state.active).toBe(tabId('a.php'))
    expect(forward.attributes('disabled')).toBeUndefined()
    await forward.trigger('click')
    expect(tabs.state.active).toBe(tabId('b.php'))
  })

  it('goes back on the mouse’s back button, and stops the web view’s own', async () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    const w = await show()
    expect(w.exists()).toBe(true)
    const { useNavButtons } = await import('~/composables/useNavButtons')
    const { defineComponent, h } = await import('vue')
    const listener = mount(
      defineComponent({
        setup() {
          useNavButtons()
          return () => h('div')
        }
      })
    )
    await flushPromises()
    const press = new MouseEvent('mouseup', { button: 3, cancelable: true, bubbles: true })
    window.dispatchEvent(press)
    expect(press.defaultPrevented).toBe(true)
    expect(tabs.state.active).toBe(tabId('a.php'))
    listener.unmount()
  })

  it('offers close, close others, close all, copy path and reveal in Files', async () => {
    openTab({ path: 'app/InvoiceService.php' })
    openTab({ path: 'app/TaxRates.php' })
    const w = await show()
    await fileTabs(w)[0]!.trigger('contextmenu')
    const labels = menu.state.items.filter((one) => !one.separator).map((one) => one.label)
    expect(labels).toEqual(['Close', 'Close others', 'Close all', 'Copy path', 'Reveal in Files'])

    await menu.state.items.find((one) => one.label === 'Reveal in Files')!.action!()
    expect(treeSession().mode).toBe('files')
    expect(treeSession().open).toEqual({ app: true })
    expect(treeSession().reveal?.path).toBe('app/InvoiceService.php')

    await fileTabs(w)[0]!.trigger('contextmenu')
    await menu.state.items.find((one) => one.label === 'Close others')!.action!()
    expect(tabs.state.tabs.map((tab) => tab.path)).toEqual(['app/InvoiceService.php'])
  })

  it('brings the graph forward when a branch is clicked, and keeps the files open', async () => {
    git.store.rows = [{ oid: 'c0ffee' }] as never
    openTab({ path: 'a.php' })
    const w = await show()
    await git.revealCommit('c0ffee')
    await flushPromises()
    expect(tabs.state.active).toBeNull()
    expect(fileTabs(w)).toHaveLength(1)
    // And back returns to the file.
    tabs.goBack()
    expect(tabs.state.active).toBe(tabId('a.php'))
    git.store.rows = []
  })
})

describe('the right panel follows the tab', () => {
  it('goes back to the working tree when a working-tree tab comes on screen while a commit is selected', async () => {
    openTab({ path: 'app/Old.php', commit: 'a'.repeat(40), mode: 'file' })
    git.store.selected = 'a'.repeat(40)
    git.store.detail = { oid: 'a'.repeat(40) } as never
    await flushPromises()
    expect(git.store.selected).toBe('a'.repeat(40))

    openTab({ path: 'app/InvoiceService.php', mode: 'file' })
    await flushPromises()
    expect(git.store.selected).toBe(WIP)
    expect(git.store.detail).toBeNull()
  })
})
