// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SideBar from '~/components/SideBar.vue'
import GraphList from '~/components/GraphList.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useGit, type LocalBranch, type RefTree } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()

const branch = (name: string, isHead: boolean): LocalBranch => ({
  name,
  oid: `${name}-oid`,
  is_head: isHead,
  upstream: `origin/${name}`,
  ahead: 0,
  behind: 0
})

const tree = (...names: string[]): RefTree => ({
  locals: names.map((name) => branch(name, name === 'main')),
  remotes: [{ name: 'serp', remote: 'origin', oid: 'serp-oid' }],
  tags: [],
  stashes: []
})

const Host = {
  components: { SideBar, GraphList, ContextMenu },
  template: '<div><SideBar /><GraphList /><ContextMenu /></div>'
}

let open: ReturnType<typeof mount> | null = null

const show = async () => {
  open = mount(Host, { global: { components: { MidTruncate }, stubs: { Teleport: true } } })
  await flushPromises()
  return open
}

const labels = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('.menu .item .label').map((one) => one.text())

const rowOf = (wrapper: ReturnType<typeof mount>, name: string) =>
  wrapper.findAll('.row').find((one) => one.find('.name').exists() && one.text().includes(name))!

async function pick(wrapper: ReturnType<typeof mount>, label: string) {
  const item = wrapper.findAll('.menu .item').find((one) => one.find('.label').text() === label)
  expect(item, `no "${label}" in ${labels(wrapper).join(', ')}`).toBeDefined()
  await item!.trigger('click')
  await flushPromises()
}

/** The solo the graph was last asked to draw. */
const lastSolo = () =>
  [...asked.mock.calls].reverse().find(([command]) => command === 'commit_graph')?.[1] as
    | { solo: string[] }
    | undefined

beforeEach(async () => {
  asked.mockReset()
  asked.mockImplementation(async () => null)
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.stashes = []
  git.store.rows = []
  git.store.trunk = { name: 'main', chosen: false }
  git.store.refs = tree('main', 'staging', 'tickets')
  await git.soloOn([])
  asked.mockClear()
})

afterEach(() => {
  open?.unmount()
  open = null
})

describe('soloing a branch', () => {
  it('draws only that branch, and quiets the rest of the list', async () => {
    const wrapper = await show()
    await rowOf(wrapper, 'staging').trigger('contextmenu')
    await flushPromises()
    expect(labels(wrapper)).toContain('Solo with main')

    await pick(wrapper, 'Solo in the graph')

    expect(lastSolo()?.solo).toEqual(['refs/heads/staging'])
    expect(rowOf(wrapper, 'staging').classes()).toContain('solo')
    expect(rowOf(wrapper, 'staging').classes()).not.toContain('dim')
    expect(rowOf(wrapper, 'tickets').classes()).toContain('dim')
    expect(rowOf(wrapper, 'main').classes()).toContain('dim')
  })

  it('can take the trunk along, to see where the branch forked', async () => {
    const wrapper = await show()
    await rowOf(wrapper, 'staging').trigger('contextmenu')
    await flushPromises()
    await pick(wrapper, 'Solo with main')
    expect(lastSolo()?.solo).toEqual(['refs/heads/staging', 'refs/heads/main'])
  })

  it('shows a way out in the graph header only while soloed', async () => {
    const wrapper = await show()
    expect(wrapper.find('.solo-tool').exists()).toBe(false)

    await git.soloOn(['refs/heads/staging', 'refs/remotes/origin/serp'])
    await flushPromises()
    const tool = wrapper.find('.solo-tool')
    expect(tool.attributes('title')).toContain('staging, origin/serp')

    await tool.trigger('click')
    await flushPromises()
    expect(lastSolo()?.solo).toEqual([])
    expect(wrapper.find('.solo-tool').exists()).toBe(false)
  })

  it('stops soloing on Escape', async () => {
    await git.soloOn(['refs/heads/staging'])
    await show()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await flushPromises()
    expect(git.store.solo).toEqual([])
  })

  it('offers the way out from the graph too', async () => {
    git.store.rows = [
      {
        oid: 'staging-oid',
        short: 'staging',
        summary: 'Work',
        author: 'Robin Vale',
        email: 'a@b.c',
        time: 0,
        parents: [],
        lane: 0,
        color: 0,
        width: 1,
        segments: [],
        labels: [{ kind: 'local', name: 'staging', head: false }],
        unpushed: false,
        unpulled: false,
        carries: [],
        stash: null
      }
    ]
    const wrapper = await show()
    await wrapper.find('.graph .row').trigger('contextmenu')
    await flushPromises()
    await pick(wrapper, 'Solo staging')
    expect(lastSolo()?.solo).toEqual(['refs/heads/staging'])

    await wrapper.find('.graph .row').trigger('contextmenu')
    await flushPromises()
    expect(labels(wrapper)).not.toContain('Solo staging')
    await pick(wrapper, 'Show every branch')
    expect(lastSolo()?.solo).toEqual([])
  })

  it('adds another branch from its menu while one is soloed', async () => {
    await git.soloOn(['refs/heads/staging'])
    const wrapper = await show()
    await rowOf(wrapper, 'tickets').trigger('contextmenu')
    await flushPromises()
    await pick(wrapper, 'Add to the solo')
    expect(lastSolo()?.solo).toEqual(['refs/heads/staging', 'refs/heads/tickets'])

    await rowOf(wrapper, 'staging').trigger('contextmenu')
    await flushPromises()
    await pick(wrapper, 'Solo just this one')
    expect(lastSolo()?.solo).toEqual(['refs/heads/staging'])

    await rowOf(wrapper, 'staging').trigger('contextmenu')
    await flushPromises()
    await pick(wrapper, 'Stop soloing')
    expect(lastSolo()?.solo).toEqual([])
  })

  it('solos a remote branch by its remote ref', async () => {
    const wrapper = await show()
    await rowOf(wrapper, 'serp').trigger('contextmenu')
    await flushPromises()
    await pick(wrapper, 'Solo in the graph')
    expect(lastSolo()?.solo).toEqual(['refs/remotes/origin/serp'])
  })

  it('solos on an alt-click, and lets go on the next one', async () => {
    const wrapper = await show()
    await rowOf(wrapper, 'tickets').trigger('click', { altKey: true })
    await flushPromises()
    expect(git.store.solo).toEqual(['refs/heads/tickets'])

    await rowOf(wrapper, 'tickets').trigger('click', { altKey: true })
    await flushPromises()
    expect(git.store.solo).toEqual([])
  })

  it('asks for the soloed rows when looking for a commit', async () => {
    await git.soloOn(['refs/heads/staging'])
    await git.revealCommit('tickets-oid')
    const depth = asked.mock.calls.find(([command]) => command === 'commit_depth')
    expect((depth?.[1] as { solo: string[] }).solo).toEqual(['refs/heads/staging'])
  })

  it('lets go of a soloed branch once it is gone', async () => {
    await git.soloOn(['refs/heads/staging', 'refs/heads/tickets'])
    asked.mockImplementation(async (command) =>
      command === 'ref_tree' ? tree('main', 'tickets') : null
    )
    await git.refresh()
    await flushPromises()
    expect(git.store.solo).toEqual(['refs/heads/tickets'])
  })

  it('keeps each tab its own solo', async () => {
    await git.soloOn(['refs/heads/staging'])
    asked.mockImplementation(async (command, args) =>
      command === 'open_repo'
        ? { path: (args as { path: string }).path, name: 'x', head: 'main', detached: false }
        : null
    )
    await git.openRepo('/other')
    expect(git.store.solo).toEqual([])
    await git.openRepo('/repo')
    expect(git.store.solo).toEqual(['refs/heads/staging'])
  })
})
