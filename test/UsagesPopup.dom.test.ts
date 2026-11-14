// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import UsagesPopup from '~/components/UsagesPopup.vue'
import { useGit } from '~/composables/useGit'
import { openTab, resetTabs, switchTabs, useTabs } from '~/composables/useTabs'
import { useToasts } from '~/composables/useToasts'
import { closeUsages, jumpFrom, usagesPopup, type CodeLocation } from '~/composables/useSymbols'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const asked = vi.mocked(invoke)
const git = useGit()
const tabs = useTabs()
const toasts = useToasts()
let wrapper: ReturnType<typeof mount> | null = null

const SERVICE = 'app/Billing/InvoiceService.php'
const at = (path: string, line: number, text: string): CodeLocation => ({ path, line, column: 4, kind: 'call', text })

let symbol: Record<string, unknown> | null = null
let found: CodeLocation[] = []

beforeEach(() => {
  resetTabs()
  closeUsages()
  toasts.clear()
  switchTabs('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as never
  git.store.status = {
    staged: [{ path: 'tests/InvoiceServiceTest.php', kind: 'added' }],
    unstaged: [{ path: 'app/Http/InvoiceController.php', kind: 'modified' }],
    conflicted: []
  } as never
  openTab({ path: SERVICE, mode: 'file' })
  symbol = { name: 'total', kind: 'method', definition: true, shape: 'plain', line: 9, from: 120, to: 125 }
  found = [
    at('app/Mail/SendInvoiceMail.php', 27, '$svc->total($inv)'),
    at('app/Http/InvoiceController.php', 42, '$this->billing->total($invoice)'),
    at('tests/InvoiceServiceTest.php', 18, 'assertEquals(1, $svc->total($i))')
  ]
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'symbol_at') return symbol
    if (cmd === 'usages' || cmd === 'definitions') return found
    return null
  })
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
  git.store.status = null
})

async function show() {
  wrapper = mount(UsagesPopup, { attachTo: document.body })
  await flushPromises()
  return wrapper
}

function press(w: ReturnType<typeof mount>, key: string) {
  w.find('.usages').element.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }))
}

describe('⌘-click on a definition', () => {
  it('lists the usages, the ones in changed files first and marked', async () => {
    await jumpFrom(SERVICE, 122, 'the text', { x: 100, y: 200 })
    expect(asked).toHaveBeenCalledWith('symbol_at', { path: SERVICE, offset: 122, text: 'the text' })
    expect(asked).toHaveBeenCalledWith('usages', { name: 'total', path: SERVICE, kind: 'method' })
    expect(usagesPopup.open).toBe(true)
    const w = await show()
    expect(w.find('.title').text()).toBe('Usages of total() · 3 in 3 files')
    const rows = w.findAll('.row')
    expect(rows.map((row) => row.find('.file').text())).toEqual([
      'InvoiceController.php',
      'InvoiceServiceTest.php',
      'SendInvoiceMail.php'
    ])
    expect(rows.map((row) => row.find('.state').exists() ? row.find('.state').text() : '')).toEqual([
      'changed',
      'new',
      ''
    ])
    expect(rows[0]!.find('.code').text()).toBe('$this->billing->total($invoice)')
    expect(w.find('.overlay').exists()).toBe(true)
  })

  it('opens the one picked with ↓ and ↵, pinned, at its line, and on the history', async () => {
    await jumpFrom(SERVICE, 122, null)
    const w = await show()
    press(w, 'ArrowDown')
    await flushPromises()
    expect(w.findAll('.row')[1]!.classes()).toContain('on')
    press(w, 'Enter')
    await flushPromises()
    expect(usagesPopup.open).toBe(false)
    const tab = tabs.activeTab()!
    expect(tab).toMatchObject({ path: 'tests/InvoiceServiceTest.php', mode: 'file', preview: false, line: 18 })
    expect(tabs.state.back.at(-1)?.path).toBe(SERVICE)
  })

  it('opens one on a click, and closes on Esc or a click beside it', async () => {
    await jumpFrom(SERVICE, 122, null)
    let w = await show()
    await w.findAll('.row')[2]!.trigger('click')
    expect(tabs.activeTab()?.path).toBe('app/Mail/SendInvoiceMail.php')
    w.unmount()

    await jumpFrom(SERVICE, 122, null)
    w = await show()
    press(w, 'Escape')
    expect(usagesPopup.open).toBe(false)
    w.unmount()

    await jumpFrom(SERVICE, 122, null)
    w = await show()
    await w.find('.usages-backdrop').trigger('mousedown')
    expect(usagesPopup.open).toBe(false)
  })

  it('jumps straight there when there is one usage', async () => {
    found = [at('app/Http/InvoiceController.php', 42, 'total()')]
    await jumpFrom(SERVICE, 122, null)
    expect(usagesPopup.open).toBe(false)
    expect(tabs.activeTab()).toMatchObject({ path: 'app/Http/InvoiceController.php', line: 42 })
  })

  it('takes an empty answer from the backend as no usages', async () => {
    asked.mockImplementation(async (cmd: string) => (cmd === 'symbol_at' ? symbol : null))
    await jumpFrom(SERVICE, 122, null)
    expect(toasts.items.value.map((one) => one.title)).toContain('No usages found')
  })

  it('says so when nothing uses it', async () => {
    found = []
    await jumpFrom(SERVICE, 122, null)
    expect(usagesPopup.open).toBe(false)
    expect(toasts.items.value.map((one) => one.title)).toContain('No usages found')
    expect(tabs.activeTab()?.path).toBe(SERVICE)
  })
})

describe('⌘-click on a reference', () => {
  it('goes to the definition, asking for the shape of the call', async () => {
    symbol = { name: 'subtotal', kind: 'call', definition: false, shape: 'member', line: 11, from: 1, to: 9 }
    found = [{ path: SERVICE, line: 16, column: 21, kind: 'method', text: 'private function subtotal()' }]
    await jumpFrom(SERVICE, 3, null)
    expect(asked).toHaveBeenCalledWith('definitions', { name: 'subtotal', path: SERVICE, shape: 'member' })
    expect(tabs.activeTab()).toMatchObject({ path: SERVICE, line: 16 })
    expect(tabs.state.landing).toMatchObject({ line: 16 })
  })

  it('lists the definitions when there are several', async () => {
    symbol = { name: 'Money', kind: 'class', definition: false, shape: 'type', line: 1, from: 1, to: 6 }
    found = [at('app/Money.php', 5, 'class Money'), at('lib/Money.php', 3, 'class Money')]
    await jumpFrom(SERVICE, 3, null)
    const w = await show()
    expect(w.find('.title').text()).toBe('Definitions of Money · 2')
    expect(w.findAll('.row')).toHaveLength(2)
  })

  it('does nothing on something that is not a name', async () => {
    symbol = null
    await jumpFrom(SERVICE, 3, null)
    expect(asked.mock.calls.map(([cmd]) => cmd)).toEqual(['symbol_at'])
    expect(usagesPopup.open).toBe(false)
  })
})
