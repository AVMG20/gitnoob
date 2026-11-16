// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { EditorView } from '@codemirror/view'
import { completionStatus, currentCompletions, startCompletion } from '@codemirror/autocomplete'
import { undo } from '@codemirror/commands'
import { EditorState } from '@codemirror/state'
import { php } from '@codemirror/lang-php'
import FileEditor from '~/components/FileEditor.vue'
import { useGit } from '~/composables/useGit'
import { openTab, resetTabs, switchTabs, tabId } from '~/composables/useTabs'
import { resetEdits } from '~/composables/useFileEdit'
import { aimAt } from '~/composables/useInvoke'
import { useConfig } from '~/composables/useConfig'
import { memberAccess, wantsImport } from '~/composables/useAutocomplete'
import { slots } from '~/composables/editorBuffers'
import { readOnly } from '~/composables/useCodeMirror'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => true) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const asked = vi.mocked(invoke)
const git = useGit()
const PATH = 'app/Tax.php'
const TAB = tabId(PATH)

const PHP = '<?php\n\nnamespace App;\n\nclass Tax\n{\n    public function run()\n    {\n        \n    }\n}\n'
/** Where the empty line inside `run()` is. */
const INSIDE = PHP.indexOf('        \n') + 8

let disk = PHP
let readExtra: Record<string, unknown> = {}
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let settings: Record<string, unknown> = {}
/** Holds `complete_import` back until a test lets it go. */
let importGate: Promise<void> | null = null
let wrapper: ReturnType<typeof mount> | null = null

const MEMBERS = {
  receiver: 'Tax',
  access: 'instance',
  from: 0,
  items: [
    { name: 'total', kind: 'method', static: false, visibility: 'public', signature: '(): int', summary: 'The sum.', class: 'Tax', inherited: false },
    { name: 'rate', kind: 'property', static: false, visibility: 'private', signature: 'float', summary: null, class: 'Tax', inherited: false },
    { name: 'save', kind: 'method', static: false, visibility: 'public', signature: '()', summary: null, class: 'Model', inherited: true }
  ]
}

beforeEach(async () => {
  resetTabs()
  resetEdits()
  switchTabs('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as never
  openTab({ path: PATH, mode: 'file' })
  disk = PHP
  readExtra = {}
  calls = []
  settings = {}
  importGate = null
  aimAt(null)
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'read_file') {
      return { text: disk, hash: 'h1', eol: 'lf', bom: false, readonly: null, missing: false, ...readExtra }
    }
    if (cmd === 'complete_members') {
      const text = String(args?.text)
      const offset = Number(args?.offset)
      // What the backend does: only after an arrow.
      if (!/(->|::)\w*$/.test(text.slice(0, offset))) return { receiver: null, access: null, from: offset, items: [] }
      const from = offset - (/\w*$/.exec(text.slice(0, offset))?.[0].length ?? 0)
      return { ...MEMBERS, from }
    }
    if (cmd === 'complete_symbols') {
      if (String(args?.prefix).startsWith('Dup')) {
        return [
          { name: 'Duplicate', kind: 'class', path: 'app/Models/Duplicate.php' },
          { name: 'Duplicate', kind: 'class', path: 'app/Admin/Duplicate.php' }
        ]
      }
      return String(args?.prefix).startsWith('Bil')
        ? [{ name: 'Billing', kind: 'class', path: 'app/Services/Billing.php' }]
        : []
    }
    if (cmd === 'complete_import') {
      if (importGate) await importGate
      return { offset: PHP.indexOf('\n\nclass Tax'), text: '\n\nuse App\\Services\\Billing;' }
    }
    if (cmd === 'write_file') return 'h2'
    if (cmd === 'config_get') return { global: { usage_counts: false, ...settings }, profiles: [] }
    if (cmd === 'unused_in' || cmd === 'usage_counts') return []
    return null
  })
  await useConfig().load()
})

afterEach(async () => {
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
  settings = {}
  await useConfig().load()
})

async function show(props: Record<string, unknown> = {}) {
  wrapper = mount(FileEditor, {
    attachTo: document.body,
    props: { tabId: TAB, path: PATH, hunks: [], runs: null, blame: false, find: null, fallback: null, ...props }
  })
  await flushPromises()
  // The PHP grammar is imported when the file opens.
  await wait(80)
  await flushPromises()
  return wrapper
}

const wait = (ms: number) => new Promise((done) => setTimeout(done, ms))
const editorOf = (w: ReturnType<typeof mount>) => EditorView.findFromDOM(w.find('.cm-editor').element as HTMLElement)!

/** Types text at the caret, the way a keyboard does. */
function typeAt(view: EditorView, text: string, at = view.state.selection.main.head) {
  view.dispatch({ changes: { from: at, insert: text }, selection: { anchor: at + text.length }, userEvent: 'input.type' })
}

/** Long enough for the typing delay, the pause before the backend, and the list's own guard on ↵. */
const settle = () => wait(260).then(flushPromises)

function press(view: EditorView, key: string) {
  const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true })
  view.contentDOM.dispatchEvent(event)
  return event
}

const labels = (view: EditorView) => currentCompletions(view.state).map((one) => one.label)

describe('autocomplete in the editor', () => {
  it('opens the receiver’s members right after $this->', async () => {
    const view = editorOf(await show())
    typeAt(view, '$this->', INSIDE)
    await settle()
    expect(completionStatus(view.state)).toBe('active')
    expect(labels(view)).toEqual(expect.arrayContaining(['total', 'rate', 'save']))
    const call = calls.find((one) => one.cmd === 'complete_members')!
    expect(call.args).toMatchObject({ path: PATH, offset: INSIDE + 7 })
    expect(String(call.args.text)).toContain('$this->')
    // Its own before what it inherits, and a letter for the kind.
    expect(document.querySelector('.gn-complete')).not.toBeNull()
    expect(document.querySelector('.gn-complete .cm-completionIcon-method')).not.toBeNull()
    expect(document.querySelector('.gn-complete')?.textContent).toContain('· Model')
  })

  it('accepts on ↵ instead of a new line', async () => {
    const view = editorOf(await show())
    typeAt(view, '$this->to', INSIDE)
    await settle()
    expect(labels(view)).toContain('total')
    const event = press(view, 'Enter')
    expect(event.defaultPrevented).toBe(true)
    expect(view.state.doc.toString()).toContain('        $this->total\n    }')
    expect(completionStatus(view.state)).toBe(null)
  })

  it('accepts on Tab while the list is open, and indents when it is not', async () => {
    const view = editorOf(await show())
    typeAt(view, '$this->ra', INSIDE)
    await settle()
    press(view, 'Tab')
    expect(view.state.doc.toString()).toContain('        $this->rate\n')
    // Closed now: Tab is indentation again.
    const before = view.state.doc.length
    press(view, 'Tab')
    expect(view.state.doc.length).toBeGreaterThan(before)
    expect(view.state.doc.toString()).not.toContain('$this->rate\n\n')
  })

  it('closes on Esc without the window hearing it, and lets Esc through once closed', async () => {
    const view = editorOf(await show())
    const heard = vi.fn()
    window.addEventListener('keydown', heard)
    try {
      typeAt(view, '$this->', INSIDE)
      await settle()
      expect(completionStatus(view.state)).toBe('active')
      press(view, 'Escape')
      expect(completionStatus(view.state)).toBe(null)
      // The list took it: graph.show (the viewer's Esc on the window) never ran.
      expect(heard).not.toHaveBeenCalled()
      press(view, 'Escape')
      expect(heard).toHaveBeenCalledTimes(1)
    } finally {
      window.removeEventListener('keydown', heard)
    }
  })

  it('offers nothing inside a comment', async () => {
    const view = editorOf(await show())
    typeAt(view, '// see $this->', INSIDE)
    await settle()
    expect(completionStatus(view.state)).toBe(null)
    expect(calls.some((one) => one.cmd === 'complete_members')).toBe(false)
  })

  it('offers nothing in a read-only editor, even on Ctrl+Space', async () => {
    readExtra = { readonly: 'dependency' }
    const view = editorOf(await show())
    view.dispatch({ changes: { from: INSIDE, insert: '$this->' }, selection: { anchor: INSIDE + 7 } })
    startCompletion(view)
    await settle()
    expect(completionStatus(view.state)).toBe(null)
    expect(calls.some((one) => one.cmd === 'complete_members')).toBe(false)
  })

  it('stays out of the way when Settings → Editor turns it off', async () => {
    settings = { autocomplete: false }
    await useConfig().load()
    const view = editorOf(await show())
    typeAt(view, '$this->', INSIDE)
    await settle()
    expect(completionStatus(view.state)).toBe(null)
    expect(calls.some((one) => one.cmd.startsWith('complete_'))).toBe(false)
  })

  it('opens only on Ctrl+Space when told not to suggest while typing', async () => {
    settings = { autocomplete_typing: false }
    await useConfig().load()
    const view = editorOf(await show())
    typeAt(view, '$this->', INSIDE)
    await settle()
    expect(completionStatus(view.state)).toBe(null)
    startCompletion(view)
    await settle()
    expect(completionStatus(view.state)).toBe('active')
  })

  it('adds the use line when a PHP class is picked', async () => {
    const view = editorOf(await show())
    typeAt(view, 'new Bil', INSIDE)
    await settle()
    expect(labels(view)[0]).toBe('Billing')
    expect(calls.find((one) => one.cmd === 'complete_symbols')?.args).toMatchObject({ prefix: 'Bil', path: PATH })
    press(view, 'Enter')
    await flushPromises()
    await wait(10)
    const text = view.state.doc.toString()
    expect(text).toContain('new Billing')
    expect(text).toContain('namespace App;\n\nuse App\\Services\\Billing;\n\nclass Tax')
    expect(calls.find((one) => one.cmd === 'complete_import')?.args).toMatchObject({
      path: PATH,
      classPath: 'app/Services/Billing.php',
      name: 'Billing'
    })
  })
})

describe('which receiver a list is for', () => {
  it('reads ->, :: and . before the caret', () => {
    expect(memberAccess('a.php', '$this->')).toBe('->')
    expect(memberAccess('a.php', 'Foo::$co')).toBe('::')
    expect(memberAccess('a.php', 'foo')).toBe(null)
    expect(memberAccess('a.ts', 'this.client.re')).toBe('.')
    expect(memberAccess('a.ts', 'x?.')).toBe('.')
    expect(memberAccess('a.ts', '1.')).toBe(null)
    expect(memberAccess('notes.md', 'a.b')).toBe(null)
  })
})

describe('⌘-click on a method whose class cannot be worked out', () => {
  async function clickOn(candidates: unknown[]) {
    const { jumpFrom, usagesPopup, closeUsages } = await import('~/composables/useSymbols')
    const { useToasts } = await import('~/composables/useToasts')
    closeUsages()
    useToasts().clear()
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} })
      if (cmd === 'symbol_at') {
        return { name: 'withToken', kind: 'call', definition: false, shape: 'member', line: 3, from: 0, to: 9 }
      }
      if (cmd === 'member_candidates') return candidates
      if (cmd === 'definitions') return []
      return null
    })
    await jumpFrom(PATH, 4, '<?php $x->withToken();')
    return { popup: usagesPopup, toasts: useToasts().items.value }
  }

  it('says so instead of claiming there is no definition', async () => {
    const { popup, toasts } = await clickOn([])
    expect(popup.open).toBe(false)
    expect(toasts.map((one: { title: string }) => one.title)).toContain(
      "Couldn't work out which class withToken belongs to"
    )
  })

  it('offers the imported classes that have a method of that name', async () => {
    const place = {
      path: 'vendor/laravel/framework/src/Illuminate/Http/Client/PendingRequest.php',
      line: 9,
      column: 20,
      kind: 'method',
      text: 'public function withToken($token): static',
      dependency: true,
      note: null
    }
    const { popup } = await clickOn([place])
    expect(calls.find((one) => one.cmd === 'member_candidates')?.args).toMatchObject({ path: PATH, name: 'withToken' })
    expect(popup.open).toBe(true)
    expect(popup.title).toBe('Possible definitions of withToken() · 1')
    expect(popup.note).toBe("Couldn't work out which class withToken belongs to")
    expect(popup.items[0]).toMatchObject({ path: place.path, line: 9 })
  })
})

describe('what accepting a suggestion does to the text', () => {
  it('completes a PHP variable over its own $, not after it', async () => {
    disk = PHP.replace('        \n', '        $total = 1;\n        \n')
    const view = editorOf(await show())
    const at = disk.indexOf('        \n') + 8
    typeAt(view, '$to', at)
    await settle()
    expect(labels(view)).toEqual(['$total'])
    // A variable is never a project name or a keyword.
    expect(calls.some((one) => one.cmd === 'complete_symbols')).toBe(false)
    press(view, 'Enter')
    await flushPromises()
    expect(view.state.doc.toString()).toContain('$total = 1;\n        $total\n')
  })

  it('offers the PHP keywords, which the language package does not have', async () => {
    const view = editorOf(await show())
    typeAt(view, 'ret', INSIDE)
    await settle()
    expect(labels(view)).toContain('return')
    press(view, 'Enter')
    await flushPromises()
    expect(view.state.doc.toString()).toContain('        return\n')
  })

  it('lists two classes of one name as two rows', async () => {
    const view = editorOf(await show())
    typeAt(view, 'new Dup', INSIDE)
    await settle()
    const rows = currentCompletions(view.state).filter((one) => one.label === 'Duplicate')
    expect(rows.map((one) => one.detail).sort()).toEqual(['app/Admin', 'app/Models'])
  })

  it('takes the name and its use line back in one undo', async () => {
    const view = editorOf(await show())
    typeAt(view, 'new Bil', INSIDE)
    await settle()
    press(view, 'Enter')
    await flushPromises()
    await wait(10)
    expect(view.state.doc.toString()).toContain('use App\\Services\\Billing;')
    undo(view)
    expect(view.state.doc.toString()).toBe(PHP.replace('        \n', '        new Bil\n'))
  })

  it('adds no use line for a name in a use line, a namespace or a qualified name', async () => {
    const view = editorOf(await show())
    typeAt(view, 'use Bil', PHP.indexOf('class Tax'))
    await settle()
    expect(labels(view)).toContain('Billing')
    press(view, 'Enter')
    await flushPromises()
    await wait(10)
    expect(view.state.doc.toString()).toContain('use Billingclass Tax')
    expect(calls.some((one) => one.cmd === 'complete_import')).toBe(false)
    expect(view.state.doc.toString()).not.toContain('App\\Services\\Billing')
  })

  it('knows which names a use line would serve', () => {
    const at = (text: string) => {
      const state = EditorState.create({ doc: text.replace('|', ''), extensions: [php()] })
      return wantsImport(state, text.indexOf('|'))
    }
    expect(at('<?php\nnew |Bil')).toBe(true)
    expect(at('<?php\nfunction pay(|Bil $b) {}')).toBe(true)
    expect(at('<?php\nuse |Bil')).toBe(false)
    expect(at('<?php\nuse App\\Services\\|Bil')).toBe(false)
    expect(at('<?php\nnamespace |Bil')).toBe(false)
    expect(at('<?php\nnew \\App\\Services\\|Bil')).toBe(false)
    // A trait used in a class is a name like any other.
    expect(at('<?php\nclass A\n{\n    use |Bil\n}\n')).toBe(true)
  })

  it('leaves the file alone when the editor went read-only before the use line came', async () => {
    let open: () => void = () => undefined
    importGate = new Promise<void>((done) => (open = done))
    const view = editorOf(await show())
    typeAt(view, 'new Bil', INSIDE)
    await settle()
    press(view, 'Enter')
    await flushPromises()
    expect(view.state.doc.toString()).toContain('new Billing')
    // Lent to the search window's preview meanwhile: this one only follows.
    view.dispatch({ effects: slots.access.reconfigure(readOnly(true)) })
    open()
    await flushPromises()
    await wait(10)
    expect(view.state.doc.toString()).not.toContain('use App\\Services\\Billing;')
  })
})

describe('the list and the rest of the editor', () => {
  it('leaves an input method’s ↵ to the input method', async () => {
    const view = editorOf(await show())
    typeAt(view, '$this->', INSIDE)
    await settle()
    expect(completionStatus(view.state)).toBe('active')
    const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true, isComposing: true })
    view.contentDOM.dispatchEvent(event)
    expect(view.state.doc.toString()).not.toMatch(/->(rate|total|save)/)
  })

  it('does not come back open with the tab', async () => {
    aimAt('/repos/shop')
    const first = await show()
    const view = editorOf(first)
    typeAt(view, '$this->', INSIDE)
    await settle()
    expect(completionStatus(view.state)).toBe('active')
    first.unmount()
    wrapper = null
    await flushPromises()
    const reads = calls.filter((one) => one.cmd === 'read_file').length
    const again = editorOf(await show())
    // The same state, kept warm: the text typed is there, the list is not.
    expect(calls.filter((one) => one.cmd === 'read_file').length).toBe(reads)
    expect(again.state.doc.toString()).toContain('$this->')
    expect(completionStatus(again.state)).toBe(null)
  })
})
