// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { EditorView } from '@codemirror/view'
import SearchModal from '~/components/SearchModal.vue'
import FileEditor from '~/components/FileEditor.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit } from '~/composables/useGit'
import { openTab, resetTabs, switchTabs, tabId, useTabs } from '~/composables/useTabs'
import { editKey, resetEdits, saveStates } from '~/composables/useFileEdit'
import { aimAt } from '~/composables/useInvoke'
import { openSearch, resetSearch, search, takeDone, takeHits } from '~/composables/useSearch'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => true) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

/**
 * The search window's preview is an editor: it saves like a tab, shares the
 * tab's buffer, hands the keys back to the box on Esc, finds inside itself on
 * ⌘F and searches the file again once an edit is on disk.
 */

const asked = vi.mocked(invoke)
const git = useGit()
const tabs = useTabs()
const isMac = navigator.userAgent.includes('Mac')
const PATH = 'app/Cart.php'

let disk: Record<string, { text: string; hash: string }> = {}
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let refusing = false
let wrappers: ReturnType<typeof mount>[] = []

beforeEach(() => {
  resetTabs()
  resetEdits()
  resetSearch()
  switchTabs('/repos/shop')
  aimAt('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  disk = { [PATH]: { text: '<?php\n$total = 1;\n$x = $total;\n', hash: 'h1' } }
  calls = []
  refusing = false
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    const path = String(args?.path ?? '')
    if (cmd === 'read_file') {
      const now = disk[path]!
      return { text: now.text, hash: now.hash, eol: 'lf', bom: false, readonly: null, missing: false }
    }
    if (cmd === 'write_file') {
      if (refusing || args?.expectedHash !== disk[path]!.hash) throw new Error('Conflict: changed on disk')
      disk[path] = { text: String(args?.text), hash: `w${calls.length}` }
      return disk[path]!.hash
    }
    if (cmd === 'search_file') {
      // What the backend would find in the file as it is on disk now.
      return disk[path]!.text
        .split('\n')
        .map((text, at) => ({ line: at + 1, text, ranges: [[text.indexOf('$total'), text.indexOf('$total') + 6]] }))
        .filter((hit) => hit.text.includes('$total'))
    }
    if (cmd === 'unused_in' || cmd === 'usage_counts') return []
    if (cmd === 'config_get') return { global: {}, profiles: [] }
    return null
  })
})

afterEach(() => {
  for (const one of wrappers) one.unmount()
  wrappers = []
  git.store.repo = null
  git.store.status = null
})

const wait = (ms: number) => new Promise((done) => setTimeout(done, ms))
const writes = () => calls.filter((one) => one.cmd === 'write_file')

async function showSearch() {
  openSearch('text')
  const w = mount(SearchModal, { attachTo: document.body, global: { components: { AppModal } } })
  wrappers.push(w)
  await flushPromises()
  await w.find('.search input[type="text"]').setValue('$total')
  await wait(150)
  await flushPromises()
  const request = search.request
  takeHits({
    request,
    path: PATH,
    hits: [
      { line: 2, text: '$total = 1;', ranges: [[0, 6]] },
      { line: 3, text: '$x = $total;', ranges: [[5, 11]] }
    ]
  })
  takeDone({ request, matches: 2, files: 1, more: 0, searched: 1, cancelled: false, error: null })
  await flushPromises()
  await wait(20)
  await flushPromises()
  return w
}

/** The editor in a tab, as the main window shows it. */
async function showTab() {
  openTab({ path: PATH, mode: 'file' })
  const w = mount(FileEditor, {
    attachTo: document.body,
    props: { tabId: tabId(PATH), path: PATH, hunks: [], runs: null, blame: false, find: null, fallback: null },
    global: { components: { AppModal } }
  })
  wrappers.push(w)
  await flushPromises()
  return w
}

const previewEditor = () => EditorView.findFromDOM(document.querySelector('.preview .cm-editor') as HTMLElement)!
const box = () => document.querySelector('.search input[type="text"]') as HTMLInputElement

function key(target: Element, init: KeyboardEventInit) {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })
  target.dispatchEvent(event)
  return event
}

function type(view: EditorView, at: number, text: string) {
  view.dispatch({ changes: { from: at, insert: text }, userEvent: 'input.type' })
}

describe('the search preview as an editor', () => {
  it('saves what is typed in it against the hash it read, without opening a tab', async () => {
    await showSearch()
    const view = previewEditor()
    expect(view.state.doc.toString()).toBe(disk[PATH]!.text)
    type(view, 0, '// note\n')
    await flushPromises()
    expect(document.querySelector('.where .dirty')).not.toBeNull()
    await wait(900)
    await flushPromises()
    expect(writes()).toHaveLength(1)
    expect(writes()[0]!.args).toMatchObject({ path: PATH, expectedHash: 'h1' })
    expect(disk[PATH]!.text.startsWith('// note\n')).toBe(true)
    expect(tabs.state.tabs.some((one) => one.path === PATH)).toBe(false)
    expect(document.querySelector('.where .dirty')).toBeNull()
  })

  it('is the open tab’s buffer: each shows what was typed in the other', async () => {
    const tab = await showTab()
    const tabView = EditorView.findFromDOM(tab.find('.cm-editor').element as HTMLElement)!
    type(tabView, 0, '// from the tab\n')
    await showSearch()
    const view = previewEditor()
    // The preview starts from the tab's text, edits and all, not from disk.
    expect(view.state.doc.toString()).toBe(`// from the tab\n<?php\n$total = 1;\n$x = $total;\n`)
    type(view, view.state.doc.length, '// from the preview\n')
    // The tab follows as it is typed.
    const typed = view.state.doc.toString()
    expect(tabView.state.doc.toString()).toBe(typed)
    // The window closing hands the file back: the tab edits it again, with
    // the preview's edit in its undo history, and saves it.
    const searching = wrappers[1]!
    searching.unmount()
    wrappers = wrappers.filter((one) => one !== searching)
    await flushPromises()
    await wait(20)
    await flushPromises()
    expect(tabView.state.doc.toString()).toBe(typed)
    expect(tabView.state.facet(EditorView.editable)).toBe(true)
    expect(disk[PATH]!.text).toBe(typed)
  })

  it('goes back to the box on Esc, and the next Esc closes the window', async () => {
    await showSearch()
    const view = previewEditor()
    view.focus()
    key(view.contentDOM, { key: 'Escape' })
    expect(search.open).toBe(true)
    expect(document.activeElement).toBe(box())
    expect(box().value).toBe('$total')
    key(box(), { key: 'Escape' })
    expect(search.open).toBe(false)
  })

  it('puts the caret on the match with ⌘↓ from the box', async () => {
    await showSearch()
    box().focus()
    key(box(), { key: 'ArrowDown', metaKey: isMac, ctrlKey: !isMac })
    const view = previewEditor()
    // Line 2, `$total = 1;`, at its first match.
    expect(view.state.selection.main.head).toBe(view.state.doc.line(2).from)
    expect(view.hasFocus).toBe(true)
  })

  it('opens its own find bar on ⌘F, on the window’s query', async () => {
    await showSearch()
    const view = previewEditor()
    key(view.contentDOM, { key: 'f', metaKey: isMac, ctrlKey: !isMac })
    const field = document.querySelector('.preview .cm-search input[name="search"]') as HTMLInputElement
    expect(field).not.toBeNull()
    expect(field.value).toBe('$total')
  })

  it('takes ↵ as a new line, not as opening the result', async () => {
    await showSearch()
    const view = previewEditor()
    view.dispatch({ selection: { anchor: view.state.doc.line(2).to } })
    key(view.contentDOM, { key: 'Enter' })
    expect(view.state.doc.lines).toBe(5)
    expect(search.open).toBe(true)
    expect(tabs.state.tabs.some((one) => one.path === PATH)).toBe(false)
  })

  it('searches the file again once an edit is saved, and stays on the nearest hit', async () => {
    const w = await showSearch()
    const view = previewEditor()
    type(view, 0, '$total = 0;\n')
    await wait(900)
    await flushPromises()
    await flushPromises()
    expect(calls.some((one) => one.cmd === 'search_file' && one.args.path === PATH)).toBe(true)
    expect(calls.filter((one) => one.cmd === 'search_text')).toHaveLength(1)
    expect(search.found[0]!.hits.map((hit) => hit.line)).toEqual([1, 3, 4])
    expect(w.find('.group .count').text()).toBe('3')
    // The cursor was on line 2; the nearest hit now is line 1 or 3, in this file.
    expect(w.find('.item.on .num').text()).toMatch(/^[13]$/)
  })

  it('opens a file whose edit could not be saved in a tab when the window closes', async () => {
    const w = await showSearch()
    refusing = true
    const view = previewEditor()
    type(view, 0, '// mine\n')
    await wait(900)
    await flushPromises()
    expect(saveStates[editKey('/repos/shop', tabId(PATH))]).toBe('conflict')
    expect(document.querySelector('.where .save-warning')?.textContent).toContain('Changed on disk')
    w.unmount()
    wrappers = wrappers.filter((one) => one !== w)
    await flushPromises()
    await wait(10)
    await flushPromises()
    const tab = tabs.state.tabs.find((one) => one.path === PATH)
    expect(tab).toBeDefined()
    expect(tab!.preview).toBe(false)
    expect(tabs.state.active).toBe(tab!.id)
  })
})
