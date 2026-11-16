import { Compartment, type ChangeSet, type EditorState, type StateEffect } from '@codemirror/state'
import type { EditorconfigIndent } from './indentation'
import type { Shape } from './useFileEdit'

/**
 * Open files kept warm, and handed between the views that show them.
 *
 * Switching away from a tab used to throw its editor away, and coming back
 * read the file again, parsed it again and coloured it again, visibly. Now an
 * editor going away puts its `EditorState` here: the document, the parse
 * tree, the undo history, the selection, the change bars, the usage counts
 * and the dimmed code are all in it. The next editor for that tab starts from
 * it and draws coloured text on its first frame.
 *
 * The same slot is what keeps two views of one file from becoming two
 * buffers. A file is held by one editor at a time: when a second one opens
 * it (the search window's preview over the tab showing it), the first lends
 * it the state and stops saving, and shows each edit made in the other as it
 * is made; when the second goes, the first takes the state back, with its
 * undo history, and saves it.
 *
 * Only the project on screen is kept: switching projects drops the lot, after
 * the editors going away have left their unsaved edits to the usual drafts.
 * Within it, the twelve most recent tabs are kept, a closed tab is dropped,
 * and a file over a megabyte is not kept at all.
 */

/** How many tabs are kept warm. */
export const WARM_MAX = 12
/** Past this many characters a file is read again rather than kept. */
export const WARM_LIMIT = 1_000_000

/**
 * The editor's slots, shared by every editor so a state built by one can be
 * reconfigured by the next. `bound` holds what calls back into the editor
 * component (listeners, click handlers), swapped for the new one's on the way
 * in.
 */
export const slots = {
  language: new Compartment(),
  colours: new Compartment(),
  access: new Compartment(),
  indenting: new Compartment(),
  bound: new Compartment()
}

/** Everything an editor needs to carry on where another left off. */
export interface Held {
  path: string
  state: EditorState
  /** Where it was scrolled to, as CodeMirror puts it back. */
  scroll: StateEffect<unknown> | null
  /** The file on disk the edits were made on. */
  base: { text: string; hash: string | null; shape: Shape | null }
  reason: string | null
  editorconfig: EditorconfigIndent | null
  /** The colour version the state's highlighting was made with. */
  colours: number
  /** The work tree's count of changes when it was put away. */
  epoch: number
  /** A save still on its way when it was handed over: `base` is final after it. */
  settle: Promise<void> | null
}

/** A live editor holding a file. */
export interface Holder {
  /** Hands the state over and stops saving, until it is given back. */
  lend(): Held | null
  /** Takes the state back, with whatever the other editor did to it. */
  reclaim(held: Held | null): void
  /** While lent: an edit made in the editor that has it, to show here too. */
  follow?(changes: ChangeSet): void
  /**
   * Keeps the file while it is open (the search window's preview): an editor
   * opening the same file meanwhile starts from a copy and follows it, and
   * takes it over when this one goes.
   */
  sticky?: boolean
  /** For a sticky holder: what it has, without giving it up. */
  peek?(): Held | null
}

interface Slot {
  parked: Held | null
  /** The editors that have it, the one in charge last. */
  holders: Holder[]
}

/** By repository and tab: a tab id is the same in every project. */
const all = new Map<string, Slot>()
let project: string | null = null
let epoch = 0

function k(repo: string | null, tab: string) {
  return `${repo ?? ''}\u0001${tab}`
}

/** Something in the work tree changed: what was put away may be out of date. */
export function workTreeMoved() {
  epoch++
}

export function workTreeEpoch() {
  return epoch
}

/** Whether a file has changed on disk, as far as anyone can tell, since it was put away. */
export function isStale(held: Held) {
  return held.epoch !== epoch
}

/**
 * Takes a tab's file for an editor. Returns the state to start from: the one
 * kept warm, or the one the editor already holding it lends. Null means read
 * it from disk.
 */
export function acquire(repo: string | null, tab: string, holder: Holder): Held | null {
  const key = k(repo, tab)
  let slot = all.get(key)
  if (!slot) {
    slot = { parked: null, holders: [] }
  }
  let held = repo === project ? slot.parked : null
  slot.parked = null
  const current = slot.holders[slot.holders.length - 1]
  if (current?.sticky) {
    // It keeps the file: this one goes under it, following.
    held = current.peek?.() ?? null
    slot.holders.splice(slot.holders.length - 1, 0, holder)
  } else {
    if (current) held = current.lend()
    slot.holders.push(holder)
  }
  // Most recent last, for the trimming.
  all.delete(key)
  all.set(key, slot)
  return held
}

/** Whether an editor is the one in charge of its file, rather than following another. */
export function inCharge(repo: string | null, tab: string, holder: Holder) {
  const holders = all.get(k(repo, tab))?.holders ?? []
  return holders[holders.length - 1] === holder
}

/**
 * Gives a file back as its editor goes: to the editor it was taken from, or
 * put away warm. An editor that had lent it has nothing to give.
 */
export function release(
  repo: string | null,
  tab: string,
  holder: Holder,
  held: Held | null
): 'handed' | 'kept' | 'dropped' {
  const key = k(repo, tab)
  const slot = all.get(key)
  if (!slot) return 'dropped'
  const at = slot.holders.indexOf(holder)
  if (at < 0) return 'dropped'
  const inCharge = at === slot.holders.length - 1
  slot.holders.splice(at, 1)
  if (!inCharge) return 'dropped'
  const previous = slot.holders[slot.holders.length - 1]
  if (previous) {
    previous.reclaim(held)
    return 'handed'
  }
  if (held && repo === project && held.state.doc.length <= WARM_LIMIT) {
    slot.parked = held
    // Most recently left last: the one used all afternoon is not the first
    // to go just because it was the first opened.
    all.delete(key)
    all.set(key, slot)
    trim()
    return 'kept'
  }
  all.delete(key)
  return 'dropped'
}

/** An edit made by the editor in charge of a file, shown in the one it borrowed it from. */
export function follow(repo: string | null, tab: string, holder: Holder, changes: ChangeSet) {
  const slot = all.get(k(repo, tab))
  if (!slot) return
  const at = slot.holders.indexOf(holder)
  if (at > 0 && at === slot.holders.length - 1) slot.holders[at - 1]!.follow?.(changes)
}

function trim() {
  const parked = [...all.entries()].filter(([, slot]) => slot.parked && !slot.holders.length)
  for (const [key] of parked.slice(0, Math.max(0, parked.length - WARM_MAX))) all.delete(key)
}

/** Whether a tab's file is kept warm, for tests and for the viewer. */
export function isWarm(tab: string, repo: string | null = project) {
  return !!all.get(k(repo, tab))?.parked
}

/** How many tabs are kept warm, for tests. */
export function warmCount() {
  return [...all.values()].filter((slot) => slot.parked).length
}

/** Forgets a tab's kept state in the project in front: its tab closed, or its file went. */
export function dropWarm(tab: string, repo: string | null = project) {
  const key = k(repo, tab)
  const slot = all.get(key)
  if (!slot) return
  slot.parked = null
  if (!slot.holders.length) all.delete(key)
}

/**
 * Keeps only the tabs of the project in front that are still open, and
 * anything an editor has on screen.
 */
export function pruneWarm(open: Iterable<string>) {
  const keep = new Set([...open].map((tab) => k(project, tab)))
  for (const [key, slot] of [...all.entries()]) {
    if (!keep.has(key) && !slot.holders.length) all.delete(key)
  }
}

/** Another project in front: nothing of the last one is kept. */
export function clearWarm(next: string | null) {
  project = next
  for (const [key, slot] of [...all.entries()]) {
    slot.parked = null
    if (!slot.holders.length) all.delete(key)
  }
}

/** For tests. */
export function resetWarm() {
  all.clear()
  project = null
  epoch = 0
}
