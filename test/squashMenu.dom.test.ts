// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import GraphList from '~/components/GraphList.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import AppModal from '~/components/AppModal.vue'
import SquashDialog from '~/components/SquashDialog.vue'
import { useGit, type GraphRow } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []

function row(over: Partial<GraphRow>): GraphRow {
  return {
    oid: 'c1',
    short: 'c1',
    summary: 'feat: a commit',
    author: 'Ramon',
    email: 'r@x',
    time: 1756000000,
    parents: ['c2'],
    lane: 0,
    color: 0,
    width: 2,
    segments: [],
    labels: [],
    unpushed: false,
    stash: null,
    ...over
  }
}

const Host = {
  components: { GraphList, ContextMenu },
  template: '<div><GraphList /><ContextMenu /></div>'
}

let open: ReturnType<typeof mount> | null = null

const show = () => {
  open = mount(Host, {
    global: { components: { AppModal, SquashDialog }, stubs: { Teleport: true } }
  })
  return open
}

/** The menu row whose label starts with "Squash". */
const squashRow = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('.menu .item').find((item) => item.find('.label').text().startsWith('Squash'))

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'squash_preview') {
      return {
        commits: [
          {
            oid: 'c3',
            short: 'c3',
            summary: 'feat: older',
            message: 'feat: older',
            author: 'Ramon',
            time: 1756000000,
            pushed: false
          },
          {
            oid: 'c1',
            short: 'c1',
            summary: 'feat: a commit',
            message: 'feat: a commit',
            author: 'Ramon',
            time: 1756000100,
            pushed: false
          }
        ],
        message: 'feat: older\n\nfeat: a commit',
        onto: 'c4',
        above: 0,
        branch: 'main',
        refusal: null
      }
    }
    if (cmd === 'squash') return 'Squashed 2 commits into one'
    return null
  })
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.stashes = []
  git.store.rows = [
    row({}),
    row({ oid: 'c3', short: 'c3', summary: 'feat: older', parents: ['c4'] }),
    row({ oid: 'c4', short: 'c4', summary: 'feat: the first', parents: [] })
  ]
})

afterEach(() => {
  open?.unmount()
  open = null
})

describe('squashing from the commit list', () => {
  it('offers the fold by name once several commits are marked', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.row')[1]!.trigger('click', { ctrlKey: true })
    await flushPromises()
    expect(wrapper.find('.marks').text()).toContain('2 selected')

    await wrapper.findAll('.row')[1]!.trigger('contextmenu')
    await flushPromises()
    expect(squashRow(wrapper)!.text()).toContain('Squash 2 commits into one')
  })

  it('folds a single commit into the one below it', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('contextmenu')
    await flushPromises()
    const item = squashRow(wrapper)!
    expect(item.find('.label').text()).toBe('Squash into the commit below…')
    // The commit it would join is named, so it is not a guess.
    expect(item.find('.hint').text()).toContain('c2')

    await item.trigger('click')
    await flushPromises()
    // Its parent first: the fold is a run, oldest to newest.
    expect(calls.find((c) => c.cmd === 'squash_preview')?.args).toEqual({ oids: ['c2', 'c1'] })
  })

  it('refuses the single-commit fold at the first commit, which has nothing below it', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[2]!.trigger('contextmenu')
    await flushPromises()
    const item = squashRow(wrapper)!
    expect(item.attributes('disabled')).toBeDefined()
    expect(item.find('.hint').text()).toBe('nothing below it')
  })

  it('hands the dialog exactly the commits that were marked', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.row')[1]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.row')[1]!.trigger('contextmenu')
    await flushPromises()

    await squashRow(wrapper)!.trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'squash_preview')?.args).toEqual({ oids: ['c1', 'c3'] })
    expect(wrapper.find('.commits').exists()).toBe(true)
  })

  it('drops the marks once the fold has happened, since they point at commits that are gone', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.row')[1]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.row')[1]!.trigger('contextmenu')
    await flushPromises()
    await squashRow(wrapper)!.trigger('click')
    await flushPromises()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'squash')?.args).toEqual({
      oids: ['c1', 'c3'],
      message: 'feat: older\n\nfeat: a commit'
    })
    expect(wrapper.find('.marks').exists()).toBe(false)
  })

  it('keeps the marks when the dialog is closed without folding', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.row')[1]!.trigger('click', { ctrlKey: true })
    await wrapper.findAll('.row')[1]!.trigger('contextmenu')
    await flushPromises()
    await squashRow(wrapper)!.trigger('click')
    await flushPromises()

    await wrapper.find('.btn-ghost').trigger('click')
    await flushPromises()
    expect(wrapper.find('.marks').text()).toContain('2 selected')
    expect(calls.some((c) => c.cmd === 'squash')).toBe(false)
  })

  it('is not offered on a stash, which is not part of the history', async () => {
    git.store.stashes = [
      { index: 0, oid: 's1', message: 'half an idea', branch: 'main', time: 1756000100, files: 1 }
    ]
    git.store.rows = [row({ oid: 's1', short: 's1', summary: 'half an idea', stash: 0 }), ...git.store.rows]
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('contextmenu')
    await flushPromises()
    expect(squashRow(wrapper)).toBeUndefined()
  })
})
