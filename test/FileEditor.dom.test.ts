// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { EditorView } from '@codemirror/view'
import FileEditor from '~/components/FileEditor.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit } from '~/composables/useGit'
import { findTab, openTab, resetTabs, switchTabs, tabId, useTabs } from '~/composables/useTabs'
import {
  closeTabsSafely,
  compareBlocks,
  drafts,
  editKey,
  flushing,
  prepareToQuit,
  resetEdits,
  saveStates
} from '~/composables/useFileEdit'
import { aimAt, aimedAt } from '~/composables/useInvoke'
import { useShortcuts } from '~/composables/useShortcuts'
import { useConfig } from '~/composables/useConfig'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const answer = vi.hoisted(() => ({ ask: true, asked: [] as string[] }))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  ask: vi.fn(async (message: string) => {
    answer.asked.push(message)
    return answer.ask
  })
}))

/** The watcher's event, which the editor listens for to see changes on disk. */
let diskChanged: ((event: { payload: { work_tree: boolean } }) => void) | null = null
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: never) => void) => {
    if (name === 'repo-changed') diskChanged = handler as never
    return () => undefined
  })
}))

const asked = vi.mocked(invoke)
const git = useGit()
const tabs = useTabs()
const PATH = 'app/Tax.php'
const TAB = tabId(PATH)
const KEY = () => editKey(aimedAt(), TAB)

let disk = { text: '<?php\nreturn 21;\n', hash: 'h1' }
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let refuse: string | null = null
let wrapper: ReturnType<typeof mount> | null = null

const read = (extra: Record<string, unknown> = {}) => ({
  text: disk.text,
  hash: disk.hash,
  eol: 'lf',
  bom: false,
  readonly: null,
  missing: false,
  ...extra
})
let readExtra: Record<string, unknown> = {}
/** What the backend says about the names in the file. */
let unused: unknown[] = []
let lenses: unknown[] = []
let countsOff = false
/** Holds writes or reads back until released, to catch them in flight. */
let writeGate: Promise<void> | null = null
let readGate: Promise<void> | null = null

beforeEach(() => {
  resetTabs()
  resetEdits()
  switchTabs('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as never
  openTab({ path: PATH, mode: 'file' })
  disk = { text: '<?php\nreturn 21;\n', hash: 'h1' }
  readExtra = {}
  refuse = null
  calls = []
  diskChanged = null
  unused = []
  lenses = []
  countsOff = false
  writeGate = null
  readGate = null
  answer.ask = true
  answer.asked = []
  aimAt(null)
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'read_file') {
      const now = read(readExtra)
      if (readGate) await readGate
      return now
    }
    if (cmd === 'unused_in') return unused
    if (cmd === 'usage_counts') return lenses
    if (cmd === 'symbol_at') return null
    if (cmd === 'usages' || cmd === 'definitions') return []
    if (cmd === 'config_get') return { global: { usage_counts: !countsOff }, profiles: [] }
    if (cmd === 'write_file') {
      if (writeGate) await writeGate
      if (refuse) throw new Error(refuse)
      if (args?.expectedHash !== disk.hash) throw new Error('Conflict: changed on disk')
      disk = { text: String(args?.text), hash: `h${calls.length}` }
      return disk.hash
    }
    return null
  })
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
})

async function show(props: Record<string, unknown> = {}) {
  wrapper = mount(FileEditor, {
    attachTo: document.body,
    props: {
      tabId: TAB,
      path: PATH,
      hunks: [],
      runs: null,
      blame: false,
      find: null,
      fallback: null,
      ...props
    },
    global: { components: { AppModal } }
  })
  await flushPromises()
  return wrapper
}

const editorOf = (w: ReturnType<typeof mount>) =>
  EditorView.findFromDOM(w.find('.cm-editor').element as HTMLElement)!

function type(w: ReturnType<typeof mount>, text: string, at = 0) {
  editorOf(w).dispatch({ changes: { from: at, insert: text }, userEvent: 'input.type' })
}

const writes = () => calls.filter((one) => one.cmd === 'write_file')
const wait = (ms: number) => new Promise((done) => setTimeout(done, ms))

describe('the file editor', () => {
  it('opens the file from disk, editable', async () => {
    const w = await show()
    expect(editorOf(w).state.doc.toString()).toBe(disk.text)
    expect(w.find('.cm-content').attributes('contenteditable')).toBe('true')
    expect(saveStates[KEY()]).toBe('saved')
  })

  it('saves on its own 800 ms after the last keystroke, naming the hash it read', async () => {
    const w = await show()
    type(w, '// note\n')
    expect(saveStates[KEY()]).toBe('unsaved')
    expect(tabs.state.tabs[0]!.dirty).toBe(true)
    await wait(300)
    expect(writes()).toHaveLength(0)
    await wait(700)
    await flushPromises()
    expect(writes()).toHaveLength(1)
    expect(writes()[0]!.args).toMatchObject({ path: PATH, text: `// note\n${'<?php\nreturn 21;\n'}`, expectedHash: 'h1' })
    expect(saveStates[KEY()]).toBe('saved')
    expect(tabs.state.tabs[0]!.dirty).toBe(false)
  })

  it('saves straight away on ⌘S while it has the caret', async () => {
    const w = await show()
    type(w, 'x')
    const isMac = navigator.userAgent.includes('Mac')
    w.find('.cm-content').element.dispatchEvent(
      new KeyboardEvent('keydown', { key: 's', metaKey: isMac, ctrlKey: !isMac, bubbles: true, cancelable: true })
    )
    await flushPromises()
    expect(writes()).toHaveLength(1)
  })

  it('names the new hash on the save after, and ignores the watcher’s report of its own save', async () => {
    const w = await show()
    type(w, 'a')
    await editorOf(w)
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    const after = disk.hash
    // The watcher reports the write the editor just made.
    diskChanged?.({ payload: { work_tree: true } })
    await wait(200)
    await flushPromises()
    expect(w.find('.disk-bar').exists()).toBe(false)
    type(w, 'b')
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    expect(writes()[1]!.args.expectedHash).toBe(after)
  })

  it('reloads quietly when the file changes on disk and nothing is typed', async () => {
    const w = await show()
    const view = editorOf(w)
    view.dispatch({ selection: { anchor: 3 } })
    disk = { text: '<?php\nreturn 9;\n', hash: 'h9' }
    diskChanged?.({ payload: { work_tree: true } })
    await wait(200)
    await flushPromises()
    expect(view.state.doc.toString()).toBe('<?php\nreturn 9;\n')
    expect(view.state.selection.main.head).toBe(3)
    expect(w.find('.disk-bar').exists()).toBe(false)
    expect(writes()).toHaveLength(0)
  })

  it('never writes over a file changed on disk while there are edits, and asks', async () => {
    const w = await show()
    type(w, 'mine ')
    // An agent rewrites the file before the save goes out.
    disk = { text: '<?php\nreturn 9;\n', hash: 'h9' }
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    await flushPromises()
    expect(disk.text).toBe('<?php\nreturn 9;\n')
    expect(w.find('.disk-bar').text()).toContain('Changed on disk')
    expect(saveStates[KEY()]).toBe('conflict')

    // Keep mine: written over the file as it is now, naming its hash.
    const keep = w.findAll('.disk-bar button').find((one) => one.text() === 'Keep mine')!
    await keep.trigger('click')
    await flushPromises()
    expect(writes().at(-1)!.args.expectedHash).toBe('h9')
    expect(disk.text).toBe('mine <?php\nreturn 21;\n')
    expect(w.find('.disk-bar').exists()).toBe(false)
  })

  it('takes theirs when asked, dropping the edits', async () => {
    const w = await show()
    type(w, 'mine ')
    disk = { text: 'theirs\n', hash: 'h9' }
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    await flushPromises()
    const take = w.findAll('.disk-bar button').find((one) => one.text() === 'Take theirs')!
    await take.trigger('click')
    expect(editorOf(w).state.doc.toString()).toBe('theirs\n')
    expect(saveStates[KEY()]).toBe('saved')
  })

  it('compares the two side by side and saves what was picked', async () => {
    const w = await show()
    type(w, 'mine\n')
    disk = { text: 'theirs\n<?php\nreturn 21;\n', hash: 'h9' }
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    await flushPromises()
    await w.findAll('.disk-bar button').find((one) => one.text() === 'Compare')!.trigger('click')
    await flushPromises()
    // Nothing is saved until every difference is answered.
    expect(w.find('[data-testid="compare-save"]').attributes('disabled')).toBeDefined()
    expect(w.text()).toContain('1 difference left to decide')
    // One difference: the first line. Both keeps both.
    await w.find('[data-pick="both"]').trigger('click')
    expect(w.find('[data-testid="compare-save"]').attributes('disabled')).toBeUndefined()
    await w.find('[data-testid="compare-save"]').trigger('click')
    await flushPromises()
    expect(writes().at(-1)!.args).toMatchObject({ expectedHash: 'h9', text: 'mine\ntheirs\n<?php\nreturn 21;\n' })
  })

  it('opens a binary or non-UTF-8 file read-only, and says why', async () => {
    readExtra = { readonly: 'encoding' }
    const w = await show()
    expect(w.find('.cm-content').attributes('contenteditable')).toBe('false')
    expect(w.text()).toContain('Not UTF-8')
  })

  it('is read-only for a commit’s copy', async () => {
    const w = await show({ locked: true, fallback: 'old\n' })
    expect(editorOf(w).state.doc.toString()).toBe('old\n')
    expect(w.find('.cm-content').attributes('contenteditable')).toBe('false')
    expect(calls.some((one) => one.cmd === 'read_file')).toBe(false)
  })

  it('saves what was typed when the tab is left before the timer ran', async () => {
    const w = await show()
    type(w, 'left ')
    w.unmount()
    wrapper = null
    await flushPromises()
    expect(writes()).toHaveLength(1)
    expect(writes()[0]!.args.text).toBe('left <?php\nreturn 21;\n')
    expect(drafts.size).toBe(0)
  })

  it('keeps edits whose save was refused, and asks when the tab comes back', async () => {
    const w = await show()
    type(w, 'left ')
    disk = { text: 'theirs\n', hash: 'h9' }
    w.unmount()
    wrapper = null
    await flushPromises()
    expect(drafts.get(KEY())?.text).toBe('left <?php\nreturn 21;\n')
    const again = await show()
    await flushPromises()
    expect(editorOf(again).state.doc.toString()).toBe('left <?php\nreturn 21;\n')
    expect(again.find('.disk-bar').exists()).toBe(true)
  })

  it('draws change bars from the hunks, and a click opens the card rather than the diff', async () => {
    const hunks = [
      {
        header: '@@ -1,1 +1,1 @@',
        lines: [
          { origin: '-', old_lineno: 1, new_lineno: null, content: '<?hh' },
          { origin: '+', old_lineno: null, new_lineno: 1, content: '<?php' }
        ]
      }
    ]
    const w = await show({ hunks })
    const bar = w.find('.cm-bar.changed')
    expect(bar.exists()).toBe(true)
    // happy-dom lays nothing out, so the click lands on the first line: where the bar is.
    bar.element.parentElement!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, clientY: 0 }))
    await flushPromises()
    expect(w.find('.change-card').exists()).toBe(true)
    expect(w.emitted('bar')).toBeUndefined()
    // The old behaviour is one button away.
    await w.find('[data-act="diff"]').trigger('click')
    expect(w.emitted('bar')?.[0]).toEqual([1])
  })

  it('shows who wrote the caret line when blame is on', async () => {
    const runs = [
      { oid: 'a'.repeat(40), short: 'aaaaaaa', summary: 'Tax rates', author: 'Sam', email: 's@x', time: Math.floor(Date.now() / 1000) - 60, start: 1, lines: 2, uncommitted: false }
    ]
    const w = await show({ runs, blame: true })
    expect(w.find('.cm-blame').text()).toContain('Sam')
  })

  it('lands on the line a jump asked for', async () => {
    tabs.goToLine(2)
    const w = await show()
    const view = editorOf(w)
    expect(view.state.doc.lineAt(view.state.selection.main.head).number).toBe(2)
  })
})

function gate() {
  let open = () => undefined as void
  const promise = new Promise<void>((done) => (open = done))
  return { promise, open }
}

describe('what the audit found', () => {
  it('never rewrites a CRLF file it was only shown', async () => {
    disk = { text: 'a\r\nb\r\n', hash: 'h1' }
    const w = await show()
    window.dispatchEvent(new Event('blur'))
    w.unmount()
    wrapper = null
    await flushPromises()
    expect(writes()).toHaveLength(0)
  })

  it('saves to the repository the edits were made in, after a switch to another', async () => {
    aimAt('/repos/shop')
    const w = await show()
    type(w, 'mine ')
    aimAt('/repos/other')
    w.unmount()
    wrapper = null
    await flushPromises()
    expect(writes()).toHaveLength(1)
    expect(writes()[0]!.args.__repo).toBe('/repos/shop')
    // Kept apart by repository: the other project has nothing of it.
    expect(saveStates[editKey('/repos/other', TAB)]).toBeUndefined()
  })

  it('makes the unmount save from the hash a save still under way leaves', async () => {
    const w = await show()
    type(w, 'one ')
    const held = gate()
    writeGate = held.promise
    const first = (w.vm as unknown as { save: () => Promise<void> }).save()
    await flushPromises()
    type(w, 'two ')
    w.unmount()
    wrapper = null
    writeGate = null
    held.open()
    await first
    await flushPromises()
    expect(writes()).toHaveLength(2)
    expect(disk.text).toBe('two one <?php\nreturn 21;\n')
    expect(saveStates[KEY()]).toBe('saved')
  })

  it('asks before closing a tab whose edits are in conflict, and keeps it on no', async () => {
    const w = await show()
    type(w, 'mine ')
    disk = { text: 'theirs\n', hash: 'h9' }
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    expect(saveStates[KEY()]).toBe('conflict')
    answer.ask = false
    const closed = await closeTabsSafely([TAB], () => tabs.closeTab(TAB))
    expect(closed).toBe(false)
    expect(answer.asked[0]).toContain('app/Tax.php')
    expect(tabs.state.tabs).toHaveLength(1)

    answer.ask = true
    await closeTabsSafely([TAB], () => {
      tabs.closeTab(TAB)
      w.unmount()
      wrapper = null
    })
    await flushPromises()
    expect(drafts.size).toBe(0)
    expect(disk.text).toBe('theirs\n')
    expect(saveStates[KEY()]).toBeUndefined()
  })

  it('asks before quitting with edits that cannot be saved, and saves the rest', async () => {
    const w = await show()
    type(w, 'fine ')
    expect(await prepareToQuit(500)).toBe(true)
    expect(disk.text).toBe('fine <?php\nreturn 21;\n')
    expect(answer.asked).toHaveLength(0)

    type(w, 'stuck ')
    disk = { text: 'theirs\n', hash: 'h9' }
    answer.ask = false
    expect(await prepareToQuit(500)).toBe(false)
    expect(answer.asked[0]).toContain('app/Tax.php')
  })

  it('goes to a jump once, and back to where it was left after that', async () => {
    tabs.goToLine(2)
    const w = await show()
    expect(tabs.state.landing).toBeNull()
    w.unmount()
    // Left with the caret somewhere else since.
    tabs.state.tabs[0]!.caret = 1
    wrapper = await show()
    expect(editorOf(wrapper).state.selection.main.head).toBe(1)
  })

  it('keeps a preview tab once something is typed in it', async () => {
    resetTabs()
    switchTabs('/repos/shop')
    openTab({ path: PATH, mode: 'file' }, { preview: true })
    const w = await show()
    editorOf(w).dispatch({ selection: { anchor: 0 } })
    expect(tabs.state.tabs[0]!.preview).toBe(true)
    type(w, 'x')
    expect(tabs.state.tabs[0]!.preview).toBe(false)
  })

  it('leaves a draft alone when it is closed again while an earlier save runs', async () => {
    const held = gate()
    const key = KEY()
    drafts.set(key, { repo: aimedAt(), tab: TAB, path: PATH, text: 'draft\n', baseHash: 'h1', baseText: disk.text })
    flushing.set(key, held.promise)
    const w = mount(FileEditor, {
      props: { tabId: TAB, path: PATH, hunks: [], runs: null, blame: false, find: null, fallback: null }
    })
    w.unmount()
    held.open()
    await flushPromises()
    expect(drafts.get(key)?.text).toBe('draft\n')
  })

  it('does not take a read from before its own save landed', async () => {
    const w = await show()
    const held = gate()
    readGate = held.promise
    diskChanged?.({ payload: { work_tree: true } })
    await wait(150)
    // The read is out with the old text; meanwhile a save lands.
    readGate = null
    type(w, 'new ')
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    held.open()
    await flushPromises()
    expect(editorOf(w).state.doc.toString()).toBe('new <?php\nreturn 21;\n')
  })

  it('is saved, not in conflict, when kept text is the same as the disk', async () => {
    const w = await show()
    type(w, 'same ')
    disk = { text: 'same <?php\nreturn 21;\n', hash: 'h9' }
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    await flushPromises()
    await w.findAll('.disk-bar button').find((one) => one.text() === 'Keep mine')!.trigger('click')
    await flushPromises()
    expect(saveStates[KEY()]).toBe('saved')
    expect(w.find('.disk-bar').exists()).toBe(false)
  })

  it('keeps the edits and the question when writing a deleted file again fails', async () => {
    const w = await show()
    type(w, 'mine ')
    readExtra = { missing: true }
    refuse = 'Conflict: deleted'
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    refuse = 'Permission denied'
    await w.findAll('.disk-bar button').find((one) => one.text() === 'Keep mine')!.trigger('click')
    await flushPromises()
    expect(saveStates[KEY()]).toBe('conflict')
    expect(w.find('.disk-bar').exists()).toBe(true)
    // Left now, the edits stay as a draft rather than going with the editor.
    w.unmount()
    wrapper = null
    await flushPromises()
    expect(drafts.get(KEY())?.text).toBe('mine <?php\nreturn 21;\n')
  })

  it('leaves its caret on its own project’s tab after a switch', async () => {
    aimAt('/repos/shop')
    const w = await show()
    editorOf(w).dispatch({ selection: { anchor: 3 } })
    switchTabs('/repos/other')
    openTab({ path: PATH, mode: 'file' })
    aimAt('/repos/other')
    w.unmount()
    wrapper = null
    expect(findTab('/repos/shop', TAB)?.caret).toBe(3)
    expect(tabs.state.tabs[0]!.caret).toBeUndefined()
  })

  it('recreates a deleted file the way it was written', async () => {
    readExtra = { eol: 'crlf', bom: true }
    const w = await show()
    type(w, 'mine ')
    readExtra = { missing: true }
    refuse = 'Conflict: deleted'
    await (w.vm as unknown as { save: () => Promise<void> }).save()
    refuse = null
    disk = { text: '', hash: '' }
    asked.mockImplementationOnce(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} })
      return 'h-new'
    })
    await w.findAll('.disk-bar button').find((one) => one.text() === 'Keep mine')!.trigger('click')
    await flushPromises()
    expect(writes().at(-1)!.args).toMatchObject({ expectedHash: null, eol: 'crlf', bom: true })
  })
})

describe('names in the editor', () => {
  // <?php\nreturn 21;\n — "return" is 6..12.
  const flush = () => new Promise((done) => setTimeout(done, 20))

  it('dims what nothing uses, hints at public code, and badges what is new', async () => {
    unused = [
      { name: 'x', kind: 'variable', level: 'dim', line: 2, from: 6, to: 12, block_from: 6, block_to: 12, new: false },
      { name: 'y', kind: 'function', level: 'hint', line: 1, from: 0, to: 5, block_from: 0, block_to: 16, new: true }
    ]
    const w = await show()
    await flush()
    await flushPromises()
    expect(calls.find((one) => one.cmd === 'unused_in')?.args).toMatchObject({ path: PATH, text: disk.text })
    expect(w.find('.cm-unused').text()).toBe('return')
    expect(w.find('.cm-unused-hint').text()).toBe('<?php')
    expect(w.find('.cm-badge').text()).toBe('new · 0 usages')
  })

  it('shows usage counts over definitions, and lists them on a click', async () => {
    lenses = [{ name: 'total', kind: 'method', line: 2, from: 6, count: 3 }]
    const w = await show()
    await flush()
    await flushPromises()
    const lens = w.find('.cm-lens')
    expect(lens.text()).toBe('3 usages')
    lens.element.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    await flushPromises()
    expect(calls.find((one) => one.cmd === 'usages')?.args).toEqual({ name: 'total', path: PATH, kind: 'method', line: 2 })
  })

  it('shows possible usages muted, and what the framework calls by the uses of its class', async () => {
    lenses = [
      { name: 'total', kind: 'method', line: 2, from: 6, count: 2, possible: 50, framework: false, owner: null },
      { name: 'handle', kind: 'method', line: 3, from: 20, count: 2, possible: 0, framework: true, owner: 'ExportAnalyticsCommand' },
      { name: 'boot', kind: 'method', line: 4, from: 40, count: 0, possible: 0, framework: true, owner: null }
    ]
    disk = { text: '<?php\nfunction total() {}\nfunction handle() {}\nfunction boot() {}\n', hash: 'h1' }
    const w = await show()
    await flush()
    await flushPromises()
    const texts = w.findAll('.cm-lens').map((lens) => lens.text())
    expect(texts).toEqual(['2 usages · 50 possible', '2 uses of ExportAnalyticsCommand', 'called by the framework'])
    expect(w.find('.cm-lens-possible').text()).toBe('· 50 possible')
  })

  it('leaves the counts out when settings turn them off', async () => {
    countsOff = true
    await useConfig().load()
    lenses = [{ name: 'total', kind: 'method', line: 2, from: 6, count: 3 }]
    const w = await show()
    await flush()
    await flushPromises()
    expect(calls.some((one) => one.cmd === 'usage_counts')).toBe(false)
    expect(w.find('.cm-lens').exists()).toBe(false)
    countsOff = false
    await useConfig().load()
  })

  it('asks what is at the caret on ⌘J, with the text as typed', async () => {
    const w = await show()
    type(w, 'x')
    editorOf(w).dispatch({ selection: { anchor: 9 } })
    ;(w.vm as unknown as { jumpAtCaret: () => void }).jumpAtCaret()
    await flushPromises()
    expect(calls.find((one) => one.cmd === 'symbol_at')?.args).toEqual({
      path: PATH,
      offset: 9,
      text: 'x<?php\nreturn 21;\n'
    })
  })

  it('asks nothing about a file in a language it cannot read', async () => {
    await show({ path: 'notes.md' })
    await flush()
    expect(calls.some((one) => one.cmd === 'unused_in')).toBe(false)
  })
})

describe('keys with the caret in the editor', () => {
  it('leaves ⌘Z to the editor but lets back and forward through', async () => {
    const undo = vi.fn()
    const back = vi.fn()
    const keys = mount(
      defineComponent({
        setup() {
          useShortcuts({ 'history.undo': undo, 'nav.back': back })
          return () => h('div')
        }
      })
    )
    const w = await show()
    const isMac = navigator.userAgent.includes('Mac')
    const content = w.find('.cm-content').element
    content.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'z', metaKey: isMac, ctrlKey: !isMac, bubbles: true, cancelable: true })
    )
    content.dispatchEvent(
      new KeyboardEvent('keydown', { key: '[', metaKey: isMac, ctrlKey: !isMac, bubbles: true, cancelable: true })
    )
    expect(undo).not.toHaveBeenCalled()
    expect(back).toHaveBeenCalledTimes(1)
    keys.unmount()
  })

  it('lets the next changed file through, which CodeMirror would add a cursor with', async () => {
    const next = vi.fn()
    const keys = mount(
      defineComponent({
        setup() {
          useShortcuts({ 'work.next': next })
          return () => h('div')
        }
      })
    )
    const w = await show()
    const isMac = navigator.userAgent.includes('Mac')
    w.find('.cm-content').element.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'ArrowDown',
        code: 'ArrowDown',
        altKey: true,
        metaKey: isMac,
        ctrlKey: !isMac,
        bubbles: true,
        cancelable: true
      })
    )
    expect(next).toHaveBeenCalledTimes(1)
    expect(editorOf(w).state.selection.ranges).toHaveLength(1)
    keys.unmount()
  })
})

describe('comparing your text with the disk’s', () => {
  it('lines up what both have and makes a region of each difference', () => {
    const blocks = compareBlocks('a\nmine\nc\nd', 'a\ntheirs\nc\nd\ne')
    expect(blocks.map((one) => one.kind)).toEqual(['context', 'conflict', 'context', 'conflict'])
    const [, first, , last] = blocks as never[] as { ours: string[]; theirs: string[] }[]
    expect(first).toMatchObject({ ours: ['mine'], theirs: ['theirs'] })
    expect(last).toMatchObject({ ours: [], theirs: ['e'] })
  })
})
