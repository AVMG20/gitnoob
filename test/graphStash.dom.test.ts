// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import GraphList from '~/components/GraphList.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import AppModal from '~/components/AppModal.vue'
import DropStashDialog from '~/components/DropStashDialog.vue'
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
    segments: [{ x1: 0, y1: 1, x2: 0, y2: 2, color: 0, dashed: false }],
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

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return null
  })
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.stashes = [
    { index: 0, oid: 's1', message: 'the half-finished idea', branch: 'main', time: 1756000100, files: 2 }
  ]
  git.store.rows = [
    row({
      oid: 's1',
      short: 's1',
      summary: 'the half-finished idea',
      parents: ['c1'],
      lane: 1,
      color: 1,
      stash: 0,
      segments: [{ x1: 1, y1: 1, x2: 1, y2: 2, color: 1, dashed: true }]
    }),
    row({}),
    row({ oid: 'c2', short: 'c2', summary: 'feat: older', parents: [], segments: [] })
  ]
})

afterEach(() => {
  open?.unmount()
  open = null
})

const show = () => {
  open = mount(Host, {
    // The dialogs GraphList opens are auto-imported in the app; here they have
    // to be handed over, or the confirmation never renders.
    global: { components: { AppModal, DropStashDialog }, stubs: { Teleport: true } }
  })
  return open
}

describe('a stash drawn in the commit list', () => {
  it('draws its line broken and the history solid', () => {
    const wrapper = show()
    const dashed = wrapper.findAll('path[stroke-dasharray]')
    expect(dashed.length).toBeGreaterThan(0)
    // The ordinary commit's line carries no dashes.
    const solid = wrapper.findAll('path').filter((p) => !p.attributes('stroke-dasharray'))
    expect(solid.length).toBeGreaterThan(0)
  })

  it('marks the row as a stash rather than drawing an author', () => {
    const wrapper = show()
    expect(wrapper.html()).toContain('a stash, not a commit')
  })

  it('selects the stash when its row is clicked, like any other row', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('click')
    await flushPromises()
    expect(git.store.selected).toBe('s1')
    // Its contents are read the way a commit's are, so the list stays put.
    expect(calls.find((c) => c.cmd === 'commit_detail')?.args).toEqual({ oid: 's1' })
  })

  it('leaves the commit list where it is either way', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('click')
    await flushPromises()
    expect(wrapper.findAll('.row').length).toBeGreaterThan(1)

    await wrapper.findAll('.row')[1]!.trigger('click')
    await flushPromises()
    expect(git.store.selected).toBe('c1')
    expect(wrapper.findAll('.row').length).toBeGreaterThan(1)
  })

  it('offers stash actions on right-click, not commit ones', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('contextmenu')
    await flushPromises()
    const labels = wrapper.findAll('button').map((b) => b.text())
    expect(labels.some((one) => one.startsWith('Pop'))).toBe(true)
    expect(labels.some((one) => one.includes('Cherry-pick'))).toBe(false)
    expect(labels.some((one) => one.includes('Branch from here'))).toBe(false)
  })

  it('asks before dropping one from the menu', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[0]!.trigger('contextmenu')
    await flushPromises()

    const drop = wrapper.findAll('button').find((b) => b.text().startsWith('Drop this stash'))!
    await drop.trigger('click')
    await flushPromises()

    // The menu asked; git has not been told anything yet.
    expect(wrapper.text()).toContain('Drop this stash?')
    expect(calls.some((c) => c.cmd === 'stash_drop')).toBe(false)

    await wrapper.find('.btn-danger').trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'stash_drop')?.args).toEqual({ index: 0 })
  })
})
