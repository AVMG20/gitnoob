// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SideBar from '~/components/SideBar.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useGit, type LocalBranch } from '~/composables/useGit'

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

const Host = {
  components: { SideBar, ContextMenu },
  template: '<div><SideBar /><ContextMenu /></div>'
}

let open: ReturnType<typeof mount> | null = null

const show = () => {
  open = mount(Host, { global: { components: { MidTruncate }, stubs: { Teleport: true } } })
  return open
}

/** Every menu row's label, in the order they are drawn. */
const labels = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('.menu .item .label').map((one) => one.text())

/** The rows in the branch list, which are the ones carrying a name. */
const branchRows = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('.row').filter((one) => one.find('.name').exists())

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async () => null)
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'tickets', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.stashes = []
  git.store.refs = {
    locals: [branch('tickets', true), branch('staging', false)],
    remotes: [],
    tags: [],
    stashes: []
  }
})

afterEach(() => {
  open?.unmount()
  open = null
})

describe('the menu on a local branch', () => {
  it('leaves out merge and rebase on the branch you are standing on', async () => {
    const wrapper = show()
    await flushPromises()
    const row = branchRows(wrapper).find((one) => one.text().includes('tickets'))!
    await row.trigger('contextmenu')
    await flushPromises()

    // "Merge tickets into tickets" is not an action that was refused; it is a
    // sentence that means nothing, so the block is not drawn at all.
    expect(labels(wrapper).some((label) => label.startsWith('Merge'))).toBe(false)
    expect(labels(wrapper).some((label) => label.startsWith('Rebase'))).toBe(false)
  })

  it('still offers checking out and deleting there, greyed rather than gone', async () => {
    const wrapper = show()
    await flushPromises()
    const row = branchRows(wrapper).find((one) => one.text().includes('tickets'))!
    await row.trigger('contextmenu')
    await flushPromises()

    const items = wrapper.findAll('.menu .item')
    const checkout = items.find((one) => one.find('.label').text() === 'Check out')!
    const remove = items.find((one) => one.find('.label').text() === 'Delete branch…')!
    expect(checkout.attributes('disabled')).toBeDefined()
    expect(remove.attributes('disabled')).toBeDefined()
    // And the worktree row, which is muted for its own reason.
    expect(items.some((one) => one.find('.label').text().startsWith('Open in a new worktree'))).toBe(
      true
    )
  })

  it('offers all three on any other branch, naming both ends', async () => {
    const wrapper = show()
    await flushPromises()
    const row = branchRows(wrapper).find((one) => one.text().includes('staging'))!
    await row.trigger('contextmenu')
    await flushPromises()

    const found = labels(wrapper)
    expect(found).toContain('Merge staging into tickets')
    expect(found).toContain('Merge tickets into staging')
    expect(found).toContain('Rebase tickets onto staging')
    // None of them muted: they are all things that can be done from here.
    const items = wrapper
      .findAll('.menu .item')
      .filter((one) => /^(Merge|Rebase)/.test(one.find('.label').text()))
    expect(items.every((one) => one.attributes('disabled') === undefined)).toBe(true)
  })

  it('draws no divider where the missing block used to be', async () => {
    const wrapper = show()
    await flushPromises()
    const row = branchRows(wrapper).find((one) => one.text().includes('tickets'))!
    await row.trigger('contextmenu')
    await flushPromises()
    const here = wrapper.findAll('.menu .divider').length

    await wrapper.find('.scrim').trigger('click')
    const other = branchRows(wrapper).find((one) => one.text().includes('staging'))!
    await other.trigger('contextmenu')
    await flushPromises()

    // One fewer, not the same number with a gap in it.
    expect(here).toBe(wrapper.findAll('.menu .divider').length - 1)
  })
})
