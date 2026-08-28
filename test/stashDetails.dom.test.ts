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
    expect(wrapper.find('.stash-note').text()).toContain('A stash, made on main')
  })

  it('points at the bar rather than offering the same buttons twice', async () => {
    const wrapper = await show()
    expect(wrapper.find('.stash-note').text()).toContain('from the bar above')
    expect(wrapper.find('.stash-note button').exists()).toBe(false)
  })

  it('still counts what the stash holds, the way a commit does', async () => {
    const wrapper = await show()
    expect(wrapper.text()).toContain('1 file')
    expect(wrapper.text()).toContain('+4')
    expect(wrapper.text()).toContain('−1')
  })

  it('says nothing of the sort for an ordinary commit', async () => {
    git.store.detail = { ...DETAIL, oid: 'c1' }
    const wrapper = await show()
    expect(wrapper.find('.stash-note').exists()).toBe(false)
  })
})
