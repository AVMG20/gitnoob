import { onMounted, onUnmounted, reactive } from 'vue'

/**
 * The keyboard, in one place.
 *
 * The list below is both the binding and the documentation: the dispatcher
 * matches against it, and the Shortcuts page in settings renders it. A key that
 * is written down but never registered still shows, which is what the rows
 * handled inside one component — the arrows in the commit list, ⌘↵ in the
 * commit box — need, and a key that changes changes in one place.
 */

export type ShortcutGroup =
  | 'Repository'
  | 'Branches and commits'
  | 'The commit list'
  | 'Changes and diffs'
  | 'Projects and windows'

export interface Shortcut {
  /** The name a component registers a handler under. */
  id: string
  /** `mod` is ⌘ on macOS and Ctrl everywhere else. */
  keys: string
  /** What it does, in the imperative. */
  label: string
  /** Where the key has to be for it to work. */
  where: string
  group: ShortcutGroup
  /** When it will not fire, where that is worth saying. */
  note?: string
}

const ANYWHERE = 'Anywhere in the window'
const WORKING = 'While the working tree is shown'

export const SHORTCUTS: Shortcut[] = [
  // Repository
  { id: 'repo.fetch', keys: 'mod+shift+f', label: 'Fetch every remote', where: ANYWHERE, group: 'Repository' },
  { id: 'repo.pull', keys: 'mod+shift+l', label: 'Pull, stashing open work first', where: ANYWHERE, group: 'Repository' },
  { id: 'repo.push', keys: 'mod+shift+p', label: 'Push the current branch', where: ANYWHERE, group: 'Repository', note: 'Asks first when the push would drop commits.' },
  { id: 'repo.refresh', keys: 'mod+r', label: 'Re-read the repository', where: ANYWHERE, group: 'Repository' },
  { id: 'repo.settings', keys: 'mod+,', label: 'Open settings', where: ANYWHERE, group: 'Repository' },

  // Branches and commits
  { id: 'branch.create', keys: 'mod+b', label: 'New branch', where: ANYWHERE, group: 'Branches and commits' },
  { id: 'stash.push', keys: 'mod+shift+s', label: 'Stash everything', where: ANYWHERE, group: 'Branches and commits' },
  { id: 'history.undo', keys: 'mod+z', label: 'Undo the last operation', where: ANYWHERE, group: 'Branches and commits', note: 'Refuses once the branch has moved on. Not while a text box has the caret.' },
  { id: 'history.redo', keys: 'mod+shift+z', label: 'Redo', where: ANYWHERE, group: 'Branches and commits' },
  { id: 'commit.write', keys: 'mod+enter', label: 'Commit what is staged', where: 'The commit message box', group: 'Branches and commits' },

  // The commit list
  { id: 'graph.search', keys: 'mod+f', label: 'Search by message, author or hash', where: ANYWHERE, group: 'The commit list' },
  { id: 'graph.next', keys: 'mod+g', label: 'Next match', where: ANYWHERE, group: 'The commit list' },
  { id: 'graph.previous', keys: 'mod+shift+g', label: 'Previous match', where: ANYWHERE, group: 'The commit list' },
  { id: 'graph.move', keys: 'ArrowUp/ArrowDown', label: 'Move one commit', where: 'The commit list', group: 'The commit list', note: 'Not while typing, and not while a dialog is open.' },
  { id: 'graph.page', keys: 'PageUp/PageDown', label: 'Move a screenful', where: 'The commit list', group: 'The commit list' },
  { id: 'graph.ends', keys: 'Home/End', label: 'First or last commit', where: 'The commit list', group: 'The commit list' },
  { id: 'graph.extend', keys: 'shift+click', label: 'Mark a run of commits', where: 'The commit list', group: 'The commit list', note: 'mod+click marks one more on its own.' },
  { id: 'graph.checkout', keys: 'double-click', label: 'Check out the branch on a chip', where: 'A branch chip in the commit list', group: 'The commit list' },
  { id: 'graph.clearsearch', keys: 'Escape', label: 'Close the search box', where: 'The commit list', group: 'The commit list' },
  { id: 'graph.solo', keys: 'alt+click', label: 'Solo a branch in the graph, or stop', where: 'A branch in the sidebar', group: 'The commit list', note: 'Escape shows every branch again. Right-click a branch to solo more than one.' },

  // Changes and diffs
  { id: 'diff.mode', keys: 'Tab', label: 'Show the whole file, or only the lines that changed', where: 'An open file', group: 'Changes and diffs', note: 'Blame is a column of the file view, on the line numbers’ right-click menu.' },
  { id: 'viewer.move', keys: 'ArrowUp/ArrowDown', label: 'Move to the file before or after this one', where: 'An open file', group: 'Changes and diffs', note: 'Walks the panel beside it, in the order it lists them.' },
  { id: 'viewer.search', keys: 'mod+f', label: 'Find text in the file', where: 'An open file', group: 'Changes and diffs', note: 'Searches whichever view is on screen. ↑, ↓ and ↵ in the box step through the matches.' },
  { id: 'viewer.next', keys: 'mod+g', label: 'Next match in the file', where: 'An open file', group: 'Changes and diffs' },
  { id: 'viewer.previous', keys: 'mod+shift+g', label: 'Previous match in the file', where: 'An open file', group: 'Changes and diffs' },
  { id: 'viewer.close', keys: 'Escape', label: 'Close the find box, then the file', where: 'An open file', group: 'Changes and diffs' },
  { id: 'conflict.move', keys: 'ArrowUp/ArrowDown', label: 'Move to the conflict before or after this one', where: 'The conflict resolver', group: 'Changes and diffs' },
  { id: 'conflict.undecided', keys: 'Tab', label: 'Jump to the next conflict nobody has decided yet', where: 'The conflict resolver', group: 'Changes and diffs' },
  { id: 'filter.clear', keys: 'Escape', label: 'Clear the branch filter', where: 'The filter box in the sidebar', group: 'Changes and diffs' },

  // Projects and windows
  { id: 'project.open', keys: 'mod+o', label: 'Open a repository', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'project.switch', keys: 'mod+p', label: 'Go to a repository', where: ANYWHERE, group: 'Projects and windows', note: 'Searches the open tabs and everything opened before them.' },
  { id: 'project.close', keys: 'mod+w', label: 'Close the current project tab', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'project.next', keys: 'mod+alt+ArrowRight', label: 'Next project tab', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'project.previous', keys: 'mod+alt+ArrowLeft', label: 'Previous project tab', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'project.nth', keys: 'mod+1…9', label: 'Jump to a project tab by position', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'dialog.close', keys: 'Escape', label: 'Close the dialog on top', where: 'Any dialog', group: 'Projects and windows' },
  { id: 'zoom.in', keys: 'mod+=', label: 'Make everything larger', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'zoom.out', keys: 'mod+-', label: 'Make everything smaller', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'zoom.reset', keys: 'mod+0', label: 'Back to the standard size', where: ANYWHERE, group: 'Projects and windows' },

  // Branches and commits, continued
  { id: 'branch.start', keys: 'mod+shift+b', label: 'Start work: fetch, then branch from main', where: ANYWHERE, group: 'Branches and commits', note: 'Open changes come along.' },
  { id: 'branch.update', keys: 'mod+shift+o', label: 'Update this branch from main', where: ANYWHERE, group: 'Branches and commits', note: 'Rebases a branch only you have, merges into one that is pushed. Settings › Behaviour changes that.' },
  // The working tree to commit loop. Chosen clear of the system's own keys and
  // of everything above; every one of them can be moved in settings.
  { id: 'commit.push', keys: 'mod+shift+enter', label: 'Commit what is staged, then push', where: WORKING, group: 'Branches and commits', note: 'An amend of a commit already on the remote stops at the force-push question.' },
  { id: 'commit.focus', keys: 'mod+shift+m', label: 'Put the caret in the commit message', where: ANYWHERE, group: 'Branches and commits' },
  { id: 'commit.amend', keys: 'mod+shift+e', label: 'Amend the previous commit, or stop amending', where: WORKING, group: 'Branches and commits' },
  { id: 'commit.generate', keys: 'mod+shift+k', label: 'Write the commit message with the model', where: WORKING, group: 'Branches and commits', note: 'Only when a model is set up. What is already typed is finished, not replaced.' },
  { id: 'work.toggle', keys: 'mod+s', label: 'Stage the open file, or unstage it', where: 'An open working-tree file', group: 'Changes and diffs' },
  { id: 'work.stageall', keys: 'mod+shift+a', label: 'Stage every change', where: WORKING, group: 'Changes and diffs' },
  { id: 'work.unstageall', keys: 'mod+shift+u', label: 'Unstage everything', where: WORKING, group: 'Changes and diffs' },
  { id: 'work.discard', keys: 'mod+backspace', label: 'Discard the open file’s unstaged changes', where: 'An open working-tree file', group: 'Changes and diffs', note: 'Always asks first.' },
  { id: 'work.next', keys: 'mod+]', label: 'Open the next changed file', where: WORKING, group: 'Changes and diffs' },
  { id: 'work.previous', keys: 'mod+[', label: 'Open the previous changed file', where: WORKING, group: 'Changes and diffs' }
]

/**
 * Keys the user moved, by shortcut id. Filled from the settings whenever they
 * are read or saved; anything not in here keeps the key it ships with.
 */
const overrides = reactive<Record<string, string>>({})

export function setShortcutOverrides(map: Record<string, string> | null | undefined) {
  for (const id of Object.keys(overrides)) delete overrides[id]
  for (const [id, keys] of Object.entries(map ?? {})) {
    if (keys && SHORTCUTS.some((one) => one.id === id)) overrides[id] = keys
  }
}

/** The keys a shortcut is bound to right now: the user's, or the default. */
export function keysFor(id: string): string {
  return overrides[id] ?? SHORTCUTS.find((one) => one.id === id)?.keys ?? ''
}

/**
 * Whether a shortcut is one chord the window dispatches, and so can be moved.
 *
 * The arrows, Tab, Escape and the clicks are handled by the lists and views
 * they belong to, in ways that depend on where the caret is; a chord with a
 * modifier on it is the same keypress wherever it lands.
 */
export function isRebindable(shortcut: Shortcut): boolean {
  const keys = shortcut.keys
  if (shortcut.id === 'project.nth') return false
  if (keys.includes('/') || keys.includes('…') || keys.includes('click')) return false
  return /^(mod|alt)\+/.test(keys)
}

/** The chord an event stands for, in the spelling the list uses. */
export function eventChord(event: KeyboardEvent): string {
  return chordOf(event)
}

/** Whether two chords are the same keypress, whatever the case of their key names. */
function same(a: string, b: string) {
  return !!a && a.toLowerCase() === b.toLowerCase()
}

/**
 * Whether a keypress is the one a shortcut is bound to.
 *
 * For the keys a component handles itself rather than through the dispatcher —
 * ⌘↵ in the commit box — so that moving the key in settings moves it there too.
 */
export function matchesShortcut(event: KeyboardEvent, id: string): boolean {
  return same(chordOf(event), keysFor(id))
}

/**
 * The places that are on screen together while the working tree is: the list,
 * the commit box inside it, the working file open beside it. A key moved onto
 * one of them is live in all three at once — ⌘↵ in the box and a commit-and-
 * push moved onto it are the same press — so they are one place here.
 */
const WORKING_TOGETHER = new Set([WORKING, 'The commit message box', 'An open working-tree file'])

function together(a: string, b: string) {
  if (a === b || a === ANYWHERE || b === ANYWHERE) return true
  return WORKING_TOGETHER.has(a) && WORKING_TOGETHER.has(b)
}

/**
 * The shortcut `chord` would collide with if `row` were moved onto it, if any:
 * one bound to the same keys and live at the same time. ⌘1…9 are the project
 * tabs', all nine of them, wherever you are.
 */
export function clashFor(row: Shortcut, chord: string): Shortcut | undefined {
  if (/^mod\+[1-9]$/i.test(chord)) {
    const nth = SHORTCUTS.find((one) => one.id === 'project.nth')
    if (nth && nth.id !== row.id) return nth
  }
  return SHORTCUTS.find(
    (one) => one.id !== row.id && same(keysFor(one.id), chord) && together(one.where, row.where)
  )
}

export const SHORTCUT_GROUPS: ShortcutGroup[] = [
  'Repository',
  'Branches and commits',
  'The commit list',
  'Changes and diffs',
  'Projects and windows'
]

const isMac = typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac')

/** What each part of a chord is drawn as, so the page reads like the keyboard. */
const GLYPHS: Record<string, string> = {
  mod: isMac ? '⌘' : 'Ctrl',
  shift: isMac ? '⇧' : 'Shift',
  alt: isMac ? '⌥' : 'Alt',
  enter: '↵',
  escape: 'Esc',
  arrowup: '↑',
  arrowdown: '↓',
  arrowleft: '←',
  arrowright: '→',
  pageup: 'PgUp',
  pagedown: 'PgDn',
  backspace: isMac ? '⌫' : 'Backspace'
}

/** `mod+shift+f` as `⌘⇧F`, and `ArrowUp/ArrowDown` as `↑/↓`. */
export function keyLabel(keys: string): string {
  return keys
    .split('/')
    .map((chord) =>
      chord
        .split('+')
        .map((part) => GLYPHS[part.toLowerCase()] ?? (part.length === 1 ? part.toUpperCase() : part))
        .join(isMac ? '' : '+')
    )
    .join('/')
}

/**
 * The chord an event stands for, in the same spelling the list uses.
 *
 * `mod` is the platform's own: ⌘ on macOS, Ctrl elsewhere. Accepting either
 * would take Ctrl-F away from the caret on a Mac, where it moves it forward.
 */
function chordOf(event: KeyboardEvent): string {
  const parts: string[] = []
  const mod = isMac ? event.metaKey : event.ctrlKey
  const other = isMac ? event.ctrlKey : event.metaKey
  if (other) return ''
  if (mod) parts.push('mod')
  if (event.altKey) parts.push('alt')
  if (event.shiftKey) parts.push('shift')
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key
  parts.push(key === ' ' ? 'space' : key)
  return parts.join('+')
}

/** True while the caret is in something that takes text. */
function typing(event: KeyboardEvent): boolean {
  const target = event.target as HTMLElement | null
  if (!target) return false
  const tag = target.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || target.isContentEditable
}

/** True while a dialog, a menu or the conflict resolver sits on top. */
function covered(): boolean {
  return !!document.querySelector('.scrim, .overlay')
}

/** `project.nth` is handed the position it was asked for; nothing else takes an argument. */
type Handler = (index: number) => unknown

/**
 * Every handler bound right now, by shortcut id, in the order they were bound.
 *
 * By id rather than by chord, so that a key moved in settings is picked up by
 * handlers already bound. Two bound to the same keys — the commit list's ⌘F
 * and an open file's — go to the one bound last, which is the one on top.
 */
const handlers: { id: string; handler: Handler }[] = []
let listening = false

function handlerFor(chord: string): Handler | null {
  for (let at = handlers.length - 1; at >= 0; at--) {
    const bound = handlers[at]!
    if (same(chord, keysFor(bound.id))) return bound.handler
  }
  return null
}

function onKey(event: KeyboardEvent) {
  const chord = chordOf(event)
  if (!chord) return

  // ⌘1…⌘9 are one binding over nine keys, so they are matched before the map.
  if (/^mod\+[1-9]$/.test(chord)) {
    const nth = [...handlers].reverse().find((one) => one.id === 'project.nth')?.handler
    if (nth && !covered()) {
      event.preventDefault()
      void nth(Number(chord.slice(-1)) - 1)
    }
    return
  }

  const handler = handlerFor(chord)
  if (!handler) return
  // Every registered chord carries a modifier, so a plain letter typed into the
  // commit box never reaches here — but ⌘Z in a text box is that box's undo.
  if (typing(event) || covered()) return
  event.preventDefault()
  void handler(0)
}

/**
 * Binds handlers for as long as the component is mounted, keyed by the ids in
 * `SHORTCUTS`. An id that is not in the list is a typo, and says so.
 */
export function useShortcuts(bindings: Record<string, Handler>) {
  const bound = Object.entries(bindings).map(([id, handler]) => {
    const shortcut = SHORTCUTS.find((one) => one.id === id)
    if (!shortcut) throw new Error(`No shortcut is written down under "${id}"`)
    return { id, handler }
  })

  onMounted(() => {
    handlers.push(...bound)
    if (!listening) {
      window.addEventListener('keydown', onKey)
      listening = true
    }
  })

  onUnmounted(() => {
    for (const one of bound) {
      const at = handlers.indexOf(one)
      if (at >= 0) handlers.splice(at, 1)
    }
  })
}
