// @vitest-environment happy-dom
import { flushPromises } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { leaveApp, resetQuit, watchQuit } from '~/composables/useQuit'
import { resetEdits, setSaveState } from '~/composables/useFileEdit'
import { resetTabs } from '~/composables/useTabs'

const win = vi.hoisted(() => ({
  destroy: vi.fn(async () => undefined),
  closeHandler: null as null | ((event: { preventDefault: () => void }) => Promise<void>)
}))
const events = vi.hoisted(() => ({ quit: null as null | ((event: { payload: number }) => void) }))
const dialog = vi.hoisted(() => ({ answers: [] as ((ok: boolean) => void)[] }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    destroy: win.destroy,
    onCloseRequested: async (handler: typeof win.closeHandler) => {
      win.closeHandler = handler
      return () => undefined
    }
  })
}))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: number }) => void) => {
    if (name === 'quit-requested') events.quit = handler
    return () => undefined
  })
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  ask: vi.fn(() => new Promise<boolean>((done) => dialog.answers.push(done)))
}))

const asked = vi.mocked(invoke)

beforeEach(() => {
  resetQuit()
  resetEdits()
  resetTabs()
  win.destroy.mockClear()
  asked.mockClear()
  dialog.answers = []
})

afterEach(() => resetQuit())

describe('quitting', () => {
  it('listens, then tells the backend it is ready and whether it holds unsaved edits', async () => {
    await watchQuit()
    const calls = asked.mock.calls.map(([cmd, args]) => [cmd, args])
    expect(calls).toContainEqual(['quit_guard', { needed: false }])
    expect(calls.at(-1)).toEqual(['quit_ready', undefined])
    setSaveState(null, 'work:a.php', 'conflict')
    await flushPromises()
    expect(asked).toHaveBeenLastCalledWith('quit_guard', { needed: true })
  })

  it('acknowledges a quit, and asks once however often it is pressed', async () => {
    await watchQuit()
    setSaveState(null, 'work:a.php', 'conflict')
    events.quit!({ payload: 7 })
    expect(asked).toHaveBeenCalledWith('quit_heard', { ticket: 7 })
    await new Promise((done) => setTimeout(done, 20))
    // A second ⌘Q and the close button while the question is up.
    events.quit!({ payload: 8 })
    void win.closeHandler!({ preventDefault: () => undefined })
    void leaveApp()
    await new Promise((done) => setTimeout(done, 20))
    expect(dialog.answers).toHaveLength(1)

    // No: the window stays, and the next quit asks again.
    dialog.answers[0]!(false)
    await flushPromises()
    expect(win.destroy).not.toHaveBeenCalled()
    void leaveApp()
    await new Promise((done) => setTimeout(done, 20))
    expect(dialog.answers).toHaveLength(2)
    dialog.answers[1]!(true)
    await flushPromises()
    expect(win.destroy).toHaveBeenCalledTimes(1)
  })

  it('closes at once when nothing is unsaved', async () => {
    await watchQuit()
    await leaveApp()
    expect(dialog.answers).toHaveLength(0)
    expect(win.destroy).toHaveBeenCalledTimes(1)
  })
})
