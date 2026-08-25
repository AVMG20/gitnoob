import { onMounted, onUnmounted } from 'vue'

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

  // Changes and diffs
  { id: 'diff.mode', keys: 'Tab', label: 'Switch between unified and side-by-side', where: 'A diff', group: 'Changes and diffs' },
  { id: 'filter.clear', keys: 'Escape', label: 'Clear the branch filter', where: 'The filter box in the sidebar', group: 'Changes and diffs' },

  // Projects and windows
  { id: 'project.open', keys: 'mod+o', label: 'Open a repository', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'project.close', keys: 'mod+w', label: 'Close the current project tab', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'project.next', keys: 'mod+alt+ArrowRight', label: 'Next project tab', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'project.previous', keys: 'mod+alt+ArrowLeft', label: 'Previous project tab', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'project.nth', keys: 'mod+1…9', label: 'Jump to a project tab by position', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'dialog.close', keys: 'Escape', label: 'Close the dialog on top', where: 'Any dialog', group: 'Projects and windows' }
]

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
  pagedown: 'PgDn'
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

const handlers = new Map<string, Handler>()
let listening = false

function onKey(event: KeyboardEvent) {
  const chord = chordOf(event)
  if (!chord) return

  // ⌘1…⌘9 are one binding over nine keys, so they are matched before the map.
  if (/^mod\+[1-9]$/.test(chord)) {
    const nth = handlers.get('project.nth')
    if (nth && !covered()) {
      event.preventDefault()
      void nth(Number(chord.slice(-1)) - 1)
    }
    return
  }

  const handler = handlers.get(chord)
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
  const chords = Object.entries(bindings).map(([id, handler]) => {
    const shortcut = SHORTCUTS.find((one) => one.id === id)
    if (!shortcut) throw new Error(`No shortcut is written down under "${id}"`)
    // The nine project keys register under their id; everything else under the
    // chord the dispatcher builds from the event.
    return [id === 'project.nth' ? id : shortcut.keys, handler] as const
  })

  onMounted(() => {
    for (const [key, handler] of chords) handlers.set(key, handler)
    if (!listening) {
      window.addEventListener('keydown', onKey)
      listening = true
    }
  })

  onUnmounted(() => {
    for (const [key, handler] of chords) {
      if (handlers.get(key) === handler) handlers.delete(key)
    }
  })
}
