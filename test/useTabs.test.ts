// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  HISTORY_LIMIT,
  consumeLanding,
  findTab,
  closeAll,
  closeOthers,
  closeTab,
  goBack,
  goForward,
  openTab,
  pinTab,
  resetTabs,
  retarget,
  selectTab,
  showGraph,
  stepTab,
  switchTabs,
  tabId,
  useTabs
} from '~/composables/useTabs'
import { diffMode } from '~/composables/useDiffMode'
import { navFrom, resetNav } from '~/composables/useNavButtons'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const tabs = useTabs()
const state = tabs.state
const ids = () => state.tabs.map((tab) => tab.path)
const here = () => tabs.activeTab()?.path ?? null

beforeEach(() => {
  resetTabs()
  resetNav()
  diffMode.mode = 'diff'
  switchTabs('/repos/app')
})

describe('opening files', () => {
  it('starts on the graph with no tabs', () => {
    expect(state.tabs).toEqual([])
    expect(state.active).toBeNull()
    expect(tabs.viewer.value).toBeNull()
  })

  it('selects the tab a file already has instead of opening a second', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    openTab({ path: 'a.php' })
    expect(ids()).toEqual(['a.php', 'b.php'])
    expect(here()).toBe('a.php')
  })

  it('keeps the same file at a commit apart from the working copy', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'a.php', commit: 'abc1234' })
    expect(state.tabs.map((tab) => tab.id)).toEqual([tabId('a.php'), tabId('a.php', 'abc1234')])
    expect(tabs.viewer.value).toEqual({ path: 'a.php', commit: 'abc1234' })
  })

  it('replaces the preview tab with the next preview', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' }, { preview: true })
    openTab({ path: 'c.php' }, { preview: true })
    expect(ids()).toEqual(['a.php', 'c.php'])
    expect(state.tabs[1]!.preview).toBe(true)
  })

  it('keeps a preview once it is pinned, and the next preview opens beside it', () => {
    openTab({ path: 'a.php' }, { preview: true })
    pinTab(tabId('a.php'))
    openTab({ path: 'b.php' }, { preview: true })
    expect(ids()).toEqual(['a.php', 'b.php'])
  })

  it('pins a preview tab that is opened again without preview', () => {
    openTab({ path: 'a.php' }, { preview: true })
    openTab({ path: 'a.php' })
    expect(state.tabs[0]!.preview).toBe(false)
  })

  it('opens in the mode it was asked for, and remembers each tab’s own', () => {
    openTab({ path: 'a.php', mode: 'file' })
    expect(diffMode.mode).toBe('file')
    openTab({ path: 'b.php', mode: 'diff' })
    expect(diffMode.mode).toBe('diff')
    // Flipping the switch flips the tab on screen.
    diffMode.mode = 'file'
    selectTab(tabId('a.php'))
    selectTab(tabId('b.php'))
    expect(diffMode.mode).toBe('file')
  })
})

describe('closing tabs', () => {
  beforeEach(() => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    openTab({ path: 'c.php' })
  })

  it('moves to the left neighbour when the tab on screen closes', () => {
    selectTab(tabId('b.php'))
    closeTab(tabId('b.php'))
    expect(here()).toBe('a.php')
  })

  it('moves right when the first one closes', () => {
    selectTab(tabId('a.php'))
    closeTab(tabId('a.php'))
    expect(here()).toBe('b.php')
  })

  it('stays put when a tab in the background closes', () => {
    closeTab(tabId('a.php'))
    expect(here()).toBe('c.php')
    expect(ids()).toEqual(['b.php', 'c.php'])
  })

  it('brings the graph back when the last one closes', () => {
    closeAll()
    expect(state.tabs).toEqual([])
    expect(state.active).toBeNull()
  })

  it('closes the others', () => {
    closeOthers(tabId('b.php'))
    expect(ids()).toEqual(['b.php'])
    expect(here()).toBe('b.php')
  })
})

describe('back and forward', () => {
  it('goes back to the graph a file was opened from, and forward again', () => {
    openTab({ path: 'a.php' })
    expect(goBack()).toBe(true)
    expect(state.active).toBeNull()
    expect(ids()).toEqual(['a.php'])
    expect(goForward()).toBe(true)
    expect(here()).toBe('a.php')
  })

  it('records a tab switch and a visit to the graph', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    selectTab(tabId('a.php'))
    showGraph()
    goBack()
    expect(here()).toBe('a.php')
    goBack()
    expect(here()).toBe('b.php')
    goBack()
    expect(here()).toBe('a.php')
    goBack()
    expect(state.active).toBeNull()
    expect(goBack()).toBe(false)
  })

  it('does not record a preview replacing a preview', () => {
    openTab({ path: 'a.php' }, { preview: true })
    openTab({ path: 'b.php' }, { preview: true })
    openTab({ path: 'c.php' }, { preview: true })
    goBack()
    expect(state.active).toBeNull()
    expect(state.back).toEqual([])
  })

  it('forgets the way forward when a preview takes the place of another', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' }, { preview: true })
    goBack()
    expect(state.forward).toHaveLength(1)
    selectTab(tabId('b.php'))
    goBack()
    // Forward now points at the preview of b.php, which is about to go.
    openTab({ path: 'b.php' }, { preview: true })
    openTab({ path: 'c.php' }, { preview: true })
    expect(state.forward).toEqual([])
  })

  it('brings a file’s own tab forward rather than closing it when a walk reaches it', () => {
    const kept = openTab({ path: 'b.php' })
    kept.dirty = true
    openTab({ path: 'a.php' })
    const landed = retarget({ path: 'b.php' })
    expect(landed.id).toBe(kept.id)
    expect(landed.dirty).toBe(true)
    expect(here()).toBe('b.php')
    expect(ids()).toEqual(['b.php', 'a.php'])
    // A preview that was doing the walking is what goes.
    openTab({ path: 'c.php' }, { preview: true })
    retarget({ path: 'a.php' })
    expect(ids()).toEqual(['b.php', 'a.php'])
    expect(here()).toBe('a.php')
  })

  it('does not record walking the files in one tab', () => {
    openTab({ path: 'a.php' })
    retarget({ path: 'b.php' })
    retarget({ path: 'c.php' })
    expect(ids()).toEqual(['c.php'])
    goBack()
    expect(state.active).toBeNull()
  })

  it('reopens a tab closed since', () => {
    openTab({ path: 'a.php', mode: 'file' })
    closeTab(tabId('a.php'))
    expect(state.tabs).toEqual([])
    goBack()
    expect(here()).toBe('a.php')
    expect(state.tabs[0]!.mode).toBe('file')
    expect(state.tabs[0]!.preview).toBe(false)
  })

  it('reopens a commit tab as the commit', () => {
    openTab({ path: 'a.php', commit: 'abc1234' })
    closeTab(tabId('a.php', 'abc1234'))
    goBack()
    expect(tabs.viewer.value).toEqual({ path: 'a.php', commit: 'abc1234' })
  })

  it('forgets the way forward once somewhere new is visited', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    goBack()
    expect(state.forward).toHaveLength(1)
    openTab({ path: 'c.php' })
    expect(state.forward).toEqual([])
    expect(goForward()).toBe(false)
  })

  it('keeps at most a hundred places each way', () => {
    for (let at = 0; at < HISTORY_LIMIT + 30; at++) openTab({ path: `f${at}.php` })
    expect(state.back).toHaveLength(HISTORY_LIMIT)
    while (goBack());
    expect(state.forward).toHaveLength(HISTORY_LIMIT)
  })

  it('skips places the window is already at', () => {
    openTab({ path: 'a.php' })
    showGraph()
    openTab({ path: 'a.php' })
    // Back from a.php is the graph, and back again is a.php — never a.php
    // twice in a row.
    goBack()
    expect(state.active).toBeNull()
    goBack()
    expect(here()).toBe('a.php')
  })

  it('carries the line a jump asked for', () => {
    openTab({ path: 'a.php' }, { line: 42 })
    expect(state.landing).toMatchObject({ tab: tabId('a.php'), line: 42 })
  })
})

describe('stepping through tabs', () => {
  it('counts the graph as the first tab and wraps', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    stepTab(1)
    expect(state.active).toBeNull()
    stepTab(1)
    expect(here()).toBe('a.php')
    stepTab(-1)
    expect(state.active).toBeNull()
  })
})

describe('one set of tabs per project', () => {
  it('keeps each project’s tabs and history for when it comes back', () => {
    openTab({ path: 'a.php' })
    switchTabs('/repos/other')
    expect(state.tabs).toEqual([])
    expect(state.active).toBeNull()
    openTab({ path: 'z.php' })
    switchTabs('/repos/app')
    expect(here()).toBe('a.php')
    expect(state.back).toHaveLength(1)
    switchTabs('/repos/other')
    expect(here()).toBe('z.php')
  })

  it('does not count coming back to a project as opening its file again', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    selectTab(tabId('a.php'))
    const order = [...state.recent]
    switchTabs('/repos/other')
    switchTabs('/repos/app')
    expect(here()).toBe('a.php')
    expect(state.recent).toEqual(order)
  })
})

describe('the mouse’s side buttons', () => {
  it('navigates on one route and drops the other route’s copy of the same press', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    expect(navFrom('back', 'native', 1000)).toBe(true)
    expect(here()).toBe('a.php')
    // The page's mouseup for the same press, a moment later.
    expect(navFrom('back', 'page', 1120)).toBe(false)
    expect(here()).toBe('a.php')
    // A second press is a second step.
    expect(navFrom('back', 'native', 1400)).toBe(true)
    expect(state.active).toBeNull()
  })

  it('acts on every press when only one route delivers them', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    navFrom('back', 'page', 1000)
    navFrom('back', 'page', 1200)
    expect(state.active).toBeNull()
    navFrom('forward', 'page', 1400)
    expect(here()).toBe('a.php')
  })
})

describe('jumps inside one file, and where they land', () => {
  it('remembers a jump to another line of the file on screen, so back returns', () => {
    openTab({ path: 'a.php' }, { line: 3 })
    openTab({ path: 'a.php' }, { line: 40 })
    expect(state.back.at(-1)).toMatchObject({ path: 'a.php', line: 3 })
    goBack()
    expect(tabs.activeTab()?.line).toBe(3)
    // The same line again is not a step.
    const steps = state.back.length
    openTab({ path: 'a.php' }, { line: 3 })
    expect(state.back).toHaveLength(steps)
  })

  it('lets a landing go once the view has used it', () => {
    openTab({ path: 'a.php' }, { line: 9 })
    const seq = state.landing!.seq
    consumeLanding(seq - 1)
    expect(state.landing).not.toBeNull()
    consumeLanding(seq)
    expect(state.landing).toBeNull()
  })

  it('finds a tab in a project that is not on screen', () => {
    switchTabs('/repos/one')
    openTab({ path: 'x.php' })
    switchTabs('/repos/two')
    expect(findTab('/repos/one', tabId('x.php'))?.path).toBe('x.php')
    expect(findTab('/repos/two', tabId('x.php'))).toBeUndefined()
  })
})

describe('the side buttons with something on top', () => {
  it('does nothing under a dialog, and a long press is still one step', () => {
    openTab({ path: 'a.php' })
    openTab({ path: 'b.php' })
    const scrim = document.createElement('div')
    scrim.className = 'overlay'
    document.body.appendChild(scrim)
    expect(navFrom('back', 'native', 1000)).toBe(false)
    expect(here()).toBe('b.php')
    scrim.remove()
    resetNav()
    // Held for two seconds: the page's copy comes on the way up.
    expect(navFrom('back', 'native', 5000)).toBe(true)
    expect(navFrom('back', 'page', 7000)).toBe(false)
    expect(here()).toBe('a.php')
  })
})
