// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { EditorView } from '@codemirror/view'
import { syntaxTree } from '@codemirror/language'
import FileEditor from '~/components/FileEditor.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit } from '~/composables/useGit'
import { closeTab, openTab, resetTabs, switchTabs, tabId } from '~/composables/useTabs'
import { editKey, prepareToQuit, resetEdits, saveStates } from '~/composables/useFileEdit'
import { aimAt, aimedAt } from '~/composables/useInvoke'
import { isWarm, warmCount, workTreeMoved } from '~/composables/editorBuffers'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => true) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

/**
 * Switching tabs keeps the editor's state (`editorBuffers.ts`): coming back
 * takes it up again without reading the file, with its undo history, its
 * parse tree and its place.
 */

const asked = vi.mocked(invoke)
const git = useGit()
const A = 'app/Tax.php'
const B = 'app/Other.php'

let disk: Record<string, { text: string; hash: string }> = {}
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let writeGate: Promise<void> | null = null
let wrapper: ReturnType<typeof mount> | null = null

beforeEach(() => {
  resetTabs()
  resetEdits()
  switchTabs('/repos/shop')
  aimAt('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as never
  openTab({ path: A, mode: 'file' })
  openTab({ path: B, mode: 'file' })
  disk = {
    [A]: { text: '<?php\nreturn 21;\n', hash: 'a1' },
    [B]: { text: '<?php\nreturn 7;\n', hash: 'b1' }
  }
  calls = []
  writeGate = null
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    const path = String(args?.path ?? '')
    if (cmd === 'read_file') {
      const now = disk[path]!
      return { text: now.text, hash: now.hash, eol: 'lf', bom: false, readonly: null, missing: false }
    }
    if (cmd === 'write_file') {
      if (writeGate) await writeGate
      if (args?.expectedHash !== disk[path]!.hash) throw new Error('Conflict: changed on disk')
      disk[path] = { text: String(args?.text), hash: `w${calls.length}` }
      return disk[path]!.hash
    }
    if (cmd === 'unused_in' || cmd === 'usage_counts') return []
    if (cmd === 'config_get') return { global: {}, profiles: [] }
    return null
  })
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
})

async function show(path: string) {
  wrapper = mount(FileEditor, {
    attachTo: document.body,
    props: { tabId: tabId(path), path, hunks: [], runs: null, blame: false, find: null, fallback: null },
    global: { components: { AppModal } }
  })
  await flushPromises()
  return wrapper
}

function away() {
  wrapper?.unmount()
  wrapper = null
}

const editorOf = (w: ReturnType<typeof mount>) =>
  EditorView.findFromDOM(w.find('.cm-editor').element as HTMLElement)!
const reads = (path: string) => calls.filter((one) => one.cmd === 'read_file' && one.args.path === path).length
const writes = () => calls.filter((one) => one.cmd === 'write_file')
const wait = (ms: number) => new Promise((done) => setTimeout(done, ms))

function press(w: ReturnType<typeof mount>, key: string, shift = false) {
  const isMac = navigator.userAgent.includes('Mac')
  w.find('.cm-content').element.dispatchEvent(
    new KeyboardEvent('keydown', { key, metaKey: isMac, ctrlKey: !isMac, shiftKey: shift, bubbles: true, cancelable: true })
  )
}

describe('switching tabs keeps the editor', () => {
  it('comes back from another tab without reading the file, parsed, with its undo history', async () => {
    let w = await show(A)
    await wait(50)
    const view = editorOf(w)
    view.dispatch({ changes: { from: 0, insert: '// one\n' }, userEvent: 'input.type' })
    view.dispatch({ selection: { anchor: 3 } })
    const tree = syntaxTree(view.state)
    away()
    // The edit goes on saving without the editor.
    await flushPromises()
    expect(writes()).toHaveLength(1)
    expect(isWarm(tabId(A))).toBe(true)

    await show(B)
    away()
    const before = reads(A)
    w = await show(A)
    expect(reads(A)).toBe(before)
    const back = editorOf(w)
    expect(back.state.doc.toString()).toBe('// one\n<?php\nreturn 21;\n')
    expect(back.state.selection.main.head).toBe(3)
    // The same parse: nothing was read or parsed again.
    expect(syntaxTree(back.state)).toBe(tree)
    // Saved while away: the editor knows, and is not showing it as unsaved.
    expect(saveStates[editKey(aimedAt(), tabId(A))]).toBe('saved')

    // ⌘Z still undoes what was typed before the switch.
    press(w, 'z')
    expect(back.state.doc.toString()).toBe('<?php\nreturn 21;\n')
  })

  it('drops what it kept when the tab is closed', async () => {
    await show(A)
    away()
    expect(isWarm(tabId(A))).toBe(true)
    closeTab(tabId(A))
    expect(isWarm(tabId(A))).toBe(false)
    expect(warmCount()).toBe(0)
  })

  it('checks the disk on the way back when something there moved meanwhile', async () => {
    await show(A)
    away()
    disk[A] = { text: '<?php\nreturn 42;\n', hash: 'a2' }
    workTreeMoved()
    const before = reads(A)
    const w = await show(A)
    await flushPromises()
    expect(reads(A)).toBe(before + 1)
    // Nothing was typed: it takes the new text quietly.
    expect(editorOf(w).state.doc.toString()).toBe('<?php\nreturn 42;\n')
    expect(w.find('.disk-bar').exists()).toBe(false)
  })

  it('still saves a background tab’s edits when the app quits', async () => {
    const w = await show(A)
    let release: () => void = () => undefined
    writeGate = new Promise((done) => (release = done))
    editorOf(w).dispatch({ changes: { from: 0, insert: 'x' }, userEvent: 'input.type' })
    away()
    await flushPromises()
    expect(saveStates[editKey(aimedAt(), tabId(A))]).toBe('saving')
    const quitting = prepareToQuit()
    release()
    expect(await quitting).toBe(true)
    expect(writes()[0]!.args.text).toBe('x<?php\nreturn 21;\n')
  })

  it('forgets every kept editor when another project comes to the front', async () => {
    await show(A)
    away()
    expect(warmCount()).toBe(1)
    switchTabs('/repos/other')
    expect(warmCount()).toBe(0)
  })

  it('is quicker to come back to than to open, on a long PHP file', async () => {
    const lines = ['<?php', 'namespace App;', '']
    for (let i = 0; lines.length < 3000; i++) {
      lines.push(`final class Thing${i}`, '{', `    public function total${i}(int $a): int`, '    {', `        return $a * ${i}; // sum`, '    }', '}', '')
    }
    disk[A] = { text: lines.join('\n'), hash: 'big' }
    // Opening once loads the PHP grammar, which the old way of switching back
    // had loaded too.
    await show(A)
    await wait(100)
    away()
    closeTab(tabId(A))
    openTab({ path: A, mode: 'file' })

    const coloured = async () => {
      const started = performance.now()
      const w = await show(A)
      for (let i = 0; i < 400; i++) {
        // Coloured: the highlighter has put its spans in the lines.
        if (w.element.querySelector('.cm-line span')) break
        await wait(1)
      }
      const took = performance.now() - started
      away()
      return took
    }
    // Opened fresh: read, parse, colour.
    const fresh = await coloured()
    // Come back to: taken up from what was kept.
    const kept = await coloured()
    console.info(`3,000-line PHP file: ${fresh.toFixed(1)} ms opened, ${kept.toFixed(1)} ms come back to`)
    expect(kept).toBeLessThan(fresh)
  })
})
