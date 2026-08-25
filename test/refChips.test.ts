import { describe, expect, it } from 'vitest'
import { lineChips, refChips } from '../app/composables/useRefChips'
import type { GraphRow, RefLabel } from '../app/composables/useGit'

/**
 * A row as the branch column sees it: the refs on it, and enough of the line it
 * sits on to say where that line goes next.
 *
 * `to` is the colours leaving the bottom of the row, which is all the ghost
 * needs of the drawing — a line arrives at the top of the next row in the same
 * colour it left this one in, and a colour that does not arrive belongs to a
 * line that starts there. Every line crossing the row is in it, not only the
 * one the row's own commit sits on; it defaults to just that one, so a case
 * with a second line running past has to say so.
 */
function row(
  oid: string,
  color: number,
  labels: RefLabel[] = [],
  to: number[] = [color]
): GraphRow {
  return {
    oid,
    short: oid.slice(0, 7),
    summary: oid,
    author: 'Tester',
    email: 'tester@example.com',
    time: 0,
    parents: [],
    lane: 0,
    color,
    width: 1,
    segments: to.map((c) => ({ x1: 0, y1: 1, x2: 0, y2: 2, color: c })),
    labels,
    unpushed: false
  }
}

const local = (name: string, head = false): RefLabel => ({ kind: 'local', name, head })
const remote = (name: string): RefLabel => ({ kind: 'remote', name, head: false })
const tag = (name: string): RefLabel => ({ kind: 'tag', name, head: false })
const detached = (): RefLabel => ({ kind: 'head', name: 'HEAD', head: true })

describe('refChips', () => {
  it('folds a remote into the local branch it tracks', () => {
    const chips = refChips(row('a', 0, [local('main'), remote('origin/main')]))

    expect(chips).toHaveLength(1)
    expect(chips[0]!.name).toBe('main')
    expect(chips[0]!.remotes).toEqual(['origin/main'])
  })

  it('splits a tracking name on the first slash only', () => {
    const chips = refChips(
      row('a', 0, [local('feature/x'), remote('origin/feature/x')])
    )

    expect(chips).toHaveLength(1)
    expect(chips[0]!.remotes).toEqual(['origin/feature/x'])
  })

  it('keeps a remote that no local branch tracks', () => {
    const chips = refChips(row('a', 0, [local('main'), remote('origin/other')]))

    expect(chips.map((c) => c.name)).toEqual(['main', 'origin/other'])
  })

  it('puts the checked-out branch first', () => {
    const chips = refChips(row('a', 0, [local('alpha'), local('beta', true)]))

    expect(chips[0]!.name).toBe('beta')
  })

  // Only the first chip is drawn; the rest fold away behind a counter. So a
  // detached HEAD sorting anywhere but first is a detached HEAD you cannot see
  // on any commit that a branch or a tag also points at — which is most of the
  // commits anyone checks out directly.
  it('puts a detached HEAD ahead of every branch and tag on the same commit', () => {
    const chips = refChips(row('a', 0, [local('main'), tag('v1.0'), detached()]))

    expect(chips[0]!.name).toBe('HEAD')
    expect(chips[0]!.kind).toBe('head')
  })

  it('leaves tags last', () => {
    const chips = refChips(row('a', 0, [tag('v1.0'), remote('origin/x'), local('main')]))

    expect(chips.map((c) => c.kind)).toEqual(['local', 'remote', 'tag'])
  })
})

describe('lineChips', () => {
  it('carries a branch down the line it names', () => {
    const owners = lineChips([
      row('tip', 0, [local('main')]),
      row('mid', 0),
      row('old', 0)
    ])

    expect(owners.map((c) => c?.name)).toEqual(['main', 'main', 'main'])
  })

  it('says nothing above the first tip', () => {
    // The trunk's column is reserved before its tip is reached, so the rows
    // above it are on other lines and this one has no name yet.
    const owners = lineChips([row('newer', 1), row('tip', 0, [local('main')])])

    expect(owners[0]).toBeNull()
    expect(owners[1]!.name).toBe('main')
  })

  it('keeps two lines apart by their colour', () => {
    // Both lines run the whole way down, so every row in between carries both
    // colours out of its bottom edge — the one it sits on and the one merely
    // passing it by.
    const owners = lineChips([
      row('a', 0, [local('main')], [0]),
      row('b', 1, [local('feature')], [1, 0]),
      row('c', 1, [], [1, 0]),
      row('d', 0, [], [0])
    ])

    expect(owners.map((c) => c?.name)).toEqual(['main', 'feature', 'feature', 'main'])
  })

  // A colour is handed out again once the line wearing it has ended. A line
  // that no ref points at — what a merge brought in from a branch since
  // deleted — must not inherit the name of whatever wore its colour last and
  // state it as fact.
  it('drops the name when a colour is reused by a new line', () => {
    const owners = lineChips([
      // `feature` ends here: nothing in its colour leaves the bottom.
      row('tip', 1, [local('feature')], []),
      row('other', 0, [local('main')]),
      // Colour 1 again, but nothing was running into it, so it is a new line.
      row('orphan', 1)
    ])

    expect(owners[0]!.name).toBe('feature')
    expect(owners[2]).toBeNull()
  })

  it('takes over from the previous holder when a line names itself', () => {
    const owners = lineChips([
      row('tip', 1, [local('feature')], []),
      row('other', 0, [local('main')]),
      row('newtip', 1, [local('later')]),
      row('below', 1)
    ])

    expect(owners.map((c) => c?.name)).toEqual(['feature', 'main', 'later', 'later'])
  })
})
