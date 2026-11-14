// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { EditorView } from '@codemirror/view'
import { indentUnit, syntaxTree } from '@codemirror/language'
import { insertNewlineAndIndent } from '@codemirror/commands'
import FileEditor from '~/components/FileEditor.vue'
import DiffViewer from '~/components/DiffViewer.vue'
import FileView from '~/components/FileView.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit } from '~/composables/useGit'
import { diffMode } from '~/composables/useDiffMode'
import { openTab, resetTabs, switchTabs, tabId } from '~/composables/useTabs'
import { resetEdits } from '~/composables/useFileEdit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => true) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const asked = vi.mocked(invoke)
const git = useGit()
const PATH = 'app/Console/Commands/MakeThing.php'
const LINE = "        $type = $this->argument('type');"
const TEXT = `<?php

class MakeThing extends Command
{
    public function handle(): int
    {
${LINE}
        return 0;
    }
}
`
let editorconfig: unknown = null
let wrapper: ReturnType<typeof mount> | null = null

beforeEach(() => {
  resetTabs()
  resetEdits()
  switchTabs('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as never
  openTab({ path: PATH, mode: 'file' })
  editorconfig = null
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'read_file') return { text: TEXT, hash: 'h1', eol: 'lf', bom: false, readonly: null, missing: false }
    if (cmd === 'indent_config') return editorconfig
    if (cmd === 'file_text') return TEXT
    if (cmd === 'working_file_diff' || cmd === 'head_file_diff') return { path: PATH, binary: false, truncated: 0, hunks: [] }
    if (cmd === 'config_get') return { global: {}, profiles: [] }
    if (cmd === 'unused_in' || cmd === 'usage_counts') return []
    return null
  })
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
  git.store.viewer = null
  git.store.status = null
  diffMode.mode = 'diff'
})

async function show() {
  wrapper = mount(FileEditor, {
    attachTo: document.body,
    props: { tabId: tabId(PATH), path: PATH, hunks: [], runs: null, blame: false, find: null, fallback: null },
    global: { components: { AppModal } }
  })
  await flushPromises()
  return wrapper
}

const editorOf = (w: ReturnType<typeof mount>) =>
  EditorView.findFromDOM(w.find('.host .cm-editor').element as HTMLElement)!

/** Waits for the PHP grammar to load and parse the file. */
async function parsed(view: EditorView) {
  for (let i = 0; i < 100 && syntaxTree(view.state).length < view.state.doc.length; i++) {
    await new Promise((done) => setTimeout(done, 10))
  }
  expect(syntaxTree(view.state).length).toBe(view.state.doc.length)
}

describe('indenting in the editor', () => {
  it('lands a new line inside a method at eight spaces in a four-space file', async () => {
    const view = editorOf(await show())
    expect(view.state.facet(indentUnit)).toBe('    ')
    await parsed(view)
    const end = TEXT.indexOf(LINE) + LINE.length
    view.dispatch({ selection: { anchor: end } })
    insertNewlineAndIndent(view)
    const line = view.state.doc.lineAt(view.state.selection.main.head)
    expect(line.text).toBe(' '.repeat(8))
  })

  it('takes .editorconfig over the file', async () => {
    editorconfig = { style: 'tab', size: 4, tab_width: 4 }
    const view = editorOf(await show())
    expect(view.state.facet(indentUnit)).toBe('\t')
  })

  it('indents and dedents with Tab and ⇧Tab, and leaves Diff and File alone', async () => {
    git.store.status = { staged: [], unstaged: [{ path: PATH, kind: 'modified' }], conflicted: [] } as never
    git.store.viewer = { path: PATH, side: 'unstaged' } as never
    diffMode.mode = 'file'
    wrapper = mount(DiffViewer, {
      attachTo: document.body,
      global: { components: { FileView, FileEditor }, stubs: { Spinner: true, ChangeRuler: true } }
    })
    await flushPromises()
    const view = editorOf(wrapper)
    const start = TEXT.indexOf('return 0;')
    view.dispatch({ selection: { anchor: start } })
    view.focus()
    const key = (shiftKey: boolean) =>
      view.contentDOM.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey, bubbles: true, cancelable: true }))
    key(false)
    expect(view.state.doc.lineAt(start).text).toBe(' '.repeat(12) + 'return 0;')
    key(true)
    expect(view.state.doc.lineAt(start).text).toBe(' '.repeat(8) + 'return 0;')
    expect(diffMode.mode).toBe('file')
    // The header says what it indents with.
    expect(wrapper.find('[data-fact="indent"]').text()).toBe('Spaces: 4')
  })
})
