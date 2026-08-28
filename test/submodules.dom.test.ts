// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SideBar from '~/components/SideBar.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useGit, type Submodule } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const git = useGit()

const Host = {
  components: { SideBar, ContextMenu },
  template: '<div><SideBar /><ContextMenu /></div>'
}

function submodule(over: Partial<Submodule>): Submodule {
  return {
    name: 'libs/shared',
    path: 'libs/shared',
    abs: '/repo/libs/shared',
    url: 'git@github.com:acme/shared.git',
    branch: null,
    oid: '0e8a1f2c4b6d8e0a2c4e6a8c0e2a4c6e8a0c2e4a',
    short: '0e8a1f2',
    described: 'v1.4.0',
    state: 'ready',
    ...over
  }
}

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return null
  })
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.submodules = []
})

const mountSideBar = () =>
  mount(Host, { global: { components: { MidTruncate }, stubs: { Teleport: true } } })

describe('the submodules section', () => {
  it('is not there at all for a repository that declares none', () => {
    const wrapper = mountSideBar()
    expect(wrapper.text()).not.toContain('Submodules')
  })

  it('lists each one with where it stands', () => {
    git.store.submodules = [
      submodule({}),
      submodule({ path: 'vendor/theme', abs: '/repo/vendor/theme', state: 'absent' }),
      submodule({ path: 'tools/cli', abs: '/repo/tools/cli', state: 'moved', described: null })
    ]
    const wrapper = mountSideBar()
    expect(wrapper.text()).toContain('Submodules')
    expect(wrapper.text()).toContain('v1.4.0')
    expect(wrapper.text()).toContain('not cloned')
    expect(wrapper.text()).toContain('0e8a1f2 · moved')
  })

  it('marks the count while any of them is adrift, and leaves it alone otherwise', async () => {
    git.store.submodules = [submodule({})]
    let wrapper = mountSideBar()
    expect(wrapper.find('.count.adrift').exists()).toBe(false)

    wrapper.unmount()
    git.store.submodules = [submodule({ state: 'moved' })]
    wrapper = mountSideBar()
    expect(wrapper.find('.count.adrift').exists()).toBe(true)
  })

  it('fetches one that was never cloned instead of opening an empty folder', async () => {
    git.store.submodules = [submodule({ state: 'absent' })]
    const wrapper = mountSideBar()
    await wrapper.find('.row.stash').trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'submodule_update')?.args).toMatchObject({
      path: 'libs/shared'
    })
  })

  it('steps into a cloned one rather than opening a tab or touching git', async () => {
    git.store.submodules = [submodule({})]
    const wrapper = mountSideBar()
    await wrapper.find('.row.stash').trigger('click')
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'submodule_update')).toBe(false)
    const bar = wrapper.findComponent(SideBar)
    expect(bar.emitted('open')).toBeFalsy()
    expect(bar.emitted('enter')?.[0]?.[0]).toMatchObject({ abs: '/repo/libs/shared' })
  })

  it('still offers a tab of its own, for looking at two at once', async () => {
    git.store.submodules = [submodule({})]
    const wrapper = mountSideBar()
    await wrapper.find('.row.stash').trigger('contextmenu')
    await flushPromises()
    const asTab = wrapper.findAll('button').find((b) => b.text().startsWith('Open as its own tab'))
    await asTab!.trigger('click')
    expect(wrapper.findComponent(SideBar).emitted('open')?.[0]).toEqual(['/repo/libs/shared'])
  })

  it('updates every one from the heading', async () => {
    git.store.submodules = [submodule({})]
    const wrapper = mountSideBar()
    const updateAll = wrapper
      .findAll('.head-action')
      .find((b) => b.attributes('title')?.startsWith('Fetch every submodule'))
    await updateAll!.trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'submodule_update')?.args).toMatchObject({ path: null })
  })

  it('will not empty a folder that was never cloned', async () => {
    git.store.submodules = [submodule({ state: 'absent' })]
    const wrapper = mountSideBar()
    await wrapper.find('.row.stash').trigger('contextmenu')
    await flushPromises()
    const empty = wrapper.findAll('button').find((b) => b.text().startsWith('Empty its folder'))
    expect(empty?.attributes('disabled')).toBeDefined()
  })
})
