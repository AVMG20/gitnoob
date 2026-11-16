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
 * The names come from tree-sitter in the backend (symbols.rs). A method's
 * usages are split: definite when the call's receiver is its class or one in
 * that hierarchy, possible when the receiver could not be worked out. Calls
 * on another class are not its usages at all. What the framework calls
 * (a command's `handle`, a controller action) is counted by the uses of its
 * class instead of by its name.
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

/** What the backend's `usages` answers. */
export interface UsageList {
  definite: CodeLocation[]
  possible: CodeLocation[]
  framework: boolean
  owner: string | null
}

export const usagesPopup = reactive({
  open: false,
  /** `Usages of total()`, `Definitions of Money`. */
  title: '',
  /** A line under the title: `handle() is called by the framework`. */
  note: '',
  items: [] as PopupItem[],
  /** Same name, receiver unknown: listed under a group, folded at first. */
  possible: [] as PopupItem[],
  showPossible: false,
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

/** The files `dependency_definition` reads names in: PHP, TypeScript, JavaScript. */
const RESOLVED = /\.(php|[cm]?[jt]sx?)$/i

/** Where `dependency_definition` lands: a place, maybe in a dependency. */
export interface DependencyJump extends CodeLocation {
  /** In vendor/ or node_modules/: it opens read-only. */
  dependency: boolean
  /** Something to say about it, such as a facade's method living elsewhere. */
  note: string | null
}

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

function marked(found: CodeLocation[]): PopupItem[] {
  return found
    .map((one, order) => ({ one: { ...one, state: stateOf(one.path) }, order }))
    .sort((a, b) => ORDER[a.one.state] - ORDER[b.one.state] || a.order - b.order)
    .map(({ one }) => one)
}

function showPopup(
  title: string,
  found: CodeLocation[],
  at: { x: number; y: number } | null,
  extra: { possible?: CodeLocation[]; note?: string } = {}
) {
  usagesPopup.title = title
  usagesPopup.note = extra.note ?? ''
  usagesPopup.items = marked(found)
  usagesPopup.possible = marked(extra.possible ?? [])
  usagesPopup.showPossible = !found.length
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

/**
 * The usages of a definition: a notice, a jump, or the list. `line` picks the
 * definition when a file has several of that name. ⌘-click jumps whenever
 * there is one definite usage (`jump: 'one'`); a click on the count only
 * when there is nothing else to show.
 */
export async function showUsages(
  name: string,
  path: string,
  kind: string,
  at: { x: number; y: number } | null = null,
  line: number | null = null,
  jump: 'one' | 'only' = 'only'
) {
  let list: UsageList | null
  try {
    list = await invoke<UsageList | null>('usages', { name, path, kind, line })
  } catch (error) {
    useToasts().info('Could not look up the usages', String(error))
    return
  }
  const definite = list?.definite ?? []
  const possible = list?.possible ?? []
  const framework = !!list?.framework
  const owner = list?.owner ?? null
  if (!definite.length && !possible.length) {
    if (framework) {
      useToasts().info(
        `${label(name, kind)} is called by the framework`,
        owner ? `Nothing in the code uses ${owner}.` : 'Nothing in the code calls it by name.'
      )
    } else {
      useToasts().info('No usages found', 'Templates and names built at run time are not looked through.')
    }
    return
  }
  if (definite.length === 1 && (jump === 'one' || !possible.length)) {
    openLocation(definite[0]!)
    return
  }
  const files = new Set(definite.map((one) => one.path)).size
  const where = `${definite.length} in ${files} ${files === 1 ? 'file' : 'files'}`
  const title = framework
    ? owner
      ? `Uses of ${owner} · ${definite.length}`
      : `References to ${label(name, kind)} · ${definite.length}`
    : definite.length
      ? `Usages of ${label(name, kind)} · ${where}`
      : `Possible usages of ${label(name, kind)} · ${possible.length}`
  showPopup(title, definite, at, {
    possible,
    note: framework ? `${label(name, kind)} is called by the framework` : ''
  })
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
    symbol = null
  }
  if (symbol?.definition) {
    await showUsages(symbol.name, path, symbol.kind, at, symbol.line, 'one')
    return
  }
  // PHP through the imports and Composer's autoload, scripts through
  // node_modules: one file, worked out the way the language does, and the
  // only way into a dependency, which the index leaves out.
  if (RESOLVED.test(path)) {
    const jump = await invoke<DependencyJump | null>('dependency_definition', { path, offset, text }).catch(
      () => null
    )
    if (jump) {
      openLocation(jump)
      if (jump.note) useToasts().info(jump.note)
      return
    }
  }
  if (!symbol) return
  let found: CodeLocation[]
  try {
    found =
      (await invoke<CodeLocation[] | null>('definitions', { name: symbol.name, path, shape: symbol.shape })) ?? []
  } catch (error) {
    useToasts().info('Could not look up the definition', String(error))
    return
  }
  if (!found.length) {
    if (symbol.shape !== 'member' && symbol.shape !== 'scoped') {
      useToasts().info(`No definition of ${symbol.name} in this repository`)
      return
    }
    // A method on something whose class could not be worked out. The
    // classes the file imports that have a method of that name are offered
    // to pick from, rather than nothing.
    const doubt = `Couldn't work out which class ${symbol.name} belongs to`
    const candidates = /\.php$/i.test(path)
      ? await invoke<DependencyJump[] | null>('member_candidates', { path, name: symbol.name, text })
          .then((all) => all ?? [])
          .catch(() => [] as DependencyJump[])
      : []
    if (!candidates.length) {
      useToasts().info(doubt)
      return
    }
    showPopup(`Possible definitions of ${symbol.name}() · ${candidates.length}`, candidates, at, { note: doubt })
    return
  }
  if (found.length === 1) {
    openLocation(found[0]!)
    return
  }
  showPopup(`Definitions of ${symbol.name} · ${found.length}`, found, at)
}
