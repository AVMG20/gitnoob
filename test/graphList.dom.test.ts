// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import GraphList from '~/components/GraphList.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import { useGit, type GraphRow } from '~/composables/useGit'
import { CODE_ROW, OVERSCAN } from '~/composables/useCode'
import { useDragDrop } from '~/composables/useDragDrop'

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
  unpushed: false,
  stash: null
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

  it('scrolls the working-tree row away with everything else', async () => {
    // Enough rows to scroll through, so the window has to be worked out rather
    // than being every row there is.
    git.store.rows = Array.from({ length: 80 }, (_, at) =>
      row(`c${String(at).padStart(7, '0')}`, `Commit ${at}`)
    )
    const wrapper = mount(Host)
    await flushPromises()

    const viewport = wrapper.find('.viewport')
    // It is inside the scroller, not a strip pinned above it.
    expect(viewport.find('.wip').exists()).toBe(true)

    const box = viewport.element as HTMLElement
    // Twenty rows down, plus the row the working tree takes at the top.
    const ROW = 27
    box.scrollTop = ROW + 20 * ROW
    await viewport.trigger('scroll')
    await new Promise((resolve) => requestAnimationFrame(resolve))
    await flushPromises()

    // Exactly the twentieth commit, less the margin drawn above the window.
    // A row out either way is the row above having been forgotten.
    const shown = wrapper.findAll('.spacer .row .summary').map((one) => one.text())
    expect(shown[0]).toBe(`Commit ${20 - OVERSCAN}`)
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

/**
 * Picking a row up.
 *
 * The sidebar has taken a commit dropped on a branch since it was written, but
 * nothing ever started that drag: the gesture was half built, and dragging a
 * commit did nothing at all.
 */
describe('dragging a row out of the commit list', () => {
  const drag = useDragDrop()

  beforeEach(() => drag.end())

  it('carries the commit, so a branch can take it', async () => {
    const wrapper = mount(Host)
    await flushPromises()

    const first = wrapper.findAll('.row')[0]!
    expect(first.attributes('draggable')).toBe('true')
    await first.trigger('dragstart', { dataTransfer: new DataTransfer() })

    expect(drag.state.payload).toEqual({
      kind: 'commit',
      oid: 'aaaaaaa1',
      short: 'aaaaaaa',
      summary: 'Show the inherited email subject'
    })
  })

  it('carries a stash as a stash, not as the commit underneath it', async () => {
    git.store.rows = [{ ...row('cccccccc', 'On main: half a refactor'), stash: 2 }]
    git.store.stashes = [
      { index: 2, message: 'On main: half a refactor', branch: 'main', files: 3, time: 0 } as never
    ]
    const wrapper = mount(Host)
    await flushPromises()

    await wrapper.findAll('.row')[0]!.trigger('dragstart', { dataTransfer: new DataTransfer() })

    expect(drag.state.payload).toEqual({
      kind: 'stash',
      index: 2,
      message: 'On main: half a refactor'
    })
  })

  it('lets go at the end of the gesture', async () => {
    const wrapper = mount(Host)
    await flushPromises()

    const first = wrapper.findAll('.row')[0]!
    await first.trigger('dragstart', { dataTransfer: new DataTransfer() })
    await first.trigger('dragend')

    expect(drag.state.payload).toBeNull()
  })
})

/**
 * Dropping a branch on a commit.
 *
 * A mixed reset, so nothing on disk changes and undo puts the branch back —
 * which is why it runs without a dialog. What it must never do is move a
 * branch nobody asked it to: `git reset` moves whichever branch HEAD points
 * at, so the one being dragged has to be that one.
 */
describe('dropping a branch on a commit', () => {
  const drag = useDragDrop()

  beforeEach(() => drag.end())

  /** Drags `name` onto the second row and reports what git was asked to do. */
  async function dropOn(name: string) {
    const wrapper = mount(Host)
    await flushPromises()
    asked.mockClear()
    drag.state.payload = { kind: 'branch', name, remote: false }
    await wrapper.findAll('.row')[1]!.trigger('drop')
    await flushPromises()
    return asked.mock.calls.map(([cmd, args]) => ({ cmd, args }))
  }

  it('moves the branch you are on, keeping every change on disk', async () => {
    const calls = await dropOn('main')
    expect(calls.find((call) => call.cmd === 'reset')?.args).toMatchObject({
      oid: 'bbbbbbb2',
      mode: 'mixed'
    })
  })

  it('refuses another branch rather than moving the current one behind your back', async () => {
    const calls = await dropOn('staging')
    expect(calls.some((call) => call.cmd === 'reset')).toBe(false)
    expect(git.store.log[0]?.text).toContain('Check out staging first')
  })

  it('leaves a merge or a rebase alone while it is running', async () => {
    git.store.progress = {
      merging: false,
      rebasing: true,
      cherry_picking: false,
      reverting: false,
      restoring: false,
      applied_stash: null,
      prepared: null
    }
    const calls = await dropOn('main')
    git.store.progress = null
    expect(calls.some((call) => call.cmd === 'reset')).toBe(false)
    expect(git.store.log[0]?.text).toContain('Finish or abort')
  })
})
