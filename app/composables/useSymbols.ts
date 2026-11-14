import { reactive } from 'vue'
import { invoke } from './useInvoke'
import { useGit } from './useGit'
import { openTab } from './useTabs'
import { useToasts } from './useToasts'
import { change } from './useFileView'

/**
 * ⌘-click and ⌘J: from a name to where it is defined, or from a definition
 * to whoever uses it.
 *
 * The rules: on a reference, jump to its definition, or list them when there
 * are several. On a definition, no usages says so in a notice, one jumps
 * there, and more are listed with the ones in changed files first and marked.
 * Every jump is a pinned tab and a step in the history, so ⌘[ comes back.
 *
 * The names come from tree-sitter in the backend (symbols.rs), by name and
 * not by type: every method called `total` is a candidate for `->total()`.
 */

export type NameShape = 'plain' | 'member' | 'scoped' | 'type'

export interface SymbolAt {
  name: string
  kind: string
  definition: boolean
  shape: NameShape
  line: number
  from: number
  to: number
}

export interface CodeLocation {
  path: string
  line: number
  column: number
  kind: string
  text: string
}

/** Whether a usage sits in a file this branch changed, one it added, or neither. */
export type UsageState = 'changed' | 'new' | 'unchanged'

export interface PopupItem extends CodeLocation {
  state: UsageState
}

export const usagesPopup = reactive({
  open: false,
  /** `Usages of total()`, `Definitions of Money`. */
  title: '',
  items: [] as PopupItem[],
  cursor: 0,
  /** Where the click was, in window coordinates, for placing the list. */
  x: 0,
  y: 0
})

/** A file's place in the working tree panel, for marking a usage. */
export function stateOf(path: string): UsageState {
  const status = useGit().store.status
  if (!status) return 'unchanged'
  if (status.conflicted.includes(path)) return 'changed'
  const entry =
    status.unstaged.find((one) => one.path === path) ?? status.staged.find((one) => one.path === path)
  if (!entry) return 'unchanged'
  return change(entry.kind) === 'added' ? 'new' : 'changed'
}

let lastRequest = 0

/**
 * The next `unused_in` request number: from the clock, and shared by every
 * editor, so the backend's "a newer one came in" is right across files and
 * reloads.
 */
export function nextSymbolRequest() {
  lastRequest = Math.max(lastRequest + 1, Date.now())
  return lastRequest
}

const ORDER: Record<UsageState, number> = { changed: 0, new: 1, unchanged: 2 }

/** Opens a place: pinned, at its line, and remembered for ⌘[. */
export function openLocation(place: CodeLocation) {
  openTab({ path: place.path, mode: 'file', side: 'unstaged' }, { line: place.line, reveal: 'switch' })
}

function showPopup(title: string, found: CodeLocation[], at: { x: number; y: number } | null) {
  const items = found
    .map((one, order) => ({ one: { ...one, state: stateOf(one.path) }, order }))
    .sort((a, b) => ORDER[a.one.state] - ORDER[b.one.state] || a.order - b.order)
    .map(({ one }) => one)
  usagesPopup.title = title
  usagesPopup.items = items
  usagesPopup.cursor = 0
  usagesPopup.x = at?.x ?? Math.round(window.innerWidth / 2 - 220)
  usagesPopup.y = at?.y ?? 120
  usagesPopup.open = true
}

export function closeUsages() {
  usagesPopup.open = false
}

function label(name: string, kind: string) {
  return kind === 'function' || kind === 'method' ? `${name}()` : name
}

/** The usages of a definition: a notice, a jump, or the list. */
export async function showUsages(
  name: string,
  path: string,
  kind: string,
  at: { x: number; y: number } | null = null
) {
  let found: CodeLocation[]
  try {
    found = (await invoke<CodeLocation[] | null>('usages', { name, path, kind })) ?? []
  } catch (error) {
    useToasts().info('Could not look up the usages', String(error))
    return
  }
  if (!found.length) {
    useToasts().info('No usages found', 'Templates and names built at run time are not looked through.')
    return
  }
  if (found.length === 1) {
    openLocation(found[0]!)
    return
  }
  const files = new Set(found.map((one) => one.path)).size
  showPopup(
    `Usages of ${label(name, kind)} · ${found.length} in ${files} ${files === 1 ? 'file' : 'files'}`,
    found,
    at
  )
}

/**
 * ⌘-click or ⌘J at `offset` (UTF-16) in a file. `text` is the editor's,
 * which may be ahead of the disk.
 */
export async function jumpFrom(
  path: string,
  offset: number,
  text: string | null,
  at: { x: number; y: number } | null = null
) {
  let symbol: SymbolAt | null
  try {
    symbol = await invoke<SymbolAt | null>('symbol_at', { path, offset, text })
  } catch {
    return
  }
  if (!symbol) return
  if (symbol.definition) {
    await showUsages(symbol.name, path, symbol.kind, at)
    return
  }
  let found: CodeLocation[]
  try {
    found =
      (await invoke<CodeLocation[] | null>('definitions', { name: symbol.name, path, shape: symbol.shape })) ?? []
  } catch (error) {
    useToasts().info('Could not look up the definition', String(error))
    return
  }
  if (!found.length) {
    useToasts().info(`No definition of ${symbol.name} in this repository`)
    return
  }
  if (found.length === 1) {
    openLocation(found[0]!)
    return
  }
  showPopup(`Definitions of ${symbol.name} · ${found.length}`, found, at)
}
