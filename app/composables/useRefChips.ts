import type { GraphRow } from './useGit'

/**
 * What the branch column draws, worked out from the refs the backend reports.
 *
 * Kept apart from the component because it is all decisions and no drawing:
 * which of a commit's refs is worth the one chip there is room for, and what to
 * call the line a commit that is nobody's tip belongs to. Both are easy to get
 * subtly wrong and impossible to notice — a chip that quietly sorts last is
 * simply a chip you never see — so they are written where a test can reach
 * them.
 */

/** One label in the branch column, after folding and ordering. */
export interface RefChip {
  key: string
  name: string
  kind: string
  head: boolean
  /** Remote branches for this same name sitting on this same commit. */
  remotes: string[]
}

/**
 * Turns a commit's refs into the chips to draw.
 *
 * A branch that has not moved since its last push carries both `main` and
 * `origin/main` on one commit, and drawing two chips says nothing the one chip
 * cannot: the name is the same, and the only fact worth adding is that the
 * remote is here too. So a local branch absorbs the remotes tracking its name
 * and wears a cloud beside its screen. Remotes with no local of that name, and
 * tags, keep chips of their own.
 *
 * Order puts a detached HEAD first, then the checked-out branch, then the other
 * locals, then remote-only branches, then tags — which is also the order of how
 * likely you are to be looking for it. HEAD leads because only one chip is
 * drawn: check an old commit out that three branches also point at and the one
 * marker saying you are standing here would otherwise be the one folded away
 * behind the counter.
 */
export function refChips(row: GraphRow): RefChip[] {
  const locals = row.labels.filter((l) => l.kind === 'local')
  const remotes = row.labels.filter((l) => l.kind === 'remote')
  const rest = row.labels.filter((l) => l.kind !== 'local' && l.kind !== 'remote')

  const absorbed = new Set<string>()
  const fromLocals = locals.map((local) => {
    // `origin/feature/x` tracks `feature/x`: split on the first slash only.
    const mine = remotes.filter((r) => r.name.slice(r.name.indexOf('/') + 1) === local.name)
    for (const remote of mine) absorbed.add(remote.name)
    return {
      key: `local:${local.name}`,
      name: local.name,
      kind: 'local',
      head: local.head,
      remotes: mine.map((r) => r.name)
    }
  })

  const orphanRemotes = remotes
    .filter((r) => !absorbed.has(r.name))
    .map((r) => ({ key: `remote:${r.name}`, name: r.name, kind: 'remote', head: false, remotes: [] }))

  const others = rest.map((l) => ({
    key: `${l.kind}:${l.name}`,
    name: l.name,
    kind: l.kind,
    head: l.head,
    remotes: [] as string[]
  }))

  return [
    ...others.filter((c) => c.kind === 'head'),
    ...fromLocals.filter((c) => c.head),
    ...fromLocals.filter((c) => !c.head),
    ...orphanRemotes,
    ...others.filter((c) => c.kind !== 'head')
  ]
}

/** The refs a commit carries beyond the one on show. */
export function hiddenRefs(row: GraphRow): RefChip[] {
  return refChips(row).slice(1)
}

/**
 * The branch each row's line belongs to, whether or not the row is a tip of it.
 *
 * The strip on the left is empty for all but a handful of rows, and the
 * question it leaves unanswered is the one asked most often: this commit here,
 * what branch is it on? Answering it meant selecting the commit, or counting
 * lanes back up to the nearest chip.
 *
 * A line keeps its colour for as long as it runs, and no two live lines are
 * given the same one, so the nearest ref above a row on the same colour is the
 * ref that row's line leads to. Walking newest first, that is simply the last
 * chip seen on this colour. Rows above every tip get nothing, which is right:
 * there is no branch there yet.
 *
 * A colour is handed out again once the line wearing it has ended, though, and
 * a line that no ref points at — everything a merge brought in from a branch
 * since deleted — would otherwise inherit the name of whatever wore its colour
 * last and state it as fact. So a colour that was not already running into a
 * row starts a line there, and the name is dropped rather than carried on.
 */
export function lineChips(rows: GraphRow[]): (RefChip | null)[] {
  const latest = new Map<number, RefChip>()
  let arriving = new Set<number>()

  return rows.map((row) => {
    if (!arriving.has(row.color)) latest.delete(row.color)
    const own = row.labels.length ? refChips(row)[0] : undefined
    if (own) latest.set(row.color, own)
    const chip = own ?? latest.get(row.color) ?? null
    // What leaves the bottom of this row arrives at the top of the next.
    const leaving = new Set(
      row.segments.filter((segment) => segment.y2 === 2).map((segment) => segment.color)
    )
    // A line a merge sends out to a parent nothing was waiting for starts
    // here, in a colour that was not running into this row. Whatever last
    // wore that colour has ended, and its name must not be carried on to a
    // line it has nothing to do with: a deleted branch merged in used to be
    // ghosted with the name of an older line that happened to share its
    // shade.
    for (const color of leaving) {
      if (color !== row.color && !arriving.has(color)) latest.delete(color)
    }
    arriving = leaving
    return chip
  })
}

/**
 * The tooltip on a ghosted name.
 *
 * Worded as "on" rather than as the name alone, because unlike a real chip this
 * is not a ref sitting on this commit — it is the branch the commit is part of,
 * and the difference matters the moment you go to check it out.
 */
export function ghostTitle(chip: RefChip): string {
  if (chip.head) return `On ${chip.name} — the branch you are on`
  return `On ${chip.name}\nDouble-click to check it out`
}
