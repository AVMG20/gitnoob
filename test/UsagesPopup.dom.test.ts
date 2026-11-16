// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import UsagesPopup from '~/components/UsagesPopup.vue'
import { useGit } from '~/composables/useGit'
import { openTab, resetTabs, switchTabs, useTabs } from '~/composables/useTabs'
import { useToasts } from '~/composables/useToasts'
import { closeUsages, jumpFrom, showUsages, usagesPopup, type CodeLocation } from '~/composables/useSymbols'

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
let possible: CodeLocation[] = []
let framework = false
let owner: string | null = null

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
  possible = []
  framework = false
  owner = null
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'symbol_at') return symbol
    if (cmd === 'usages') return { definite: found, possible, framework, owner }
    if (cmd === 'definitions') return found
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
    expect(asked).toHaveBeenCalledWith('usages', { name: 'total', path: SERVICE, kind: 'method', line: 9 })
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
    // The app's own explanation is a plain note, not tucked away as "what git said".
    const notice = toasts.items.value.find((one) => one.title === 'No usages found')
    expect(notice?.detail).toBeNull()
    expect(notice?.note).toMatch(/built at run time/)
    expect(tabs.activeTab()?.path).toBe(SERVICE)
  })
})

describe('definite and possible usages', () => {
  const unknown = [
    at('app/Jobs/Rebuild.php', 12, '$thing->total()'),
    at('app/Support/Report.php', 30, '$row->total()'),
    at('app/Support/Sheet.php', 8, '$x->total()')
  ]

  it('lists the definite ones, then a folded group of the possible ones', async () => {
    found = found.slice(0, 2)
    possible = unknown
    await jumpFrom(SERVICE, 122, null)
    const w = await show()
    expect(w.find('.title').text()).toBe('Usages of total() · 2 in 2 files')
    expect(w.findAll('.row')).toHaveLength(2)
    expect(w.find('.group').text()).toContain('Possible (3)')
    expect(w.find('.group').attributes('aria-expanded')).toBe('false')

    // ↓ ↓ reaches the group, ↵ unfolds it, and the possible rows are muted.
    press(w, 'ArrowDown')
    press(w, 'ArrowDown')
    await flushPromises()
    expect(w.find('.group').classes()).toContain('on')
    press(w, 'Enter')
    await flushPromises()
    expect(usagesPopup.open).toBe(true)
    const rows = w.findAll('.row')
    expect(rows).toHaveLength(5)
    expect(rows.slice(2).every((row) => row.classes().includes('possible'))).toBe(true)
    expect(rows[2]!.find('.code').text()).toBe('$thing->total()')

    // A click folds it again.
    await w.find('.group').trigger('click')
    expect(w.findAll('.row')).toHaveLength(2)
  })

  it('opens the group by itself when nothing is definite', async () => {
    found = []
    possible = unknown
    await jumpFrom(SERVICE, 122, null)
    const w = await show()
    expect(w.find('.title').text()).toBe('Possible usages of total() · 3')
    expect(w.findAll('.row')).toHaveLength(3)
  })

  it('⌘-click jumps to the one definite usage; the count lists it with the possible ones', async () => {
    found = [at('app/Http/InvoiceController.php', 42, '$this->billing->total($invoice)')]
    possible = unknown
    await jumpFrom(SERVICE, 122, null)
    expect(usagesPopup.open).toBe(false)
    expect(tabs.activeTab()).toMatchObject({ path: 'app/Http/InvoiceController.php', line: 42 })

    await showUsages('total', SERVICE, 'method', { x: 1, y: 1 }, 9)
    expect(usagesPopup.open).toBe(true)
    expect(usagesPopup.items).toHaveLength(1)
    expect(usagesPopup.possible).toHaveLength(3)
  })
})

describe('what the framework calls', () => {
  beforeEach(() => {
    symbol = { name: 'handle', kind: 'method', definition: true, shape: 'plain', line: 12, from: 120, to: 126 }
    framework = true
    owner = 'ExportAnalyticsCommand'
    found = [
      at('routes/console.php', 5, "Schedule::command('export:analytics')->daily();"),
      at('tests/Feature/ExportAnalyticsTest.php', 6, "$this->artisan('export:analytics')")
    ]
  })

  it('lists the uses of its class, and says why', async () => {
    await jumpFrom(SERVICE, 122, null)
    const w = await show()
    expect(w.find('.title').text()).toBe('Uses of ExportAnalyticsCommand · 2')
    expect(w.find('.note').text()).toBe('handle() is called by the framework')
    expect(w.findAll('.row')).toHaveLength(2)
    expect(w.find('.group').exists()).toBe(false)
  })

  it('says it is called by the framework when nothing uses the class', async () => {
    found = []
    await jumpFrom(SERVICE, 122, null)
    expect(usagesPopup.open).toBe(false)
    expect(toasts.items.value.map((one) => one.title)).toContain('handle() is called by the framework')
    expect(toasts.items.value.map((one) => one.title)).not.toContain('No usages found')
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
    // Only the way into a dependency is asked as well, and it found nothing.
    expect(asked.mock.calls.map(([cmd]) => cmd)).toEqual(['symbol_at', 'dependency_definition'])
    expect(usagesPopup.open).toBe(false)
  })

  it('opens a definition in a dependency in its own tab, and says what it has to', async () => {
    symbol = { name: 'spreadsheet', kind: 'call', definition: false, shape: 'scoped', line: 14, from: 1, to: 12 }
    const facade = 'vendor/revolution/laravel-google-sheets/src/Facades/Sheets.php'
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'symbol_at') return symbol
      if (cmd === 'dependency_definition') {
        return {
          path: facade,
          line: 9,
          column: 6,
          kind: 'class',
          text: 'class Sheets extends Facade',
          dependency: true,
          note: 'Sheets is a facade; the method lives on the class it resolves to'
        }
      }
      if (cmd === 'definitions') return []
      return null
    })
    await jumpFrom(SERVICE, 3, 'the text')
    expect(asked).toHaveBeenCalledWith('dependency_definition', { path: SERVICE, offset: 3, text: 'the text' })
    // The index is not asked: the dependency is not in it.
    expect(asked.mock.calls.map(([cmd]) => cmd)).not.toContain('definitions')
    expect(tabs.activeTab()).toMatchObject({ path: facade, mode: 'file', preview: false, line: 9 })
    expect(toasts.items.value.at(-1)?.title).toBe('Sheets is a facade; the method lives on the class it resolves to')
  })
})
