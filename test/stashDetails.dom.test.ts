// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import CommitDetails from '~/components/CommitDetails.vue'
import { useGit, type CommitDetail } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const DETAIL: CommitDetail = {
  oid: 's1',
  short: 's1',
  summary: 'the half-finished idea',
  body: '',
  author: 'Ramon Robben',
  email: 'r@x',
  time: 1756000000,
  committer: 'Ramon Robben',
  commit_time: 1756000000,
  parents: ['c1'],
  files: [
    { path: 'report.js', old_path: null, status: 'modified', additions: 4, deletions: 1, binary: false }
  ]
}

let open: ReturnType<typeof mount> | null = null

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    // Every action refreshes; the stash is still on the list after an apply,
    // so the strip has to still be there for the next press.
    if (cmd === 'stash_list') return git.store.stashes
    return null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.detail = DETAIL
  git.store.stashes = [
    { index: 0, oid: 's1', message: 'the half-finished idea', branch: 'main', time: 1756000000, files: 1 }
  ]
})

afterEach(() => {
  open?.unmount()
  open = null
})

const show = async () => {
  open = mount(CommitDetails, { global: { stubs: { Avatar: true, Spinner: true } } })
  await flushPromises()
  return open
}

describe('a stash selected like any other commit', () => {
  it('says it is one, and where it was made', async () => {
    const wrapper = await show()
    expect(wrapper.find('.stash-strip').text()).toContain('A stash, made on main')
  })

  it('offers the four things you can do with it', async () => {
    const wrapper = await show()
    const labels = wrapper.findAll('.stash-strip .tiny').map((b) => b.text())
    expect(labels).toEqual(['Apply', 'Pop', 'Branch', 'Drop'])
  })

  it('applies and pops by the position it is at', async () => {
    const wrapper = await show()
    await wrapper.findAll('.stash-strip .tiny')[0]!.trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'stash_apply')?.args).toEqual({ index: 0 })

    await wrapper.findAll('.stash-strip .tiny')[1]!.trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'stash_pop')?.args).toEqual({ index: 0 })
  })

  it('takes a name before turning one into a branch', async () => {
    const wrapper = await show()
    await wrapper.findAll('.stash-strip .tiny')[2]!.trigger('click')
    await wrapper.find('.stash-strip input').setValue('rescue/the-idea')
    await wrapper.find('.stash-strip .primary').trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'stash_branch')?.args).toEqual({
      index: 0,
      name: 'rescue/the-idea'
    })
  })

  it('says nothing of the sort for an ordinary commit', async () => {
    git.store.detail = { ...DETAIL, oid: 'c1' }
    const wrapper = await show()
    expect(wrapper.find('.stash-strip').exists()).toBe(false)
  })
})
