// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SideBar from '~/components/SideBar.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useGit, type LocalBranch } from '~/composables/useGit'
import { useDragDrop } from '~/composables/useDragDrop'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let answers: Record<string, unknown> = {}
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const branch = (name: string, over: Partial<LocalBranch> = {}): LocalBranch => ({
  name,
  oid: `${name}-oid`,
  is_head: false,
  upstream: `origin/${name}`,
  ahead: 0,
  behind: 0,
  ...over
})

const Host = {
  components: { SideBar, ContextMenu },
  template: '<div><SideBar /><ContextMenu /></div>'
}

let open: ReturnType<typeof mount> | null = null
const show = () => {
  open = mount(Host, {
    global: { components: { MidTruncate }, stubs: { Teleport: true, BranchDialog: true, TidyDialog: true } }
  })
  return open
}

const labels = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('.menu .item .label').map((one) => one.text())

const row = (wrapper: ReturnType<typeof mount>, name: string) =>
  wrapper.findAll('.row').find((one) => one.find('.name').exists() && one.text().includes(name))!

beforeEach(() => {
  answers = {}
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return answers[cmd] ?? null
  })
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'feature', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.stashes = []
  git.store.tidy = []
  git.store.trunk = { name: 'main', chosen: false }
  git.store.refs = {
    locals: [
      branch('feature', { is_head: true }),
      branch('main'),
      branch('old', { upstream: null, gone: 'origin/old' })
    ],
    remotes: [{ name: 'main', remote: 'origin', oid: 'm' }],
    tags: [],
    stashes: []
  }
})

afterEach(() => {
  open?.unmount()
  open = null
})

describe('keeping branches in step, from the sidebar', () => {
  it('offers to update a branch from main, and not main from itself', async () => {
    const wrapper = show()
    await flushPromises()
    await row(wrapper, 'feature').trigger('contextmenu')
    await flushPromises()
    expect(labels(wrapper)).toContain('Update from main')

    await row(wrapper, 'main').trigger('contextmenu')
    await flushPromises()
    expect(labels(wrapper)).not.toContain('Update from main')
  })

  it('says a push of a branch whose upstream was deleted puts it back', async () => {
    const wrapper = show()
    await flushPromises()
    await row(wrapper, 'old').trigger('contextmenu')
    await flushPromises()
    expect(labels(wrapper)).toContain('Push to recreate origin/old')
  })

  it('starts new work from a remote branch', async () => {
    const wrapper = show()
    await flushPromises()
    const remote = wrapper.findAll('.row.indent').find((one) => one.text().includes('main'))!
    await remote.trigger('contextmenu')
    await flushPromises()
    expect(labels(wrapper)).toContain('New branch from here…')
  })

  it('shows the tidy strip until it is dismissed', async () => {
    git.store.tidy = [
      { name: 'old', oid: 'o', reason: 'gone', into: null, gone: 'origin/old', proven: false }
    ]
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.tidy-strip').text()).toMatch(/1 branch merged or gone/)
    await wrapper.find('.tidy-close').trigger('click')
    expect(wrapper.find('.tidy-strip').exists()).toBe(false)
  })

  it('says before the drop whether the merge would conflict', async () => {
    answers.branch_relation = { ahead: 2, behind: 1, clean: false, conflicts: ['a[b].txt', 'b.txt'] }
    const wrapper = show()
    await flushPromises()
    useDragDrop().state.payload = { kind: 'branch', name: 'feature', remote: false } as never
    await row(wrapper, 'main').trigger('drop')
    await flushPromises()
    expect(wrapper.find('.menu .title').text()).toBe('2 ahead · 1 behind · conflicts in 2 files')
    // Read-only: nothing but the relation was asked.
    expect(calls.map((c) => c.cmd)).not.toContain('merge_into')
  })
})
