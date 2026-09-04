// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import GraphList from '~/components/GraphList.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import { useGit, type GraphRow } from '~/composables/useGit'
import { useBranchNaming } from '~/composables/useBranchNaming'
import { branchNameProblem } from '~/composables/useBranchName'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const naming = useBranchNaming()

const row = (oid: string, summary: string, over: Partial<GraphRow> = {}): GraphRow => ({
  oid,
  short: oid.slice(0, 7),
  summary,
  author: 'Robin Vale',
  email: 'a@b.c',
  time: Math.floor(Date.now() / 1000) - 60,
  parents: [],
  lane: 0,
  color: 0,
  width: 1,
  segments: [],
  labels: [],
  unpushed: false,
  unpulled: false,
  carries: [],
  stash: null,
  ...over
})

const Host = {
  components: { GraphList, ContextMenu },
  template: '<div><GraphList /><ContextMenu /></div>'
}

/** Right-clicks the row at `at` and picks "Branch from here". */
async function branchFrom(wrapper: ReturnType<typeof mount>, at: number) {
  await wrapper.findAll('.row')[at]!.trigger('contextmenu')
  await flushPromises()
  const item = wrapper
    .findAll('.menu .item')
    .find((one) => one.text().startsWith('Branch from here'))!
  await item.trigger('click')
  await flushPromises()
}

/** What was sent to the backend under `cmd`. */
function sent(cmd: string) {
  return asked.mock.calls.filter(([name]) => name === cmd).map(([, args]) => args)
}

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) =>
    cmd === 'create_branch' ? 'Switched to a new branch' : null
  )
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false, author: 'Robin Vale' } as never
  git.store.refs = { locals: [{ name: 'main', is_head: true }], remotes: [], tags: [], stashes: [] } as never
  git.store.rows = [
    row('aaaaaaa1', 'Newest', { labels: [{ kind: 'local', name: 'main', head: true }] }),
    row('bbbbbbb2', 'Older')
  ]
  git.store.status = { staged: [], unstaged: [], conflicted: [] }
})

describe('naming a branch in the graph', () => {
  it('opens an editor on the row instead of a dialog', async () => {
    const wrapper = mount(Host)
    await flushPromises()
    await branchFrom(wrapper, 1)

    expect(wrapper.find('.naming input').exists()).toBe(true)
    expect(wrapper.findAll('.row')[1]!.find('.naming').exists()).toBe(true)
    expect(wrapper.findAll('.row')[0]!.find('.naming').exists()).toBe(false)
    expect(document.querySelector('.scrim')).toBeNull()
  })

  it('creates and checks the branch out from that commit on Enter', async () => {
    const wrapper = mount(Host)
    await flushPromises()
    await branchFrom(wrapper, 1)

    const input = wrapper.find('.naming input')
    await input.setValue('feature/thing')
    await input.trigger('keydown', { key: 'Enter' })
    await flushPromises()

    expect(sent('create_branch')).toEqual([
      { name: 'feature/thing', start: 'bbbbbbb2', checkout: true }
    ])
    expect(wrapper.find('.naming').exists()).toBe(false)
  })

  it('refuses a name git would refuse, and says why', async () => {
    const wrapper = mount(Host)
    await flushPromises()
    await branchFrom(wrapper, 1)

    const input = wrapper.find('.naming input')
    await input.setValue('bad name')
    expect(wrapper.find('.naming-hint').text()).toContain('Git will not accept')
    await input.trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(sent('create_branch')).toEqual([])
    // Still open: the name is wrong, not the wish.
    expect(wrapper.find('.naming').exists()).toBe(true)

    await input.setValue('main')
    expect(wrapper.find('.naming-hint').text()).toContain('already exists')
  })

  it('forgets it on Escape without creating anything', async () => {
    const wrapper = mount(Host)
    await flushPromises()
    await branchFrom(wrapper, 1)

    const input = wrapper.find('.naming input')
    await input.setValue('half-typed')
    await input.trigger('keydown', { key: 'Escape' })
    await flushPromises()

    expect(wrapper.find('.naming').exists()).toBe(false)
    expect(sent('create_branch')).toEqual([])
  })

  it('takes the toolbar\'s request on the commit HEAD is on, from HEAD', async () => {
    const wrapper = mount(Host)
    await flushPromises()

    expect(naming.begin()).toBe(true)
    await flushPromises()
    expect(wrapper.findAll('.row')[0]!.find('.naming').exists()).toBe(true)

    const input = wrapper.find('.naming input')
    await input.setValue('next')
    await input.trigger('keydown', { key: 'Enter' })
    await flushPromises()
    // No start: the branch begins at HEAD, and the log says so.
    expect(sent('create_branch')).toEqual([{ name: 'next', start: undefined, checkout: true }])
  })

  it('declines when HEAD is not in the list, so the toolbar can ask another way', async () => {
    git.store.rows = [row('ccccccc3', 'Nobody is here')]
    mount(Host)
    await flushPromises()
    expect(naming.begin()).toBe(false)
  })

  it('declines once the graph is gone', async () => {
    const wrapper = mount(Host)
    await flushPromises()
    wrapper.unmount()
    expect(naming.begin()).toBe(false)
  })
})

describe('what makes a branch name', () => {
  it('lets ordinary names through', () => {
    expect(branchNameProblem('feature/thing', false)).toBeNull()
    expect(branchNameProblem('  fix-42  ', false)).toBeNull()
  })

  it('names the problem', () => {
    expect(branchNameProblem('', false)).toContain('name')
    expect(branchNameProblem('main', true)).toContain('already exists')
    for (const bad of ['a b', 'a..b', '-lead', '/lead', 'trail/', 'trail.', 'x@{1}', 'a:b', 'a*', 'a[b]', 'a\\b', 'a.lock', '@', 'a//b']) {
      expect(branchNameProblem(bad, false), bad).toContain('Git will not accept')
    }
  })
})
