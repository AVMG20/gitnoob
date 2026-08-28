// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import TitleBar from '~/components/TitleBar.vue'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const git = useGit()

beforeEach(() => {
  git.store.repo = { path: '/repo', name: 'playground', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.progress = null
  git.store.inside = []
})

const show = () =>
  mount(TitleBar, {
    global: { stubs: { ProfileMenu: true, HistoryMenu: true, BranchDialog: true } }
  })

describe('the trail into a submodule', () => {
  it('shows the repository name and nothing else when you are not in one', () => {
    const wrapper = show()
    expect(wrapper.find('.name').text()).toBe('playground')
    expect(wrapper.find('.crumb').exists()).toBe(false)
  })

  it('names the project it started from, and where you are now', () => {
    git.store.inside = [{ path: '/repo/libs/shared', name: 'libs/shared', from: '/repo', fromName: 'playground' }]
    const wrapper = show()
    expect(wrapper.find('.crumb.root').text()).toBe('playground')
    expect(wrapper.find('.crumb.here').text()).toContain('libs/shared')
    // The plain repository name gives way to the trail.
    expect(wrapper.find('.name').exists()).toBe(false)
  })

  it('offers a way out of it', async () => {
    git.store.inside = [{ path: '/repo/libs/shared', name: 'libs/shared', from: '/repo', fromName: 'playground' }]
    const wrapper = show()
    await wrapper.find('.crumb.here .out').trigger('click')
    expect(wrapper.emitted('leave')?.[0]).toEqual([0])
  })

  it('goes back to the project itself from the root of the trail', async () => {
    git.store.inside = [
      { path: '/repo/libs/shared', name: 'libs/shared', from: '/repo', fromName: 'playground' },
      {
        path: '/repo/libs/shared/vendor/dep',
        name: 'vendor/dep',
        from: '/repo/libs/shared',
        fromName: 'libs/shared'
      }
    ]
    const wrapper = show()
    await wrapper.find('.crumb.root').trigger('click')
    expect(wrapper.emitted('leave')?.[0]).toEqual([0])
  })

  it('draws every step of a nested one, each its own way back', async () => {
    git.store.inside = [
      { path: '/repo/libs/shared', name: 'libs/shared', from: '/repo', fromName: 'playground' },
      {
        path: '/repo/libs/shared/vendor/dep',
        name: 'vendor/dep',
        from: '/repo/libs/shared',
        fromName: 'libs/shared'
      }
    ]
    const wrapper = show()
    const crumbs = wrapper.findAll('.crumb')
    expect(crumbs.map((c) => c.text().trim())).toEqual(['playground', 'libs/shared', 'vendor/dep'])
    // Only the last one is where you are; the middle one steps back to itself.
    expect(crumbs[2]!.classes()).toContain('here')
    await crumbs[1]!.trigger('click')
    expect(wrapper.emitted('leave')?.[0]).toEqual([1])
  })
})
