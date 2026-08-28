// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import TitleBar from '~/components/TitleBar.vue'
import PromptDialog from '~/components/PromptDialog.vue'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []

let open: ReturnType<typeof mount> | null = null

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'stash_list') return git.store.stashes
    return null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.progress = null
  git.store.inside = []
  git.store.stashes = [
    { index: 0, oid: 's1', message: 'the half-finished idea', branch: 'main', time: 1756000000, files: 2 }
  ]
  // The stash's own row, as the graph draws it. Without it a refresh drops the
  // selection back to the working tree, which is what it should do — but only
  // once the stash has actually gone.
  git.store.rows = [
    {
      oid: 's1',
      short: 's1',
      summary: 'the half-finished idea',
      author: 'Ramon',
      email: 'r@x',
      time: 1756000000,
      parents: ['c1'],
      lane: 1,
      color: 1,
      width: 2,
      segments: [],
      labels: [],
      unpushed: false,
      stash: 0
    }
  ] as never
  git.store.selected = 's1'
})

afterEach(() => {
  open?.unmount()
  open = null
})

const show = () => {
  open = mount(TitleBar, {
    global: {
      components: { PromptDialog },
      stubs: { ProfileMenu: true, HistoryMenu: true, BranchDialog: true, Teleport: true }
    }
  })
  return open
}

const labels = (wrapper: ReturnType<typeof show>) =>
  wrapper.findAll('.actions .btn').map((b) => b.text())

describe('the toolbar while a stash is picked', () => {
  it('offers the stash instead of the repository', () => {
    const wrapper = show()
    expect(labels(wrapper)).toEqual(['Apply', 'Pop', 'Branch from it', 'Drop'])
    expect(wrapper.find('.stash-actions .what').text()).toContain('the half-finished idea')
  })

  it('goes back to fetch and push the moment something else is picked', async () => {
    const wrapper = show()
    git.store.selected = 'c1'
    await flushPromises()
    expect(labels(wrapper)).toEqual(['Fetch', 'Pull', 'Push', 'Branch', 'Stash'])
    expect(wrapper.find('.stash-actions').exists()).toBe(false)
  })

  it('leaves the repository keys working while it is showing a stash', async () => {
    show()
    // The buttons are gone; the shortcuts they duplicate are not.
    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, shiftKey: true, bubbles: true })
    )
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'fetch_all' || c.cmd.startsWith('fetch'))).toBe(true)
  })

  it('applies, pops and drops the stash it names', async () => {
    const wrapper = show()
    // Re-queried after each press: every action refreshes, which redraws the
    // bar and leaves the nodes found before it stale.
    const press = async (at: number) => {
      await wrapper.findAll('.actions .btn')[at]!.trigger('click')
      await flushPromises()
    }

    await press(0)
    expect(calls.find((c) => c.cmd === 'stash_apply')?.args).toEqual({ index: 0 })

    await press(1)
    expect(calls.find((c) => c.cmd === 'stash_pop')?.args).toEqual({ index: 0 })

    await press(3)
    expect(calls.find((c) => c.cmd === 'stash_drop')?.args).toEqual({ index: 0 })
  })

  it('goes back to the repository once the stash is gone', async () => {
    const wrapper = show()
    expect(wrapper.find('.stash-actions').exists()).toBe(true)

    // Dropping takes the row with it, and the selection falls back to the
    // working tree — so the bar is about the repository again with no help.
    git.store.rows = [] as never
    git.store.stashes = []
    await flushPromises()
    await wrapper.findAll('.actions .btn')[0]!.trigger('click')
    await flushPromises()
    expect(labels(wrapper)).toEqual(['Fetch', 'Pull', 'Push', 'Branch', 'Stash'])
  })

  it('asks for a name before turning one into a branch', async () => {
    const wrapper = show()
    expect(wrapper.findComponent(PromptDialog).exists()).toBe(false)

    await wrapper.findAll('.actions .btn')[2]!.trigger('click')
    const dialog = wrapper.findComponent(PromptDialog)
    expect(dialog.exists()).toBe(true)

    dialog.vm.$emit('submit', 'rescue/the-idea')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'stash_branch')?.args).toEqual({
      index: 0,
      name: 'rescue/the-idea'
    })
  })
})
