import { nextTick, reactive } from 'vue'
import { ask } from '@tauri-apps/plugin-dialog'
import { aimedAt, invokeAt } from './useInvoke'
import { findTab } from './useTabs'
import type { ConflictBlock } from './useGit'

/**
 * Reading files for the editor and saving them back.
 *
 * The backend does the careful part (`edit.rs`): the path guard, the hash
 * check, line endings and the byte order mark, the atomic write. This side
 * keeps track of what is not saved yet, so nothing typed is lost when a tab is
 * closed, the window loses focus or the app quits.
 *
 * Everything here is kept per repository as well as per tab. Tab ids are
 * `work:<path>` in every project, and a save that finishes after the project
 * was switched must still go to the repository the edits were made in: every
 * read and write names its repository rather than following the one in front.
 */

export interface FileRead {
  /** With `\n` line endings and no byte order mark. */
  text: string
  /** Of the bytes on disk; a save has to name it. */
  hash: string
  eol: 'crlf' | 'lf'
  bom: boolean
  /** `binary`, `lfs`, `encoding`, `large` or `endings` when it cannot be edited. */
  readonly: string | null
  missing: boolean
}

/** What the backend says when a save would overwrite somebody else's. */
export function isConflict(error: unknown) {
  return String(error).replace(/^Error:\s*/, '').startsWith('Conflict')
}

/** The repository a read or write is for: given, or the one in front. */
function repoOf(repo: string | null | undefined) {
  return repo === undefined ? aimedAt() : repo
}

export async function readFile(path: string, repo?: string | null): Promise<FileRead | null> {
  try {
    const read = await invokeAt<FileRead>(repoOf(repo), 'read_file', { path })
    return read && typeof read.text === 'string' ? read : null
  } catch {
    return null
  }
}

/** How a file that is not on disk any more is written again. */
export interface Shape {
  eol: 'crlf' | 'lf'
  bom: boolean
}

/**
 * Writes over the file if it is still `expected`; resolves to the new hash.
 * `shape` is used only when the file is not there: a file deleted under the
 * editor comes back with the line endings and byte order mark it had.
 */
export function writeFile(
  path: string,
  text: string,
  expected: string | null,
  repo?: string | null,
  shape?: Shape | null
) {
  return invokeAt<string>(repoOf(repo), 'write_file', {
    path,
    text,
    expectedHash: expected,
    eol: shape?.eol ?? null,
    bom: shape?.bom ?? null
  })
}

/** One open file in one repository: what drafts and save states are kept by. */
export function editKey(repo: string | null, tab: string) {
  return `${repo ?? ''}\u0001${tab}`
}

/**
 * Edits not on disk yet: kept here rather than in the editor, so they outlive
 * it. A tab closed or switched away from mid-edit leaves its text here until
 * the save lands; one whose save was refused keeps it until the editor opens
 * again and asks what to do.
 */
export interface EditDraft {
  repo: string | null
  tab: string
  path: string
  text: string
  /** The hash of the file the edits were made on. */
  baseHash: string
  /** That file's text, so the editor knows what changed. */
  baseText: string
  shape?: Shape | null
}

export const drafts = new Map<string, EditDraft>()

/** How each open file stands, for the header and the tab strip. By `editKey`. */
export type SaveState = 'saved' | 'saving' | 'unsaved' | 'conflict' | 'failed' | 'readonly'

export const saveStates = reactive<Record<string, SaveState>>({})

/** Where the edits in an open file stand. */
export function saveStateOf(repo: string | null, tab: string): SaveState | undefined {
  return saveStates[editKey(repo, tab)]
}

/** Why the last save of a tab failed, for the header's "Not saved" tooltip. */
export const saveErrors = reactive<Record<string, string>>({})

export function setSaveError(repo: string | null, tab: string, error: string | null) {
  const key = editKey(repo, tab)
  if (error) saveErrors[key] = error
  else delete saveErrors[key]
}

export function saveErrorOf(repo: string | null, tab: string) {
  return saveErrors[editKey(repo, tab)] ?? null
}

/** Sets a tab's save state and its unsaved dot, in whichever project it is. */
export function setSaveState(repo: string | null, tab: string, state: SaveState) {
  saveStates[editKey(repo, tab)] = state
  const found = findTab(repo, tab)
  if (found) found.dirty = state === 'unsaved' || state === 'saving' || state === 'conflict' || state === 'failed'
}

/**
 * What a left-behind draft wrote, and the hash it left, by `editKey`: an
 * editor taking up a state kept warm from before the save learns from it
 * what is on disk now, without reading it.
 */
export const landed = new Map<string, { text: string; hash: string }>()

/** Saves under way for drafts, so an editor reopened meanwhile waits for them. */
export const flushing = new Map<string, Promise<void>>()

function track(key: string, run: Promise<void>) {
  flushing.set(key, run)
  void run.finally(() => {
    if (flushing.get(key) === run) flushing.delete(key)
  })
  return run
}

/**
 * Saves one left-behind draft. A refusal keeps it for the editor to ask
 * about; anything else drops it, since the file on disk is now the draft.
 */
async function flushDraft(draft: EditDraft) {
  const key = editKey(draft.repo, draft.tab)
  try {
    const hash = await writeFile(draft.path, draft.text, draft.baseHash, draft.repo)
    if (typeof hash === 'string') landed.set(key, { text: draft.text, hash })
    if (drafts.get(key) === draft) drafts.delete(key)
    setSaveState(draft.repo, draft.tab, 'saved')
  } catch (error) {
    setSaveState(draft.repo, draft.tab, isConflict(error) ? 'conflict' : 'failed')
  }
}

/**
 * Saves whatever is waiting: the editor on screen through the hook it
 * registered, saves already under way, and every draft. For the window
 * closing.
 */
export async function flushAll() {
  await Promise.all([...hooks].map((hook) => hook()))
  await Promise.all([...flushing.values()])
  await Promise.all(
    [...drafts.entries()]
      .filter(([key]) => !flushing.has(key))
      .map(([key, draft]) => track(key, flushDraft(draft)))
  )
}

/**
 * Leaves edits behind for a tab and saves them. `after` is a save the editor
 * still had under way: the draft is only made once it has landed, from the
 * hash it left, so the two never race each other into a false conflict.
 * `build` returns null when nothing is left to save by then.
 */
export function leave(
  repo: string | null,
  tab: string,
  build: () => EditDraft | null,
  after: Promise<unknown> | null = null
) {
  const key = editKey(repo, tab)
  if (abandoned.has(key)) {
    // Closed with "Close anyway": these edits were given up on.
    delete saveStates[key]
    return
  }
  setSaveState(repo, tab, 'saving')
  const run = (async () => {
    if (after) await after.catch(() => undefined)
    const draft = build()
    if (!draft) {
      setSaveState(repo, tab, 'saved')
      return
    }
    drafts.set(key, draft)
    await flushDraft(draft)
  })()
  track(key, run)
}

/** Tabs closed with their unsaved edits given up on, until they are gone. */
const abandoned = new Set<string>()

/** Editors on screen register how to save themselves now. */
const hooks = new Set<() => Promise<unknown>>()

export function onFlush(hook: () => Promise<unknown>) {
  hooks.add(hook)
  return () => hooks.delete(hook)
}

/** States that mean edits exist that are not on disk and will not get there alone. */
const STUCK: SaveState[] = ['conflict', 'failed', 'unsaved', 'saving']

/** The files whose edits would be lost, for the question before they are. */
function stuckPaths(keys: string[]) {
  return keys
    .filter((key) => STUCK.includes(saveStates[key]!))
    .map((key) => key.slice(key.indexOf('\u0001') + 1).replace(/^work:/, ''))
}

/**
 * Before the window goes: saves everything it can, waiting a while, and asks
 * when anything is still not on disk. True when it may go.
 */
export async function prepareToQuit(wait = 4000): Promise<boolean> {
  const done = await Promise.race([
    flushAll().then(() => true),
    new Promise<boolean>((settle) => setTimeout(() => settle(false), wait))
  ])
  const keys = Object.keys(saveStates)
  const stuck = stuckPaths(keys)
  if (done && !stuck.length) return true
  const names = stuck.length ? stuck.slice(0, 5).join(', ') + (stuck.length > 5 ? '…' : '') : 'some files'
  // A save that is only slow is not one that failed, and should not be
  // described as one.
  const broken = keys.some((key) => saveStates[key] === 'conflict' || saveStates[key] === 'failed')
  const message = broken
    ? `Edits in ${names} are not saved: the file changed on disk, or the save failed. Quit anyway and lose them?`
    : `Edits in ${names} are still being saved. Quit now and lose what has not been written yet?`
  return confirmLoss(message, 'Quit anyway')
}

async function confirmLoss(message: string, okLabel: string) {
  try {
    return await ask(message, { title: 'Unsaved edits', kind: 'warning', okLabel, cancelLabel: 'Keep them' })
  } catch {
    // No dialog to ask with (a browser, a test): keep the edits.
    return false
  }
}

/**
 * Closes tabs, asking first when one holds edits that could not be saved —
 * a conflict with the disk or a failed write. Closing anyway drops them.
 * True when the tabs were closed.
 */
export async function closeTabsSafely(ids: string[], close: () => void): Promise<boolean> {
  const repo = aimedAt()
  const risky = ids.filter((id) => {
    const state = saveStates[editKey(repo, id)]
    return state === 'conflict' || state === 'failed'
  })
  if (risky.length) {
    const names = risky.map((id) => id.replace(/^work:/, '')).join(', ')
    const ok = await confirmLoss(
      `${names} has edits that are not on disk: the file changed underneath them, or the save failed. Close anyway and lose them?`,
      'Close anyway'
    )
    if (!ok) return false
  }
  const keys = risky.map((id) => editKey(repo, id))
  for (const key of keys) {
    abandoned.add(key)
    drafts.delete(key)
    delete saveStates[key]
  }
  close()
  if (keys.length) {
    // The editor on screen goes on the next render and would leave its
    // edits behind as it does; after that the names are free again.
    await nextTick()
    for (const key of keys) abandoned.delete(key)
  }
  return true
}

/** For tests: forgets every draft and state. */
export function resetEdits() {
  drafts.clear()
  flushing.clear()
  landed.clear()
  abandoned.clear()
  for (const key of Object.keys(saveErrors)) delete saveErrors[key]
  for (const key of Object.keys(saveStates)) delete saveStates[key]
}

// --- comparing your edits with what is on disk ----------------------------------

/**
 * Your text and the text on disk as the conflict resolver's blocks: what both
 * have as context, and each place they differ as a region with your lines on
 * one side and the disk's on the other. The resolver's grid and picks are
 * built from these, so the comparison reads and answers like a conflict.
 */
export function compareBlocks(mine: string, theirs: string): ConflictBlock[] {
  const a = mine.split('\n')
  const b = theirs.split('\n')
  let head = 0
  while (head < a.length && head < b.length && a[head] === b[head]) head++
  let tail = 0
  while (
    tail < a.length - head &&
    tail < b.length - head &&
    a[a.length - 1 - tail] === b[b.length - 1 - tail]
  ) {
    tail++
  }
  const midA = a.slice(head, a.length - tail)
  const midB = b.slice(head, b.length - tail)

  // Longest common subsequence over the middle, where the two differ. Past a
  // few million cells it is one region rather than a pause.
  const pairs: [number, number][] = []
  if (midA.length && midB.length && midA.length * midB.length <= 4_000_000) {
    const width = midB.length + 1
    const table = new Uint32Array((midA.length + 1) * width)
    for (let i = midA.length - 1; i >= 0; i--) {
      for (let j = midB.length - 1; j >= 0; j--) {
        table[i * width + j] =
          midA[i] === midB[j]
            ? table[(i + 1) * width + j + 1]! + 1
            : Math.max(table[(i + 1) * width + j]!, table[i * width + j + 1]!)
      }
    }
    let i = 0
    let j = 0
    while (i < midA.length && j < midB.length) {
      if (midA[i] === midB[j]) {
        pairs.push([i, j])
        i++
        j++
      } else if (table[(i + 1) * width + j]! >= table[i * width + j + 1]!) i++
      else j++
    }
  }

  const blocks: ConflictBlock[] = []
  const context = (lines: string[]) => {
    if (!lines.length) return
    const last = blocks[blocks.length - 1]
    if (last?.kind === 'context') last.lines.push(...lines)
    else blocks.push({ kind: 'context', lines: [...lines] })
  }
  let index = 0
  const region = (ours: string[], theirs: string[]) => {
    if (!ours.length && !theirs.length) return
    blocks.push({
      kind: 'conflict',
      index: index++,
      ours,
      base: [],
      theirs,
      has_base: false,
      ours_label: 'Your edits',
      theirs_label: 'On disk'
    } as ConflictBlock)
  }
  context(a.slice(0, head))
  let i = 0
  let j = 0
  for (const [pi, pj] of [...pairs, [midA.length, midB.length] as [number, number]]) {
    region(midA.slice(i, pi), midB.slice(j, pj))
    if (pi < midA.length) context([midA[pi]!])
    i = pi + 1
    j = pj + 1
  }
  context(a.slice(a.length - tail))
  return blocks
}
