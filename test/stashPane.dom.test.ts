// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import StashPane from '~/components/StashPane.vue'
import { useGit, type CommitDetail, type StashEntry } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const DETAIL: CommitDetail = {
  oid: 'stash1',
  short: 'stash1',
  summary: 'the half-finished idea',
  body: '',
  author: 'Ramon Robben',
  email: 'r@x',
  time: 1756000000,
  committer: 'Ramon Robben',
  commit_time: 1756000000,
  parents: [],
  files: [
    { path: 'report.js', old_path: null, status: 'modified', additions: 4, deletions: 1, binary: false },
    { path: 'notes.txt', old_path: null, status: 'added', additions: 2, deletions: 0, binary: false }
  ]
}

const stash = (over: Partial<StashEntry> = {}): StashEntry => ({
  index: 0,
  oid: 'stash1',
  message: 'the half-finished idea',
  branch: 'main',
  time: 1756000000,
  files: 2,
  ...over
})

let open: ReturnType<typeof mount> | null = null

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'commit_detail') return DETAIL
    return 'done'
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.stashes = [stash(), stash({ index: 1, oid: 'stash2', message: 'another one' })]
  git.store.stashView = 'stash1'
  git.store.viewer = null
})

afterEach(() => {
  open?.unmount()
  open = null
})

const show = async () => {
  open = mount(StashPane)
  await flushPromises()
  return open
}

describe('a stash in the content view', () => {
  it('names it, and lists what it holds', async () => {
    const wrapper = await show()
    expect(wrapper.find('h2').text()).toBe('the half-finished idea')
    expect(wrapper.find('.sub').text()).toContain('on main')
    expect(wrapper.find('.sub').text()).toContain('2 files')
    expect(wrapper.findAll('.file')).toHaveLength(2)
    expect(wrapper.findAll('.file')[0]!.text()).toContain('report.js')
    expect(wrapper.findAll('.file')[0]!.text()).toContain('+4')
  })

  it('reads its own commit rather than whatever the graph last selected', async () => {
    await show()
    expect(calls.find((c) => c.cmd === 'commit_detail')?.args).toEqual({ oid: 'stash1' })
  })

  it('opens a file at the stash it belongs to', async () => {
    const wrapper = await show()
    await wrapper.findAll('.file')[0]!.trigger('click')
    expect(git.store.viewer).toEqual({ path: 'report.js', commit: 'stash1' })
  })

  it('applies without taking it off the list, and pops with', async () => {
    const wrapper = await show()
    const buttons = wrapper.findAll('.foot .btn')
    await buttons.find((b) => b.text().includes('Apply'))!.trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'stash_apply')?.args).toEqual({ index: 0 })

    await buttons.find((b) => b.text().includes('Pop'))!.trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'stash_pop')?.args).toEqual({ index: 0 })
  })

  it('closes itself when the stash it was showing is dropped', async () => {
    const wrapper = await show()
    await wrapper.find('.foot .danger').trigger('click')
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'stash_drop')).toBe(true)
    expect(git.store.stashView).toBeNull()
  })

  it('goes when the stash leaves the list some other way', async () => {
    await show()
    git.store.stashes = [stash({ index: 0, oid: 'stash2', message: 'another one' })]
    await flushPromises()
    expect(git.store.stashView).toBeNull()
  })

  it('takes a name before turning one into a branch', async () => {
    const wrapper = await show()
    await wrapper.findAll('.foot .btn').find((b) => b.text().includes('Branch'))!.trigger('click')
    await wrapper.find('.branching input').setValue('rescue/the-idea')
    await wrapper.find('.branching .btn-primary').trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'stash_branch')?.args).toEqual({
      index: 0,
      name: 'rescue/the-idea'
    })
  })
})
