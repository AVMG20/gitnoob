// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import GraphList from '~/components/GraphList.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import { useGit, type GraphRow } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()

const row = (oid: string, summary: string): GraphRow => ({
  oid,
  short: oid.slice(0, 7),
  summary,
  author: 'Arno Visker',
  email: 'a@b.c',
  time: Math.floor(Date.now() / 1000) - 60,
  parents: [],
  lane: 0,
  color: 0,
  width: 1,
  segments: [],
  labels: [],
  unpushed: false
})

const Host = {
  components: { GraphList, ContextMenu },
  template: '<div><GraphList /><ContextMenu /></div>'
}

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async () => null)
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false, author: 'Arno Visker' } as never
  git.store.rows = [row('aaaaaaa1', 'Show the inherited email subject'), row('bbbbbbb2', 'Merged into staging')]
  git.store.status = {
    staged: [{ path: 'a.ts', kind: 'modified' }],
    unstaged: [
      { path: 'b.ts', kind: 'modified' },
      { path: 'c.ts', kind: 'untracked' }
    ],
    conflicted: []
  }
})

/**
 * The head of the commit list: the row for the working tree, and the two
 * buttons at the end of the column headings.
 */
describe('the top of the commit list', () => {
  it('says what is uncommitted in one line', async () => {
    const wrapper = mount(Host)
    await flushPromises()
    const wip = wrapper.find('.wip')
    expect(wip.text()).toContain('3 uncommitted changes')
    // The badge that used to sit beside it said the same thing a second time.
    expect(wip.find('.chip').exists()).toBe(false)
  })

  it('opens the search box from the headings, not only from the keyboard', async () => {
    const wrapper = mount(Host)
    await flushPromises()
    expect(wrapper.find('.search').exists()).toBe(false)

    await wrapper.find('.head-tools .tool').trigger('click')
    await flushPromises()
    expect(wrapper.find('.search').exists()).toBe(true)

    // And closes again from the same button.
    await wrapper.find('.head-tools .tool').trigger('click')
    await flushPromises()
    expect(wrapper.find('.search').exists()).toBe(false)
  })

  it('opens the column menu from the cog beside it', async () => {
    const wrapper = mount(Host)
    await flushPromises()
    await wrapper.findAll('.head-tools .tool')[1]!.trigger('click')
    await flushPromises()

    const rows = wrapper.findAll('.menu .item').map((one) => one.text())
    expect(rows.some((one) => one.includes('Branch / tag'))).toBe(true)
    expect(rows.some((one) => one.includes('Reset the widths'))).toBe(true)
  })
})
