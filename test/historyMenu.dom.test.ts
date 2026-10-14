// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import AppModal from '~/components/AppModal.vue'
import DropStashDialog from '~/components/DropStashDialog.vue'
import HistoryMenu from '~/components/HistoryMenu.vue'
import ResetDialog from '~/components/ResetDialog.vue'
import { useGit, type HistoryEntry, type LostStash, type Moment } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const now = Math.floor(Date.now() / 1000)

const step = (over: Partial<HistoryEntry> = {}): HistoryEntry => ({
  id: 1,
  label: 'Commit: Tidy the parser',
  kind: 'commit',
  branch: 'main',
  before: 'a'.repeat(40),
  after: 'b'.repeat(40),
  mode: 'soft',
  destructive: false,
  at: now - 60,
  ...over
})

const moment: Moment = {
  oid: 'c'.repeat(40),
  short: 'ccccccc',
  previous: 'd'.repeat(40),
  time: now - 3600,
  action: 'commit: Half a refactor',
  subject: 'Half a refactor',
  logs: ['HEAD', 'main']
}

const lostOne: LostStash = {
  oid: 'e'.repeat(40),
  short: 'eeeeeee',
  message: 'On main: the lost idea',
  time: now - 7200
}

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'reflog_moments') return [moment]
    if (cmd === 'lost_stashes') return [lostOne]
    if (cmd === 'reset_preview')
      return {
        target: moment.oid,
        short: moment.short,
        summary: moment.subject,
        branch: 'main',
        dropped: [],
        diverges: false,
        staged_files: 3,
        unstaged_files: 4
      }
    return 'done'
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.history = { undo: [step()], redo: [] }
})

let open: ReturnType<typeof mount> | null = null
afterEach(() => {
  open?.unmount()
  open = null
})

const show = () => {
  open = mount(HistoryMenu, {
    global: { components: { AppModal }, stubs: { Teleport: true } }
  })
  return open
}

describe('the history menu', () => {
  it('greys out a step that no longer fits, and offers to forget it', async () => {
    git.store.history = {
      undo: [step({ unavailable: 'main has moved since, outside this history' })],
      redo: []
    }
    const wrapper = show()

    const item = wrapper.find('.item')
    expect(item.attributes('disabled')).toBeDefined()
    expect(item.attributes('title')).toContain('main has moved since')

    await wrapper.find('.forget').trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'history_forget')?.args).toMatchObject({ id: 1 })
  })

  it('offers no forgetting for a step that still fits', () => {
    const wrapper = show()
    expect(wrapper.find('.forget').exists()).toBe(false)
    expect(wrapper.find('.item').attributes('disabled')).toBeUndefined()
  })

  it('reads the reflog only when more history is asked for', async () => {
    const wrapper = show()
    expect(calls.some((call) => call.cmd === 'reflog_moments')).toBe(false)

    await wrapper.find('.more').trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Half a refactor')
    expect(wrapper.text()).toContain('commit: Half a refactor')
    // Reading is all it did.
    expect(calls.map((call) => call.cmd)).toEqual(['reflog_moments'])
  })

  it('makes a branch at a moment without checking it out', async () => {
    const wrapper = show()
    await wrapper.find('.more').trigger('click')
    await flushPromises()

    await wrapper.findAll('.act').find((one) => one.text().includes('Branch'))!.trigger('click')
    await flushPromises()
    await wrapper.find('input[type="text"]').setValue('rescued')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(calls.find((call) => call.cmd === 'create_branch')?.args).toMatchObject({
      name: 'rescued',
      start: moment.oid,
      checkout: false
    })
  })

  it('resets to a moment only through the dialog, and keeps open work', async () => {
    const wrapper = show()
    await wrapper.find('.more').trigger('click')
    await flushPromises()

    await wrapper.findAll('.act').find((one) => one.text().includes('Reset'))!.trigger('click')
    await flushPromises()
    expect(calls.some((call) => call.cmd === 'reset')).toBe(false)

    const dialog = wrapper.findComponent(ResetDialog)
    expect(dialog.exists()).toBe(true)
    // Nothing is thrown away, so there is no tick to ask for.
    expect(dialog.find('.ack').exists()).toBe(false)
    expect(dialog.text()).toContain('uncommitted changes stay')

    await dialog.find('.btn-danger').trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'reset')?.args).toMatchObject({
      oid: moment.oid,
      mode: 'keep'
    })
  })

  it('looks for dropped stashes on request and puts one back', async () => {
    const wrapper = show()
    await wrapper.find('.more').trigger('click')
    await flushPromises()
    expect(calls.some((call) => call.cmd === 'lost_stashes')).toBe(false)

    await wrapper
      .findAll('.item')
      .find((one) => one.text().includes('Look for dropped stashes'))!
      .trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('On main: the lost idea')

    await wrapper.findAll('.act').find((one) => one.text() === 'Restore')!.trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'restore_stash')?.args).toMatchObject({
      oid: lostOne.oid
    })
    expect(wrapper.text()).not.toContain('On main: the lost idea')
  })
})

describe('dropping a stash', () => {
  it('says undo brings it back rather than that it is gone for good', () => {
    git.store.stashes = [
      { index: 0, oid: 's1', message: 'an idea', branch: 'main', time: now, files: 2 } as never
    ]
    open = mount(DropStashDialog, {
      props: { index: 0 },
      global: { components: { AppModal }, stubs: { Teleport: true } }
    })
    expect(open.text()).toContain('Undo puts it back')
    expect(open.text()).not.toContain('gone for good')
  })
})
