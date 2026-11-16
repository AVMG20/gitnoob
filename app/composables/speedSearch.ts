/**
 * Speed search: typing over a focused list to jump to a row, as in an IDE.
 *
 * Matched on the name as shown, ignoring case: a name that starts with the
 * query is best, then one with a word that starts with it (after `-`, `_`,
 * `.` or a space, or a camel hump, so `bb` finds BigBridge and `acl` finds
 * AccountLoggedIn), then one that has it anywhere. Only what the list shows
 * is looked at; nothing is asked of the backend.
 */

/** Where a speed search matched a name: how well, and which characters. */
export interface SpeedMatch {
  /** 3 at the start, 2 at word starts (camel humps too), 1 anywhere. */
  tier: number
  indices: number[]
}

/** The starts of the words in a name: after `-`, `_`, `.`, a space, and camel humps. */
function wordStarts(name: string) {
  const out: number[] = []
  for (let at = 0; at < name.length; at++) {
    const c = name[at]!
    const before = at ? name[at - 1]! : ''
    const boundary =
      at === 0 ||
      /[-_. /]/.test(before) ||
      (/[A-Z]/.test(c) && /[a-z0-9]/.test(before)) ||
      (/[0-9]/.test(c) && !/[0-9]/.test(before))
    if (boundary && !/[-_. /]/.test(c)) out.push(at)
  }
  return out
}

/** The query as pieces each starting a word, in order: `acl` in AccountLoggedIn. */
function humps(name: string, query: string): number[] | null {
  const lower = name.toLowerCase()
  const starts = wordStarts(name)
  const walk = (from: number, startAt: number): number[] | null => {
    if (from === query.length) return []
    for (let s = startAt; s < starts.length; s++) {
      const at = starts[s]!
      let length = 0
      while (from + length < query.length && lower[at + length] === query[from + length]) {
        length++
        const rest = walk(from + length, s + 1)
        if (rest) return [...Array.from({ length }, (_, k) => at + k), ...rest]
      }
    }
    return null
  }
  return walk(0, 0)
}

export function speedMatch(name: string, typed: string): SpeedMatch | null {
  const query = typed.toLowerCase()
  if (!query) return null
  const lower = name.toLowerCase()
  const run = (at: number) => Array.from({ length: query.length }, (_, k) => at + k)
  if (lower.startsWith(query)) return { tier: 3, indices: run(0) }
  const starts = wordStarts(name)
  const atStart = starts.find((at) => lower.startsWith(query, at))
  if (atStart !== undefined) return { tier: 2, indices: run(atStart) }
  const hump = humps(name, query)
  if (hump) return { tier: 2, indices: hump }
  const at = lower.indexOf(query)
  return at >= 0 ? { tier: 1, indices: run(at) } : null
}

