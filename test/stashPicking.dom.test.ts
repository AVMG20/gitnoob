// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SideBar from '~/components/SideBar.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useGit, type StashEntry, type StashRun } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let run: StashRun = { applied: [], stopped: null, conflicted: [] }

const stash = (index: number, message: string): StashEntry => ({
  index,
  oid: `oid${index}`,
  message,
  branch: 'main',
  time: 1756000000 - index * 100,
  files: 1
})

const Host = {
  components: { SideBar, ContextMenu },
  template: '<div><SideBar /><ContextMenu /></div>'
}

let open: ReturnType<typeof mount> | null = null

beforeEach(() => {
  calls = []
  run = { applied: [], stopped: null, conflicted: [] }
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'stash_apply_many') return run
    if (cmd === 'stash_oid') return 'oid0'
    return null
  })
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.log.length = 0
  git.store.stashView = null
  git.store.stashes = [stash(0, 'newest'), stash(1, 'middle'), stash(2, 'oldest')]
})

afterEach(() => {
  open?.unmount()
  open = null
})

const show = () => {
  open = mount(Host, { global: { components: { MidTruncate }, stubs: { Teleport: true } } })
  return open
}

const rows = (wrapper: ReturnType<typeof show>) =>
  wrapper.findAll('.row.stash').filter((row) => row.text().match(/newest|middle|oldest/))

describe('picking several stashes', () => {
  it('opens one on a plain click, and picks none', async () => {
    const wrapper = show()
    await rows(wrapper)[0]!.trigger('click')
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'stash_oid')).toBe(true)
    expect(wrapper.find('.picked-bar').exists()).toBe(false)
  })

  it('gathers them with ctrl-click, and says how many', async () => {
    const wrapper = show()
    await rows(wrapper)[0]!.trigger('click', { ctrlKey: true })
    await rows(wrapper)[2]!.trigger('click', { ctrlKey: true })
    expect(wrapper.find('.picked-bar').text()).toContain('2 picked')
    expect(wrapper.findAll('.row.stash.ticked')).toHaveLength(2)
  })

  it('takes the run between two with shift', async () => {
    const wrapper = show()
    await rows(wrapper)[0]!.trigger('click', { ctrlKey: true })
    await rows(wrapper)[2]!.trigger('click', { shiftKey: true })
    expect(wrapper.findAll('.row.stash.ticked')).toHaveLength(3)
  })

  it('applies the picked ones together, keeping them', async () => {
    run = { applied: ['oldest', 'newest'], stopped: null, conflicted: [] }
    const wrapper = show()
    await rows(wrapper)[0]!.trigger('click', { ctrlKey: true })
    await rows(wrapper)[2]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.pick-btn').find((b) => b.text() === 'Apply')!.trigger('click')
    await flushPromises()

    expect(calls.find((c) => c.cmd === 'stash_apply_many')?.args).toEqual({
      indexes: [0, 2],
      dropAfter: false
    })
    expect(git.store.log.some((line) => line.text.includes('Applied oldest, newest'))).toBe(true)
  })

  it('pops them when asked to', async () => {
    run = { applied: ['oldest', 'newest'], stopped: null, conflicted: [] }
    const wrapper = show()
    await rows(wrapper)[0]!.trigger('click', { ctrlKey: true })
    await rows(wrapper)[2]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.pick-btn').find((b) => b.text() === 'Pop')!.trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'stash_apply_many')?.args).toMatchObject({ dropAfter: true })
  })

  it('says which one stopped a run, and keeps the picks so it can be retried', async () => {
    run = {
      applied: ['oldest'],
      stopped: { message: 'newest', reason: 'error: Your local changes would be overwritten' },
      conflicted: ['report.js']
    }
    const wrapper = show()
    await rows(wrapper)[0]!.trigger('click', { ctrlKey: true })
    await rows(wrapper)[2]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.pick-btn').find((b) => b.text() === 'Apply')!.trigger('click')
    await flushPromises()

    const failure = git.store.log.find((line) => line.level === 'error')
    expect(failure?.text).toContain('Stopped at "newest"')
    expect(failure?.text).toContain('1 file conflicted')
    expect(wrapper.find('.picked-bar').exists()).toBe(true)
  })

  it('forgets a pick whose stash is no longer in the list', async () => {
    const wrapper = show()
    await rows(wrapper)[0]!.trigger('click', { ctrlKey: true })
    await rows(wrapper)[1]!.trigger('click', { ctrlKey: true })
    expect(wrapper.findAll('.row.stash.ticked')).toHaveLength(2)

    git.store.stashes = [stash(0, 'middle')]
    await flushPromises()
    expect(wrapper.find('.picked-bar').exists()).toBe(false)
  })
})
