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
  | 'Files and editing'
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
  /**
   * A second chord that does the same, on this platform only and not moved by
   * settings: Alt+← for back on Windows and Linux, where every browser has it.
   */
  also?: string
  /**
   * Fires with the caret in the code editor too. Every other chord stands
   * down while something takes text, so ⌘Z there is the editor's own undo and
   * never git's.
   */
  editor?: boolean
  /**
   * The shortcut this one takes the place of over code: ⌘+ makes the code
   * larger there and the whole window larger everywhere else, so the two
   * share keys without being in each other's way.
   */
  inPlaceOf?: string
}

const isMac = typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac')

const ANYWHERE = 'Anywhere in the window'
const WORKING = 'While the working tree is shown'
const CODE = 'Over code: the editor, a diff, the search preview'

export const SHORTCUTS: Shortcut[] = [
  // Repository
  { id: 'repo.fetch', keys: 'mod+alt+f', label: 'Fetch every remote', where: ANYWHERE, group: 'Repository', note: 'Was ⌘⇧F before 0.10, which searches inside files now.' },
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
  { id: 'viewer.search', keys: 'mod+f', label: 'Find text in the file', where: 'An open file', group: 'Changes and diffs', note: 'Searches whichever view is on screen. ↑, ↓ and ↵ in the box step through the matches.', editor: true },
  // No key of their own since 0.10: ↵ and ⇧↵ in the find box step, and ⌘G is
  // going to line numbers. Kept so a key somebody moved them onto still works.
  { id: 'viewer.next', keys: '', label: 'Next match in the file', where: 'An open file', group: 'Changes and diffs', note: 'Was ⌘G before 0.10, which goes to a line now. ↵ and ⇧↵ in the find box step through the matches.' },
  { id: 'viewer.previous', keys: '', label: 'Previous match in the file', where: 'An open file', group: 'Changes and diffs' },
  { id: 'conflict.move', keys: 'ArrowUp/ArrowDown', label: 'Move to the conflict before or after this one', where: 'The conflict resolver', group: 'Changes and diffs' },
  { id: 'conflict.undecided', keys: 'Tab', label: 'Jump to the next conflict nobody has decided yet', where: 'The conflict resolver', group: 'Changes and diffs' },
  { id: 'filter.clear', keys: 'Escape', label: 'Clear the branch filter', where: 'The filter box in the sidebar', group: 'Changes and diffs' },

  // Projects and windows
  { id: 'project.open', keys: 'mod+o', label: 'Open a repository', where: ANYWHERE, group: 'Projects and windows' },
  { id: 'project.switch', keys: 'mod+p', label: 'Go to a repository', where: ANYWHERE, group: 'Projects and windows', note: 'Searches the open tabs and everything opened before them.' },
  { id: 'project.close', keys: 'mod+w', label: 'Close the file tab, or the project on the graph', where: ANYWHERE, group: 'Projects and windows', note: 'With a file open it closes that file. On the graph it closes the project tab.', editor: true },
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
  { id: 'work.next', keys: 'mod+alt+ArrowDown', label: 'Open the next changed file', where: WORKING, group: 'Changes and diffs', note: 'Was ⌘] before 0.10, which is back and forward now.', editor: true },
  { id: 'work.previous', keys: 'mod+alt+ArrowUp', label: 'Open the previous changed file', where: WORKING, group: 'Changes and diffs', editor: true },

  // Files and editing
  { id: 'search.text', keys: 'mod+shift+f', label: 'Search inside files', where: ANYWHERE, group: 'Files and editing', note: 'Starts from the text selected in the editor. Press it again in the window to switch.', editor: true },
  { id: 'search.files', keys: 'mod+shift+n', label: 'Find a file by name', where: ANYWHERE, group: 'Files and editing', note: 'Fuzzy, on the whole path. Add :42 to open at line 42.', editor: true },
  { id: 'search.symbols', keys: 'mod+alt+o', label: 'Find a class, function or method', where: ANYWHERE, group: 'Files and editing', note: 'Fuzzy on the name; capitals match capitals, so MECC finds MakeExactConnectionCommand. Opens at the definition.', editor: true },
  { id: 'files.recent', keys: 'mod+e', label: 'Recent files', where: ANYWHERE, group: 'Files and editing', editor: true },
  { id: 'file.line', keys: 'mod+g', label: 'Go to line', where: 'An open file', group: 'Files and editing', editor: true },
  { id: 'file.nextChange', keys: 'alt+ArrowDown', label: 'Next change in this file', where: 'An open file', group: 'Files and editing', editor: true },
  { id: 'file.previousChange', keys: 'alt+ArrowUp', label: 'Previous change in this file', where: 'An open file', group: 'Files and editing', editor: true },
  { id: 'file.jump', keys: 'mod+j', label: 'Go to definition, or list the usages', where: 'An open file', group: 'Files and editing', note: 'Same as ⌘-click on a name. On a call it goes to the definition; on a definition it lists who uses it.', editor: true },
  { id: 'file.save', keys: 'mod+s', label: 'Save now', where: 'The editor', group: 'Files and editing', note: 'Files save on their own as you type. Outside the editor the same key stages.' },
  { id: 'nav.back', keys: 'mod+[', label: 'Go back to where you were', where: ANYWHERE, group: 'Files and editing', note: 'The mouse’s back button does the same. A tab closed since is opened again.', also: isMac ? undefined : 'alt+ArrowLeft', editor: true },
  { id: 'nav.forward', keys: 'mod+]', label: 'Go forward again', where: ANYWHERE, group: 'Files and editing', note: 'And the mouse’s forward button.', also: isMac ? undefined : 'alt+ArrowRight', editor: true },
  { id: 'tab.next', keys: 'ctrl+Tab', label: 'Next tab', where: ANYWHERE, group: 'Files and editing', note: 'The graph counts as the first tab.', editor: true },
  { id: 'tab.previous', keys: 'ctrl+shift+Tab', label: 'Previous tab', where: ANYWHERE, group: 'Files and editing', editor: true },
  { id: 'file.mode', keys: 'mod+d', label: 'Switch an open file between Diff and File', where: 'An open file', group: 'Files and editing', note: 'Tab does the same, except while typing.', editor: true },
  { id: 'editor.fontBigger', keys: 'mod+=', label: 'Make the code larger', where: CODE, group: 'Files and editing', note: 'Every view of code at once, by a pixel. Anywhere else the same key makes the whole window larger.', editor: true, inPlaceOf: 'zoom.in' },
  { id: 'editor.fontSmaller', keys: 'mod+-', label: 'Make the code smaller', where: CODE, group: 'Files and editing', editor: true, inPlaceOf: 'zoom.out' },
  { id: 'editor.fontReset', keys: 'mod+0', label: 'Code back to its standard size', where: CODE, group: 'Files and editing', note: 'Settings → Editor has the size as a number.', editor: true, inPlaceOf: 'zoom.reset' },
  { id: 'graph.show', keys: 'Escape', label: 'Back to the graph', where: 'An open file', group: 'Files and editing', note: 'The file stays open in its tab. Closes the find box first when it is open.' }
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
  // No key by default, and free to be given one.
  if (keys === '') return true
  if (keys.includes('/') || keys.includes('…') || keys.includes('click')) return false
  return /^(mod|alt|ctrl)\+/.test(keys)
}

/** The platform-only second chord a shortcut answers to, if it has one. */
export function alsoFor(id: string): string {
  return SHORTCUTS.find((one) => one.id === id)?.also ?? ''
}

/** The chord an event stands for, in the spelling the list uses. */
export function eventChord(event: KeyboardEvent): string {
  return chordOf(event)
}

/**
 * A chord as it is compared. Off macOS, Ctrl is the platform's `mod`, so the
 * list can say `ctrl+Tab` and mean the same key everywhere.
 */
function normal(chord: string) {
  const lower = chord.toLowerCase()
  return isMac ? lower : lower.replace(/^ctrl\+/, 'mod+')
}

/** Whether two chords are the same keypress, whatever the case of their key names. */
function same(a: string, b: string) {
  return !!a && !!b && normal(a) === normal(b)
}

/** Whether a chord is one a shortcut is bound to, under either of its keys. */
function boundTo(chord: string, id: string) {
  return same(chord, keysFor(id)) || same(chord, alsoFor(id))
}

/**
 * Whether a keypress is the one a shortcut is bound to.
 *
 * For the keys a component handles itself rather than through the dispatcher —
 * ⌘↵ in the commit box — so that moving the key in settings moves it there too.
 */
export function matchesShortcut(event: KeyboardEvent, id: string): boolean {
  return boundTo(chordOf(event), id)
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
    (one) =>
      one.id !== row.id &&
      boundTo(chord, one.id) &&
      together(one.where, row.where) &&
      one.inPlaceOf !== row.id &&
      row.inPlaceOf !== one.id
  )
}

/**
 * Chords the system's menu answers before the page ever sees them: quit, hide
 * and minimise on a Mac. A shortcut moved onto one would simply never fire.
 */
export function reservedChord(chord: string, mac = isMac): boolean {
  return mac && /^mod\+(q|h|m)$/i.test(chord)
}

export const SHORTCUT_GROUPS: ShortcutGroup[] = [
  'Repository',
  'Branches and commits',
  'The commit list',
  'Changes and diffs',
  'Files and editing',
  'Projects and windows'
]

/** What each part of a chord is drawn as, so the page reads like the keyboard. */
const GLYPHS: Record<string, string> = {
  mod: isMac ? '⌘' : 'Ctrl',
  ctrl: isMac ? '⌃' : 'Ctrl',
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
 * would take Ctrl-F away from the caret on a Mac, where it moves it forward —
 * so on a Mac, Control is spelled `ctrl` and matches only a chord that says so.
 *
 * Option on a Mac types a different character: ⌥F is `ƒ`, ⌥E is a dead key.
 * Read off `event.key` alone, no ⌘⌥-letter chord could ever match. When Alt is
 * held and the key is not a plain letter or digit, the physical key it sits on
 * (`KeyF`, `Digit3`) says which one was meant. Elsewhere Alt leaves the letter
 * alone, and the letter is what the layout calls it, so that one is kept.
 */
export function chordOf(event: KeyboardEvent, mac = isMac): string {
  const parts: string[] = []
  const mod = mac ? event.metaKey : event.ctrlKey
  if (!mac && event.metaKey) return ''
  // AltGr is reported as Ctrl+Alt on Windows. A character it types is not a
  // chord: `@` on a German keyboard is not Ctrl+Alt+Q, `ą` is not Ctrl+Alt+A.
  if (!mac && event.key.length === 1 && !/^[a-z0-9 ]$/i.test(event.key) && event.getModifierState?.('AltGraph')) {
    return ''
  }
  if (mac && event.ctrlKey) parts.push('ctrl')
  if (mod) parts.push('mod')
  if (event.altKey) parts.push('alt')
  if (event.shiftKey) parts.push('shift')
  let key = event.key.length === 1 ? event.key.toLowerCase() : event.key
  const code = /^(?:Key([A-Z])|Digit([0-9]))$/.exec(event.code ?? '')
  if (event.altKey && code && !/^[a-z0-9]$/.test(key)) key = (code[1] ?? code[2]!).toLowerCase()
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

function handlerFor(chord: string): { id: string; handler: Handler } | null {
  for (let at = handlers.length - 1; at >= 0; at--) {
    const bound = handlers[at]!
    if (boundTo(chord, bound.id)) return bound
  }
  return null
}

/** True while the caret is in the code editor rather than an ordinary field. */
export function inEditor(event: Event): boolean {
  const target = event.target as HTMLElement | null
  return !!target?.closest?.('.cm-editor')
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

  const bound = handlerFor(chord)
  if (!bound) return
  // Every registered chord carries a modifier, so a plain letter typed into the
  // commit box never reaches here — but ⌘Z in a text box is that box's undo.
  // The editor lets the navigation keys through; it has handled its own by now.
  if (event.defaultPrevented || covered()) return
  if (typing(event)) {
    const allowed = inEditor(event) && SHORTCUTS.find((one) => one.id === bound.id)?.editor
    if (!allowed) return
  }
  event.preventDefault()
  void bound.handler(0)
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
