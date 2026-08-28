import { SHORTCUTS } from './useShortcuts'

/**
 * One thing worth knowing about the app.
 *
 * A tip is a sentence first and a key second: the point is the thing you can
 * do, and the key is how to do it quickly if there is one. The old list was
 * three shortcut labels lifted straight out of settings, which reads as a
 * keyboard reference rather than as a hint — "Flip between the patch and the
 * whole file" means nothing to somebody who has not already found it.
 */
export interface Tip {
  /** Stable, so a tip can be pointed at from a test or a link. */
  id: string
  /** The sentence, written for somebody who has never seen the feature. */
  text: string
  /** The chord, in the spelling `keyLabel` reads, when a key does it. */
  keys?: string
}

/** The chord a shortcut is bound to, so a tip and settings cannot disagree. */
function chord(id: string): string | undefined {
  return SHORTCUTS.find((one) => one.id === id)?.keys
}

/**
 * Everything the page can say, in no particular order.
 *
 * Long enough that the three on screen are rarely the three you saw last time,
 * and every one of them is about something the window really does.
 */
export const TIPS: Tip[] = [
  {
    id: 'switch',
    text: 'Type part of a name to jump to any repository you have opened, tab or not.',
    keys: chord('project.switch')
  },
  {
    id: 'undo',
    text: 'Undo works on git itself: the merge, the reset, the branch you just deleted.',
    keys: chord('history.undo')
  },
  {
    id: 'whole-file',
    text: 'Reading a diff and want the rest of the file? Tab shows the whole file with the changes still marked.',
    keys: chord('diff.mode')
  },
  {
    id: 'search',
    text: 'Search the commit list by message, author or hash — and step through the matches.',
    keys: chord('graph.search')
  },
  {
    id: 'pull',
    text: 'Pull puts your uncommitted work aside first and hands it back afterwards.',
    keys: chord('repo.pull')
  },
  {
    id: 'stash',
    text: 'Stash everything, staged work included, without touching the command line.',
    keys: chord('stash.push')
  },
  {
    id: 'branch',
    text: 'Start a branch from wherever you are standing.',
    keys: chord('branch.create')
  },
  // A gesture belongs in the sentence and nowhere else: the chip is for a chord
  // you press, and "Shift-click … Shift+click" said the same thing twice in a
  // row, which reads as a mistake rather than as a key.
  {
    id: 'range',
    text: 'Shift-click marks a run of commits in the list; hold the modifier to add one on its own.'
  },
  {
    id: 'chip',
    text: 'Double-click a branch chip in the commit list to check that branch out.'
  },
  {
    id: 'nth',
    text: 'Project tabs answer to their number, left to right.',
    keys: chord('project.nth')
  },
  {
    id: 'fetch',
    text: 'Fetch every remote at once — or let the app do it on a timer, in settings.',
    keys: chord('repo.fetch')
  },
  {
    id: 'zoom',
    text: 'The whole window scales, and it opens at the size and zoom you left it.',
    keys: chord('zoom.in')
  },
  {
    id: 'reorder',
    text: 'Drag a tab sideways to reorder your projects. The order is remembered.'
  },
  {
    id: 'panes',
    text: 'Drag the edge of the sidebar or the panel to resize it; double-click the edge to snap it back.'
  },
  {
    id: 'blame',
    text: 'Right-click the line numbers in an open file to turn blame on beside them.'
  },
  {
    id: 'stash-list',
    text: 'Stashes sit in the sidebar. Double-click one to put it back and still keep it.'
  },
  {
    id: 'submodule',
    text: 'A submodule opens in the same tab, and the trail in the toolbar is the way back out.'
  },
  {
    id: 'reviews',
    text: 'Pull requests from your forge open in the window — the conversation, the checks and the diff.'
  },
  {
    id: 'ai-message',
    text: 'Point the app at a model in settings and it will write a commit message from what you staged.'
  },
  {
    id: 'profiles',
    text: 'Profiles keep accounts apart: name, email, signing key and its own set of tabs.'
  },
  {
    id: 'conflicts',
    text: 'A failed merge opens the resolver, where Tab hops to the next conflict nobody has decided.'
  },
  {
    id: 'rebase',
    text: 'Rebasing is a plan you can see: drag commits into order, squash or reword, then run it.'
  },
  {
    id: 'moved',
    text: 'A file that moved is shown as one file that moved, not a delete and an add.'
  },
  {
    id: 'sidebar-lists',
    text: 'Tags, remotes, worktrees and submodules each keep their own list in the sidebar.'
  },
  {
    id: 'themes',
    text: 'Settings has a shelf of themes, light and dark, applied before the next repaint.'
  },
  {
    id: 'log',
    text: 'The bar along the bottom logs every git command the app ran, so nothing happens off-screen.'
  }
]

/**
 * `count` tips at random, never the same one twice.
 *
 * Shuffled with a copy rather than picked with `Math.random` per slot, which
 * is what made the old three repeat one another on a short list.
 */
export function pickTips(count: number, from: Tip[] = TIPS): Tip[] {
  const pool = [...from]
  for (let at = pool.length - 1; at > 0; at -= 1) {
    const swap = Math.floor(Math.random() * (at + 1))
    ;[pool[at], pool[swap]] = [pool[swap]!, pool[at]!]
  }
  return pool.slice(0, Math.min(count, pool.length))
}
