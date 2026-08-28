/**
 * What Tab offers at the log's prompt.
 *
 * A terminal completes git commands because git ships a completion script;
 * there is no shell here to load one, so the same job is done from what the
 * window already knows — the refs and the changed files are in the store, and
 * the subcommand list is the one below.
 *
 * Kept apart from the component so the rules can be tested against a line of
 * text rather than through a keystroke.
 */

/** The subcommands worth offering. Not all of git — the ones people type. */
const COMMANDS = [
  'add',
  'am',
  'apply',
  'bisect',
  'blame',
  'branch',
  'checkout',
  'cherry-pick',
  'clean',
  'clone',
  'commit',
  'config',
  'describe',
  'diff',
  'fetch',
  'grep',
  'init',
  'log',
  'merge',
  'mv',
  'pull',
  'push',
  'rebase',
  'reflog',
  'remote',
  'reset',
  'restore',
  'revert',
  'rm',
  'shortlog',
  'show',
  'stash',
  'status',
  'submodule',
  'switch',
  'tag',
  'worktree'
]

/** Subcommands whose arguments are usually a branch, a tag or a commit. */
const TAKES_REF = new Set([
  'branch',
  'checkout',
  'cherry-pick',
  'describe',
  'diff',
  'log',
  'merge',
  'rebase',
  'reset',
  'revert',
  'show',
  'switch',
  'tag'
])

/** Subcommands whose arguments are usually paths. */
const TAKES_PATH = new Set([
  'add',
  'blame',
  'checkout',
  'clean',
  'diff',
  'grep',
  'mv',
  'restore',
  'rm',
  'show'
])

/** The second word, where the subcommand has its own set. */
const SUBCOMMANDS: Record<string, string[]> = {
  stash: ['push', 'pop', 'list', 'show', 'apply', 'drop', 'clear', 'branch'],
  remote: ['add', 'remove', 'rename', 'set-url', 'show', 'prune', '-v'],
  submodule: ['status', 'update', 'init', 'add', 'sync', 'deinit', 'foreach'],
  worktree: ['list', 'add', 'remove', 'prune', 'move', 'lock', 'unlock'],
  bisect: ['start', 'good', 'bad', 'skip', 'reset', 'log']
}

export interface CompletionSource {
  /** Local branch names. */
  branches: string[]
  /** Remote-tracking names, already `remote/branch`. */
  remotes: string[]
  tags: string[]
  /** Paths git currently reports as changed. */
  files: string[]
}

export interface Completion {
  /** The word being completed, which is what a match replaces. */
  word: string
  matches: string[]
}

/** The words of a line, and whether the last one is still being typed. */
function tokenize(line: string) {
  const words = line.split(/\s+/).filter((word) => word !== '')
  const typing = line !== '' && !/\s$/.test(line)
  return { words, typing }
}

/** What Tab would offer for this line. */
export function completionsFor(line: string, source: CompletionSource): Completion {
  const { words, typing } = tokenize(line)
  const word = typing ? (words[words.length - 1] ?? '') : ''
  const before = typing ? words.slice(0, -1) : words
  // `git log` typed out of habit is dropped when the line runs, so it is not
  // a word for the purpose of working out what comes next either.
  if (before[0] === 'git') before.shift()

  const pool = poolFor(before, word, source)
  const matches = pool.filter((one) => one.startsWith(word)).sort()
  // An exact and only match is not a completion, it is the word already there.
  if (matches.length === 1 && matches[0] === word) return { word, matches: [] }
  return { word, matches }
}

function poolFor(before: string[], word: string, source: CompletionSource): string[] {
  if (!before.length) return COMMANDS

  const command = before[0] ?? ''
  // A flag completes from nothing: guessing which of git's several hundred
  // options were meant would offer more wrong answers than right ones.
  if (word.startsWith('-')) return []

  const own = SUBCOMMANDS[command]
  if (own && before.length === 1) return own

  const pool: string[] = []
  if (TAKES_REF.has(command)) {
    pool.push(...source.branches, ...source.remotes, ...source.tags)
  }
  if (TAKES_PATH.has(command)) pool.push(...source.files)
  // Anything unrecognised gets the lot rather than nothing: half the value of
  // completion is not having to type a branch name, whatever the verb was.
  // The subcommand position has already returned above, so a command with its
  // own second word still reaches this for its third.
  if (!pool.length) {
    pool.push(...source.branches, ...source.remotes, ...source.tags, ...source.files)
  }
  return [...new Set(pool)]
}

/** The longest start every match shares, which is how far Tab can fill in. */
export function commonPrefix(words: string[]): string {
  if (!words.length) return ''
  let prefix = words[0] ?? ''
  for (const word of words.slice(1)) {
    while (prefix && !word.startsWith(prefix)) prefix = prefix.slice(0, -1)
  }
  return prefix
}

/** The line with its last word replaced by `word`. */
export function replaceWord(line: string, word: string): string {
  const { typing } = tokenize(line)
  if (!typing) return line + word
  return line.replace(/\S+$/, word)
}
