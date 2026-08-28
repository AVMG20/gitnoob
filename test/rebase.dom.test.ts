// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import RebasePane from '~/components/RebasePane.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import { useGit } from '~/composables/useGit'
import { useRebase, type Candidate, type RebaseProgress } from '~/composables/useRebase'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const rebase = useRebase()

let calls: { cmd: string; args: Record<string, unknown> }[] = []
let progress: RebaseProgress | null = null

const CANDIDATES: Candidate[] = [
  { oid: 'aaa', short: 'aaa1111', summary: 'feat: one', author: 'Ramon', email: 'r@x', time: 1, pushed: true },
  { oid: 'bbb', short: 'bbb2222', summary: 'fix typo', author: 'Ramon', email: 'r@x', time: 2, pushed: false },
  { oid: 'ccc', short: 'ccc3333', summary: 'test: cover it', author: 'Ramon', email: 'r@x', time: 3, pushed: false }
]

const Host = {
  components: { RebasePane, ContextMenu },
  template: '<div><RebasePane /><ContextMenu /></div>'
}

beforeEach(async () => {
  calls = []
  progress = null
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'rebase_plan') return CANDIDATES
    if (cmd === 'rebase_progress') return progress
    if (cmd.startsWith('rebase_')) return 'Rebase stopped'
    return null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  rebase.store.progress = null
  await rebase.planFrom('base1', 'base1')
})

describe('building a rebase plan', () => {
  it('lists the commits oldest first, all kept to begin with', () => {
    const wrapper = mount(Host)
    const rows = wrapper.findAll('.row')
    expect(rows).toHaveLength(3)
    expect(rows[0]!.text()).toContain('feat: one')
    expect(rows.every((row) => row.find('.act').text() === 'pick')).toBe(true)
  })

  it('marks the ones a remote already has', () => {
    const wrapper = mount(Host)
    expect(wrapper.findAll('.row')[0]!.find('.chip').text()).toContain('on a remote')
    expect(wrapper.findAll('.row')[1]!.find('.chip').exists()).toBe(false)
    expect(wrapper.find('.warn-line').text()).toContain('force')
  })

  it('folds a commit into the one above when it is squashed', async () => {
    rebase.setAction(1, 'fixup')
    await flushPromises()
    const wrapper = mount(Host)
    expect(wrapper.findAll('.row')[1]!.classes()).toContain('melded')
    // Two commits out of three in, and the preview says which absorbed it.
    expect(wrapper.find('.tally').text()).toContain('3')
    expect(wrapper.findAll('.pv li')).toHaveLength(3) // two commits plus the base
    expect(wrapper.find('.pv').text()).toContain('+1 folded in')
  })

  it('strikes a dropped commit through and leaves it out of the outcome', async () => {
    rebase.setAction(2, 'drop')
    await flushPromises()
    const wrapper = mount(Host)
    expect(wrapper.findAll('.row')[2]!.classes()).toContain('gone')
    expect(wrapper.find('.tally').text()).toContain('1 dropped')
    expect(wrapper.find('.pv').text()).not.toContain('test: cover it')
  })

  it('refuses a plan that folds the first commit into nothing', async () => {
    rebase.setAction(0, 'squash')
    await flushPromises()
    const wrapper = mount(Host)
    expect(wrapper.find('.refusal').text()).toContain('nothing above it')
    expect(wrapper.find('.btn-primary').attributes('disabled')).toBeDefined()
  })

  it('refuses a plan that drops everything', async () => {
    for (let at = 0; at < 3; at++) rebase.setAction(at, 'drop')
    await flushPromises()
    const wrapper = mount(Host)
    expect(wrapper.find('.refusal').text()).toContain('Use reset')
  })

  it('sends the plan in the order the list stands in', async () => {
    rebase.move(2, 0)
    rebase.setAction(2, 'fixup')
    await flushPromises()
    const wrapper = mount(Host)
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'rebase_start')?.args).toMatchObject({
      onto: 'base1',
      steps: [
        { oid: 'ccc', action: 'pick' },
        { oid: 'aaa', action: 'pick' },
        { oid: 'bbb', action: 'fixup' }
      ]
    })
  })

  it('puts the plan back in order when asked', async () => {
    rebase.move(0, 2)
    rebase.setAction(0, 'drop')
    rebase.reset()
    await flushPromises()
    expect(rebase.store.rows.map((r) => r.oid)).toEqual(['aaa', 'bbb', 'ccc'])
    expect(rebase.store.rows.every((r) => r.action === 'pick')).toBe(true)
  })
})

describe('a rebase that has stopped', () => {
  it('asks for a message where the plan asked for a reword', async () => {
    rebase.store.progress = {
      at: 2,
      total: 3,
      stopped: 'bbb',
      summary: 'fix typo',
      rewording: true,
      message: 'fix typo'
    }
    const wrapper = mount(Host)
    const box = wrapper.find('.msgbox')
    expect((box.element as HTMLInputElement).value).toBe('fix typo')

    await box.setValue('fix(export): the header row')
    await wrapper.find('.strip .btn.tiny').trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'rebase_reword')?.args).toEqual({
      message: 'fix(export): the header row'
    })
  })

  it('offers to carry on where it stopped for an edit', async () => {
    rebase.store.progress = {
      at: 1,
      total: 3,
      stopped: 'aaa',
      summary: 'feat: one',
      rewording: false,
      message: null
    }
    const wrapper = mount(Host)
    expect(wrapper.find('.strip').text()).toContain('Stopped at')
    expect(wrapper.find('.msgbox').exists()).toBe(false)
    await wrapper.find('.strip .btn.tiny').trigger('click')
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'rebase_continue')).toBe(true)
  })

  it('points at the conflicted files when that is why it stopped', async () => {
    git.store.status = { staged: [], unstaged: [], conflicted: ['app/app.vue'] } as never
    rebase.store.progress = {
      at: 2, total: 3, stopped: 'bbb', summary: 'fix typo', rewording: false, message: null
    }
    const wrapper = mount(Host)
    expect(wrapper.find('.strip').text()).toContain('1 conflicted file')
    await wrapper.find('.strip .btn.tiny').trigger('click')
    expect(git.store.resolving).toBe('app/app.vue')
  })

  it('marks the row it is sitting on, and will not let the plan be edited', async () => {
    rebase.store.progress = {
      at: 2, total: 3, stopped: 'bbb', summary: 'fix typo', rewording: false, message: null
    }
    const wrapper = mount(Host)
    expect(wrapper.findAll('.row')[1]!.classes()).toContain('here')
    expect(wrapper.findAll('.act')[0]!.attributes('disabled')).toBeDefined()
    expect(wrapper.find('.foot').exists()).toBe(false)
  })
})
