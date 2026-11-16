import { computed, reactive, watch } from 'vue'
import { diffMode, type DiffMode } from './useDiffMode'
import { clearWarm, pruneWarm } from './editorBuffers'

/**
 * The files open in the middle of the window, and the way back through them.
 *
 * The graph is home and always the first tab; it is `active === null` here
 * rather than a tab in the list, because it cannot be closed, moved or opened
 * twice. Everything else is a file, either as it stands in the work tree or as
 * it stood in one commit.
 *
 * Each project tab has its own set, kept for the session: switching projects
 * and coming back finds the files where they were left. Nothing survives a
 * restart, the same as the graph's solo.
 *
 * `store.viewer` in useGit is worked out from the active tab, in the shape it
 * always had, so the diff viewer, staging and the rest read it as before.
 */

export interface Tab {
  /** `work:<path>` for the work tree, `<commit>:<path>` for a commit. */
  id: string
  /** Relative to the root of the work tree. */
  path: string
  mode: DiffMode
  /** Which list a working file is read from in Diff mode. */
  side?: 'staged' | 'unstaged'
  /** Set for a read-only look at the file as one commit has it. */
  commit?: string
  /** Opened by a single click: drawn in italics, and the next one replaces it. */
  preview: boolean
  /** Edits not on disk yet. Always false until the file can be edited. */
  dirty: boolean
  caret?: number
  /** The line the caret is on, so back and forward come back to it. */
  line?: number
  scrollTop?: number
}

/** Somewhere the window has been, for back and forward. `tab` null is the graph. */
export interface Place {
  tab: string | null
  path?: string
  commit?: string
  side?: 'staged' | 'unstaged'
  mode?: DiffMode
  line?: number
}

/** What to open: a file, and how to look at it. */
export interface TabTarget {
  path: string
  commit?: string
  side?: 'staged' | 'unstaged'
  mode?: DiffMode
}

export interface OpenOptions {
  /** A single click: replaces the current preview tab rather than adding one. */
  preview?: boolean
  /** False keeps the history as it is. */
  history?: boolean
  /** Where in the file to land, for a jump or a search result. */
  line?: number
  /**
   * Whether the sidebar's Files tree comes forward to show the file.
   * `switch`: a file that was looked for (search, recent files, a jump, the
   * usages popup) switches the sidebar to Files and reveals it there.
   * `ifShown`, the default: the tree reveals it only if it is already on
   * screen, so opening from the right panel or a commit leaves Branches be.
   */
  reveal?: 'switch' | 'ifShown'
}

/** What the viewer is pointed at, in the shape `store.viewer` has always had. */
export interface Viewer {
  path: string
  side?: 'staged' | 'unstaged'
  commit?: string
}

interface TabSet {
  tabs: Tab[]
  active: string | null
  back: Place[]
  forward: Place[]
  recent: string[]
}

/** How many recently opened files the finder remembers per project. */
export const RECENT_LIMIT = 50

/** Each way, the way a browser caps it. */
export const HISTORY_LIMIT = 100

export function tabId(path: string, commit?: string | null) {
  return `${commit || 'work'}:${path}`
}

const state = reactive({
  tabs: [] as Tab[],
  active: null as string | null,
  back: [] as Place[],
  forward: [] as Place[],
  /** Work-tree files opened lately, newest first, for the file finder. */
  recent: [] as string[],
  /** A line a jump asked for, carried until the view has scrolled to it. */
  landing: null as { tab: string; line: number; seq: number } | null
})

/** Every other project's set, by path, while this one is on screen. */
const sessions = new Map<string, TabSet>()
let current: string | null = null
let landingSeq = 0

export function activeTab(): Tab | null {
  if (state.active === null) return null
  return state.tabs.find((tab) => tab.id === state.active) ?? null
}

function placeOf(tab: Tab | null): Place {
  if (!tab) return { tab: null }
  return { tab: tab.id, path: tab.path, commit: tab.commit, side: tab.side, mode: tab.mode, line: tab.line }
}

function samePlace(a: Place | undefined, b: Place | undefined) {
  return !!a && !!b && a.tab === b.tab && (a.line ?? null) === (b.line ?? null)
}

function capped(list: Place[]) {
  if (list.length > HISTORY_LIMIT) list.splice(0, list.length - HISTORY_LIMIT)
}

/** Leaves where the window is now on the back stack, and forgets the way forward. */
function remember(place: Place = placeOf(activeTab())) {
  if (!samePlace(state.back[state.back.length - 1], place)) {
    state.back.push(place)
    capped(state.back)
  }
  state.forward = []
}

/** Puts a tab on screen, and the view it was left in with it. */
function activate(tab: Tab | null) {
  state.active = tab ? tab.id : null
  if (tab && diffMode.mode !== tab.mode) diffMode.mode = tab.mode
  if (tab && !tab.commit) {
    state.recent = [tab.path, ...state.recent.filter((path) => path !== tab.path)].slice(0, RECENT_LIMIT)
  }
}

function land(tab: Tab, line?: number) {
  if (line === undefined || line === null) return
  tab.line = line
  landingSeq += 1
  state.landing = { tab: tab.id, line, seq: landingSeq }
}

/**
 * Opens a file, or selects the tab it already has.
 *
 * A preview open takes the place of the preview tab there is, so clicking down
 * a list of files reads them one after another instead of piling up tabs.
 * Anything else — a double click, a jump, a search result — opens it pinned.
 */
export function openTab(target: TabTarget, options: OpenOptions = {}) {
  const preview = !!options.preview
  const id = tabId(target.path, target.commit)
  const from = activeTab()
  const existing = state.tabs.find((tab) => tab.id === id)

  if (existing) {
    // A jump inside the file on screen is a step too, so back returns from it.
    const moving =
      state.active !== id || (options.line !== undefined && options.line !== existing.line)
    if (options.history !== false && moving) remember()
    if (target.side) existing.side = target.side
    if (target.mode) existing.mode = target.mode
    if (!preview) existing.preview = false
    activate(existing)
    land(existing, options.line)
    revealIf(target, options)
    return existing
  }

  const tab: Tab = {
    id,
    path: target.path,
    commit: target.commit || undefined,
    side: target.commit ? undefined : (target.side ?? 'unstaged'),
    mode: target.mode ?? diffMode.mode,
    preview,
    dirty: false
  }

  // Stepping from one preview to the next is reading down a list, not going
  // anywhere: only the first step of it is worth coming back to.
  const replacingSelf = preview && !!from?.preview
  if (options.history !== false) {
    if (!replacingSelf) remember()
    // Not a step worth coming back to, but still somewhere new: what was
    // ahead is not ahead of this.
    else state.forward = []
  }

  const old = preview ? state.tabs.findIndex((one) => one.preview) : -1
  if (old >= 0) {
    state.tabs.splice(old, 1, tab)
    forgetClosed()
  } else {
    const at = from ? state.tabs.indexOf(from) + 1 : state.tabs.length
    state.tabs.splice(at, 0, tab)
  }
  activate(tab)
  land(tab, options.line)
  revealIf(target, options)
  return tab
}

/**
 * What brings the Files tree forward on a file. `useFileTree` hands it over
 * when it loads: importing it from here would be a cycle through `useGit`,
 * which reads the tabs as it starts.
 */
let revealInFiles: (path: string) => void = () => undefined

export function setFileRevealer(reveal: (path: string) => void) {
  revealInFiles = reveal
}

/**
 * Brings the Files tree forward on a file that was looked for. The tree
 * itself reveals whatever tab is active whenever it is on screen, so
 * `ifShown` needs nothing here.
 */
function revealIf(target: TabTarget, options: OpenOptions) {
  if (options.reveal === 'switch' && !target.commit) revealInFiles(target.path)
}

/**
 * Points the tab on screen at another file, in place.
 *
 * Walking the changed files with the arrows, and staging moving on to the next
 * one, read one file after another in the same tab rather than opening more.
 */
export function retarget(target: TabTarget) {
  const tab = activeTab()
  if (!tab) return openTab(target)
  const id = tabId(target.path, target.commit)
  if (id !== tab.id) {
    const twin = state.tabs.find((one) => one.id === id)
    if (twin) {
      // The file already has a tab of its own, kept on purpose and perhaps
      // holding edits: that one comes forward. The tab that was walking is
      // only dropped when it was a preview, which is what a preview is for.
      if (tab.preview) state.tabs.splice(state.tabs.indexOf(tab), 1)
      if (target.side) twin.side = target.side
      if (target.mode) twin.mode = target.mode
      activate(twin)
      return twin
    }
    // Another file is read from its own start, not from where this one was.
    tab.scrollTop = undefined
    tab.caret = undefined
  }
  tab.id = id
  tab.path = target.path
  tab.commit = target.commit || undefined
  tab.side = target.commit ? undefined : (target.side ?? tab.side ?? 'unstaged')
  if (target.mode) tab.mode = target.mode
  state.active = id
  if (diffMode.mode !== tab.mode) diffMode.mode = tab.mode
  return tab
}

/**
 * Moves the caret in the tab on screen to a line, and remembers where it was:
 * going to a line is a jump, and back returns from it.
 */
export function goToLine(line: number) {
  const tab = activeTab()
  if (!tab) return
  remember()
  tab.line = line
  land(tab, line)
}

/**
 * Says the view has gone to the line a jump asked for, so coming back to the
 * tab later finds it where it was left rather than at the jump again.
 */
export function consumeLanding(seq: number) {
  if (state.landing?.seq === seq) state.landing = null
}

/** A tab in a project's set, whether that project is on screen or not. */
export function findTab(project: string | null, id: string): Tab | undefined {
  // Null is no repository named: the set on screen.
  const list = project === current || project === null ? state.tabs : sessions.get(project)?.tabs
  return list?.find((tab) => tab.id === id)
}

/** Switches to a tab that is already open. */
export function selectTab(id: string | null) {
  if (id === state.active) return
  if (id === null) return showGraph()
  const tab = state.tabs.find((one) => one.id === id)
  if (!tab) return
  remember()
  activate(tab)
}

/** Keeps a preview tab, so the next single click opens beside it. */
export function pinTab(id: string) {
  const tab = state.tabs.find((one) => one.id === id)
  if (tab) tab.preview = false
}

/** Back to the graph. The file tabs stay where they are. */
export function showGraph() {
  if (state.active === null) return
  remember()
  activate(null)
}

/**
 * Closes a tab. Closing the one on screen moves to its left neighbour, or the
 * right one when it was first; the last one closed brings the graph back.
 *
 * Closing the tab on screen leaves it on the back stack, so back reopens it.
 */
export function closeTab(id: string) {
  const at = state.tabs.findIndex((tab) => tab.id === id)
  if (at < 0) return
  if (state.active !== id) {
    state.tabs.splice(at, 1)
    forgetClosed()
    return
  }
  remember()
  state.tabs.splice(at, 1)
  activate(state.tabs[at - 1] ?? state.tabs[at] ?? null)
  forgetClosed()
}

export function closeOthers(id: string) {
  const keep = state.tabs.find((tab) => tab.id === id)
  if (!keep) return
  if (state.active !== id) remember()
  state.tabs = [keep]
  activate(keep)
  forgetClosed()
}

export function closeAll() {
  if (state.active !== null) remember()
  state.tabs = []
  activate(null)
  forgetClosed()
}

/** Closes the file tabs a test says are done with, without moving the window. */
export function closeWhere(test: (tab: Tab) => boolean) {
  const active = activeTab()
  if (active && test(active)) closeTab(active.id)
  state.tabs = state.tabs.filter((tab) => !test(tab))
  forgetClosed()
}

/** A closed tab's editor is not kept warm (`editorBuffers.ts`). */
function forgetClosed() {
  pruneWarm(state.tabs.map((tab) => tab.id))
}

/** Goes to a place: the graph, a tab that is open, or one closed since. */
function go(place: Place) {
  if (place.tab === null) {
    activate(null)
    return
  }
  let tab = state.tabs.find((one) => one.id === place.tab)
  if (!tab && place.path) {
    tab = {
      id: place.tab,
      path: place.path,
      commit: place.commit,
      side: place.commit ? undefined : (place.side ?? 'unstaged'),
      mode: place.mode ?? diffMode.mode,
      preview: false,
      dirty: false
    }
    state.tabs.push(tab)
  }
  if (!tab) return
  activate(tab)
  land(tab, place.line)
  // Back and forward land on a file the way a jump does.
  if (!tab.commit) revealInFiles(tab.path)
}

/** One step back. Places the window is already at are skipped. */
export function goBack() {
  const here = placeOf(activeTab())
  while (state.back.length) {
    const place = state.back.pop()!
    if (samePlace(place, here)) continue
    state.forward.push(here)
    capped(state.forward)
    go(place)
    return true
  }
  return false
}

export function goForward() {
  const here = placeOf(activeTab())
  while (state.forward.length) {
    const place = state.forward.pop()!
    if (samePlace(place, here)) continue
    state.back.push(here)
    capped(state.back)
    go(place)
    return true
  }
  return false
}

/** The next tab along, the graph included, wrapping at either end. */
export function stepTab(by: number) {
  const ids: (string | null)[] = [null, ...state.tabs.map((tab) => tab.id)]
  if (ids.length < 2) return
  const at = ids.indexOf(state.active)
  selectTab(ids[(at + by + ids.length) % ids.length] ?? null)
}

/**
 * Swaps in another project's tabs, keeping this one's for when it comes back.
 * Null is no project at all.
 */
export function switchTabs(path: string | null) {
  if (path === current) return
  // Editors are kept warm for the project in front only. The ones going
  // away leave their unsaved edits to the drafts as they go.
  clearWarm(path)
  if (current !== null) {
    sessions.set(current, {
      tabs: state.tabs,
      active: state.active,
      back: state.back,
      forward: state.forward,
      recent: state.recent
    })
  }
  current = path
  const saved = path === null ? undefined : sessions.get(path)
  state.tabs = saved?.tabs ?? []
  state.back = saved?.back ?? []
  state.forward = saved?.forward ?? []
  state.recent = saved?.recent ?? []
  state.landing = null
  // Put back as it was left, not opened: the list of recent files is about
  // what was opened, and coming back to a project opens nothing.
  const active = saved ? (saved.tabs.find((tab) => tab.id === saved.active) ?? null) : null
  state.active = active ? active.id : null
  if (active && diffMode.mode !== active.mode) diffMode.mode = active.mode
}

/** Drops every project's tabs. For tests, and for nothing else. */
export function resetTabs() {
  sessions.clear()
  current = null
  clearWarm(null)
  state.tabs = []
  state.active = null
  state.back = []
  state.forward = []
  state.recent = []
  state.landing = null
}

const viewer = computed<Viewer | null>(() => {
  const tab = activeTab()
  if (!tab) return null
  return tab.commit ? { path: tab.path, commit: tab.commit } : { path: tab.path, side: tab.side }
})

/**
 * The Diff | File switch belongs to the tab: flipping it is flipping the tab
 * on screen, and coming back to a tab finds it the way it was left.
 */
watch(
  () => diffMode.mode,
  (mode) => {
    const tab = activeTab()
    if (tab) tab.mode = mode
  },
  { flush: 'sync' }
)

export function useTabs() {
  return {
    state,
    viewer,
    activeTab,
    openTab,
    retarget,
    selectTab,
    pinTab,
    showGraph,
    closeTab,
    closeOthers,
    closeAll,
    closeWhere,
    goBack,
    goForward,
    goToLine,
    stepTab,
    switchTabs
  }
}
