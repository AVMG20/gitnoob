// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SideBar from '~/components/SideBar.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useGit, type Worktree } from '~/composables/useGit'
import { useConfig, type Config } from '~/composables/useConfig'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const config = useConfig()

const tree = (path: string, current: boolean): Worktree => ({
  path,
  name: path.slice(1),
  branch: path.slice(1),
  oid: `${path}-oid`,
  is_main: current,
  is_current: current,
  locked: false
})

/** A config with one tab open on the main folder. */
function stored(replaces: boolean): Config {
  return {
    version: 1,
    active_profile: 'me',
    global: { show_avatars: true, worktree_replaces_tab: replaces } as never,
    profiles: [
      {
        id: 'me',
        name: 'Me',
        projects: [{ path: '/repo', name: 'repo' }],
        recents: [],
        active_project: '/repo'
      } as never
    ]
  }
}

let opened: [string, string | undefined][] = []
let wrapper: ReturnType<typeof mount> | null = null

const Host = {
  components: { SideBar, ContextMenu },
  setup: () => ({
    onOpen: (path: string, replace?: string) => opened.push([path, replace])
  }),
  template: '<div><SideBar @open="onOpen" /><ContextMenu /></div>'
}

async function show(replaces: boolean) {
  config.store.config = stored(replaces)
  wrapper = mount(Host, { global: { components: { MidTruncate }, stubs: { Teleport: true } } })
  await flushPromises()
  return wrapper
}

const row = (w: ReturnType<typeof mount>, name: string) =>
  w.findAll('.row').find((one) => one.find('.name').text() === name)!

beforeEach(() => {
  opened = []
  asked.mockReset()
  asked.mockImplementation(async () => null)
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.stashes = []
  git.store.refs = { locals: [], remotes: [], tags: [], stashes: [] }
  git.store.worktrees = [tree('/repo', true), tree('/repo-serp', false)]
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  config.store.config = null
})

describe('clicking a worktree', () => {
  it('opens it as a tab of its own by default', async () => {
    const w = await show(false)
    await row(w, 'repo-serp').trigger('click')
    expect(opened).toEqual([['/repo-serp', undefined]])
  })

  it('turns this tab into it when the setting is on', async () => {
    const w = await show(true)
    await row(w, 'repo-serp').trigger('click')
    expect(opened).toEqual([['/repo-serp', '/repo']])
  })

  it('does nothing on the worktree already open here', async () => {
    const w = await show(true)
    await row(w, 'repo').trigger('click')
    expect(opened).toEqual([])
  })

  it('offers both ways in the menu, whatever the setting says', async () => {
    const w = await show(false)
    await row(w, 'repo-serp').trigger('contextmenu')
    await flushPromises()
    const item = w
      .findAll('.menu .item')
      .find((one) => one.find('.label').text() === 'Open in this tab')!
    await item.trigger('click')
    expect(opened).toEqual([['/repo-serp', '/repo']])
  })
})
