<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  Archive,
  ArrowDownToLine,
  Check,
  ChevronDown,
  ChevronUp,
  Cloud,
  Copy,
  FileText,
  GitBranch,
  GitBranchPlus,
  GitCommitHorizontal,
  GitMerge,
  MonitorDot,
  Search,
  Settings2,
  Tag,
  Trash2,
  Undo2,
  X
} from 'lucide-vue-next'
import {
  WIP,
  copyText,
  fullTime,
  highlight,
  isRunning,
  laneColor,
  laneTint,
  relativeTime,
  rowMatches,
  useGit,
  type GraphRow,
  type ResetMode,
  type Segment
} from '~/composables/useGit'
import {
  ghostTitle,
  hiddenRefs,
  lineChips,
  refChips,
  type RefChip
} from '~/composables/useRefChips'
import { avatarFor, initials, tint } from '~/composables/useAvatars'
import { signatureLook, signatureTitle } from '~/composables/useSigning'
import { useRebase } from '~/composables/useRebase'
import { useContextMenu } from '~/composables/useContextMenu'
import { useDragDrop } from '~/composables/useDragDrop'
import { keyLabel, useShortcuts } from '~/composables/useShortcuts'
import { useColumns, type ColumnId } from '~/composables/useColumns'
import { useConfig } from '~/composables/useConfig'

const git = useGit()
const store = git.store
const rebase = useRebase()
const menu = useContextMenu()
const drag = useDragDrop()
const config = useConfig()

/** Whether to draw author pictures at all, which Settings can turn off. */
const avatars = computed(() => config.settings.value?.show_avatars !== false)

const ROW = 27
/**
 * How far apart two parallel lines sit.
 *
 * Wide enough that a node, and the ring an unpushed one wears, clear the lane
 * next door: lanes closer together than the things drawn on them collapse into
 * a single smudged column, and a graph that cannot show two lines apart cannot
 * show what happened.
 */
const LANE = 24
const OVERSCAN = 12
/**
 * How many lanes the picture can hold. Anything past the last one is drawn in
 * the last one, so a busy repository used to end with a handful of unrelated
 * branches sharing a single column. Kept in step with `DRAWN_LANES` on the
 * other side, which drops the segments no lane can show. The palette is
 * shorter than this and repeats, which is the intent: a lane out here wearing
 * a shade already on screen still reads better than no lane at all.
 */
const MAX_LANES = 28
/** Half the width of a commit node — the author's face sits inside it. */
const NODE = 8
/** Room at the left edge for the first lane's node and its ring. */
const PAD = 6
/** How tightly a line turns where it changes lane. */
const ELBOW = 7
/** Half the width of the dot drawn where lines join or part. */
const JOINT = 5

const viewport = ref<HTMLElement | null>(null)
const searchBox = ref<HTMLInputElement | null>(null)
const searchOpen = ref(false)
const scrollTop = ref(0)
const height = ref(600)
const hit = ref(0)
/** The commit a hard reset is being confirmed against; the only one that asks. */
const resetTarget = ref<string | null>(null)
const tagTarget = ref<GraphRow | null>(null)
const branchTarget = ref<GraphRow | null>(null)
/** The stash whose drop is being confirmed, by index; null when none is. */
const dropping = ref<number | null>(null)
/** The commits whose fold is being composed, or null when none is. */
const squashing = ref<string[] | null>(null)

/**
 * Commits picked out with shift or ctrl, for the operations that take more than
 * one. Kept apart from `store.selected`, which is what the right panel shows:
 * one row is always the subject even when several are marked.
 */
const marked = ref<string[]>([])
const markedSet = computed(() => new Set(marked.value))
/** Where a shift-click measures its range from. */
const anchor = ref<string | null>(null)

const total = computed(() => store.rows.length)
const lanes = computed(() => {
  // Walked rather than spread into `Math.max`: a history long enough to be
  // worth virtualizing is also long enough to overflow the argument list.
  let widest = 2
  for (const row of store.rows) if (row.width > widest) widest = row.width
  return Math.min(MAX_LANES, widest)
})
const graphWidth = computed(() => lanes.value * LANE + 8 + PAD)

// --- columns
//
// The graph is the one column with a width of its own: it is as wide as the
// lanes in view need, until the user says otherwise. The rest carry a number
// from the start, and the message column takes whatever is left.
const cols = useColumns()
const NATURAL: Record<ColumnId, () => number> = {
  refs: () => 124,
  graph: () => graphWidth.value,
  author: () => 130,
  date: () => 88
}
const width = (id: ColumnId) => cols.widthOf(id, NATURAL[id]())
const box = (id: ColumnId) => ({ width: `${width(id)}px` })

/**
 * Drags one edge. `sign` is which way the pointer has to move to make the
 * column wider: the columns to the left of the message grow rightwards, the two
 * to its right grow leftwards, so both edges push against the message column
 * rather than against the window.
 */
const resizing = ref<ColumnId | null>(null)

function startResize(event: PointerEvent, id: ColumnId, sign: 1 | -1) {
  event.preventDefault()
  event.stopPropagation()
  const from = event.clientX
  const start = width(id)
  resizing.value = id

  const move = (moved: PointerEvent) => cols.setWidth(id, start + sign * (moved.clientX - from))
  const stop = () => {
    resizing.value = null
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', stop)
    document.body.style.cursor = ''
    document.body.style.userSelect = ''
  }

  document.body.style.cursor = 'col-resize'
  document.body.style.userSelect = 'none'
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', stop)
}

/** Right-clicking the headings says which columns there are, and drops them. */
function columnMenu(event: MouseEvent) {
  menu.show(
    event,
    [
      ...cols.columns.map((column) => ({
        label: column.label,
        icon: cols.state.shown[column.id] ? Check : undefined,
        action: () => cols.toggle(column.id)
      })),
      { label: '', separator: true },
      { label: 'Reset the widths', icon: Undo2, action: () => cols.resetWidths() }
    ],
    'Columns'
  )
}

const matches = computed(() =>
  store.query.trim() ? store.rows.filter((row) => rowMatches(row, store.query)) : []
)
const matchIds = computed(() => new Set(matches.value.map((row) => row.oid)))

/**
 * How far the commits sit below the top of the scroller.
 *
 * The working tree is the first row of the list rather than a strip pinned
 * above it: it scrolls away like anything else, and being an ordinary row is
 * what keeps its dot in line with the dots under it. Everything that turns a
 * scroll position into a row index has to step over it.
 */
const ABOVE = ROW

/** Where the list is scrolled to, counted from the first commit. */
const listTop = computed(() => Math.max(0, scrollTop.value - ABOVE))

const first = computed(() => Math.max(0, Math.floor(listTop.value / ROW) - OVERSCAN))
const last = computed(() =>
  Math.min(total.value, Math.ceil((listTop.value + height.value) / ROW) + OVERSCAN)
)
/**
 * Everything a row needs in order to draw, worked out once per commit.
 *
 * The template used to ask for the same things over and over — the face four
 * times a row, the chips three, the ghost twice — and a scroll event re-renders
 * every row in the window, so one flick of the wheel ran the lot thousands of
 * times. None of it depends on where the list is scrolled to or on anything
 * that changes while it sits there, so it is worked out once and kept.
 */
interface RowMemo {
  /** The one ref chip there is room to draw, of however many the commit has. */
  chip: RefChip | undefined
  /** The rest, which live behind the counter beside it. */
  hidden: RefChip[]
  /**
   * The branch a commit that is nobody's tip belongs to, ghosted on hover. A
   * row that is a tip has a chip of its own and gets nothing here: the ghost
   * answers "what is this commit on?", which that row already answers.
   */
  ghost: RefChip | null
  /**
   * Whether two lines become one here — a merge, and only a merge. It is the
   * busiest node in the picture, so it is drawn as a plain dot rather than a
   * face: the junction reads as a junction, and the lines meeting there are not
   * hidden behind a picture. A branch point is left as an ordinary commit even
   * though it is a junction of a kind; the line leaving it says so already, and
   * there is one for every branch ever made from the trunk.
   */
  junction: boolean
  letters: string
  tint: string
  when: string
}

const memos = new Map<string, RowMemo>()

// A refresh rebuilds the rows wholesale, and what a commit is part of can
// change with them — a branch deleted moves the ghost names — so the memos go
// with them rather than being reconciled.
watch(() => store.rows, () => {
  memos.clear()
  cancelUnfold()
})

function memoOf(row: GraphRow, index: number): RowMemo {
  let memo = memos.get(row.oid)
  if (!memo) {
    const chips = refChips(row)
    memo = {
      chip: chips[0],
      hidden: chips.slice(1),
      ghost: row.labels.length ? null : (lineOwners.value[index] ?? null),
      junction: row.parents.length > 1,
      letters: initials(row.author, row.email),
      tint: tint(row.email),
      when: relativeTime(row.time)
    }
    memos.set(row.oid, memo)
  }
  return memo
}

const window_ = computed(() =>
  store.rows.slice(first.value, last.value).map((row, i) => {
    const index = first.value + i
    const memo = memoOf(row, index)
    // Read here rather than in the memo: the answer arrives after the row has
    // already been drawn once, and reading it inside the computed is what makes
    // the row draw again when it does. Three states, not two — for the moment
    // between asking and knowing the node is the colour alone, so nothing
    // flickers from letters into a face.
    const found = avatars.value ? avatarFor(row.email) : null
    return {
      row,
      index,
      top: index * ROW,
      chip: memo.chip,
      hidden: memo.hidden,
      refs: memo.chip ? [memo.chip, ...memo.hidden] : [],
      ghost: memo.ghost,
      junction: memo.junction,
      picture: found ?? null,
      letters: found === null ? memo.letters : '',
      tint: memo.tint,
      when: memo.when,
      whenFull: fullTime(row.time),
      parts: highlight(row.summary, store.query)
    }
  })
)

const dirty = computed(
  () => (store.status?.staged.length ?? 0) + (store.status?.unstaged.length ?? 0)
)
const conflicts = computed(() => store.status?.conflicted.length ?? 0)

/**
 * The commit HEAD is on, and where in the list it sits.
 *
 * The working tree hangs off this commit, not off the newest one: those were
 * the same row only while you happened to be standing on the tip of the newest
 * branch. Check something older out and the WIP node was left drawn on a lane
 * belonging to a line it has nothing to do with.
 */
const headIndex = computed(() =>
  store.rows.findIndex((row) => row.labels.some((label) => label.head))
)
const headRow = computed(() => (headIndex.value >= 0 ? store.rows[headIndex.value] : null))
const headLane = computed(() => headRow.value?.lane ?? store.rows[0]?.lane ?? 0)
const headColor = computed(() => headRow.value?.color ?? store.rows[0]?.color ?? 0)

/**
 * How far down this row the dotted line from the working tree runs.
 *
 * It leaves the WIP node pinned above the list and ends at the commit HEAD is
 * on, so however far you have scrolled there is a thread back to where you are
 * standing — which is the one thing a graph of a dozen branches otherwise makes
 * you hunt for. Nothing is drawn past that commit: below it the line is history
 * the graph is already drawing properly.
 */
function headTrace(index: number) {
  if (headIndex.value < 0 || index > headIndex.value) return 0
  return index === headIndex.value ? ROW / 2 : ROW
}

/** True while the commit HEAD is on is scrolled out of the window. */
const headOffScreen = computed(() => {
  if (headIndex.value < 0) return false
  const top = headIndex.value * ROW
  return top + ROW < listTop.value || top > listTop.value + height.value
})
/** Which way to send the eye — and the scroll — to reach it. */
const headBelow = computed(() => headIndex.value * ROW > listTop.value)

const x = (lane: number) => Math.min(lane, MAX_LANES - 1) * LANE + LANE / 2 + PAD

const y = (level: number) => (level === 0 ? 0 : level === 1 ? ROW / 2 : ROW)

/**
 * One line segment: straight runs joined by a corner, never a diagonal.
 *
 * A line belongs to a lane, and the whole point of the picture is to show which
 * one. So it travels its lane vertically and does its sideways move in one
 * place — against the node the move is about — turning through a rounded corner
 * rather than sloping across. Two lines running side by side then stay legible
 * as two columns, and a departure or a merge is a corner the eye can land on.
 *
 * A curve between the two points instead sweeps out of one lane and into the
 * other over the whole segment, which reads as a bulge rather than a junction
 * and leaves neither end of it clearly in a lane.
 */
function path(segment: Segment) {
  const x1 = x(segment.x1)
  const x2 = x(segment.x2)
  const y1 = y(segment.y1)
  const y2 = y(segment.y2)
  if (x1 === x2) return `M${x1},${y1} L${x2},${y2}`

  const dir = Math.sign(x2 - x1)
  const r = Math.min(ELBOW, Math.abs(x2 - x1) / 2, Math.abs(y2 - y1))

  // Arriving at this row's node: down the lane it came from, then across.
  if (segment.y2 === 1) {
    return `M${x1},${y1} L${x1},${y2 - r} Q${x1},${y2} ${x1 + dir * r},${y2} L${x2},${y2}`
  }
  // Leaving this row's node: across at the node's own height, then down.
  if (segment.y1 === 1) {
    return `M${x1},${y1} L${x2 - dir * r},${y1} Q${x2},${y1} ${x2},${y1 + r} L${x2},${y2}`
  }
  // Passing the row by while its lane shifts: down, across, down again.
  const mid = (y1 + y2) / 2
  return (
    `M${x1},${y1} L${x1},${mid - r} Q${x1},${mid} ${x1 + dir * r},${mid} ` +
    `L${x2 - dir * r},${mid} Q${x2},${mid} ${x2},${mid + r} L${x2},${y2}`
  )
}

/**
 * Scroll events arrive faster than frames do, and each one re-renders the whole
 * window of rows. Coalescing them to one a frame is the difference between
 * drawing the list once per flick of the wheel and drawing it once for every
 * event the compositor felt like sending.
 */
let scrollQueued = false

function onScroll() {
  cancelUnfold()
  if (scrollQueued) return
  scrollQueued = true
  requestAnimationFrame(() => {
    scrollQueued = false
    if (viewport.value) scrollTop.value = viewport.value.scrollTop
  })
}

function measure() {
  if (viewport.value) height.value = viewport.value.clientHeight
}

function scrollTo(oid: string) {
  const index = store.rows.findIndex((row) => row.oid === oid)
  if (index < 0 || !viewport.value) return
  const target = Math.max(0, ABOVE + index * ROW - height.value / 2 + ROW)
  // Gliding is worth it across a screen or two. Across ten thousand rows it is
  // a long blur that ends somewhere the eye did not follow, so that jumps.
  const far = Math.abs(target - viewport.value.scrollTop) > height.value * 8
  viewport.value.scrollTo({ top: target, behavior: far ? 'auto' : 'smooth' })
}

function step(by: number) {
  if (!matches.value.length) return
  hit.value = (hit.value + by + matches.value.length) % matches.value.length
  const row = matches.value[hit.value]
  if (!row) return
  git.select(row.oid)
  scrollTo(row.oid)
}

// Something outside the graph — a branch clicked in the sidebar — picked a
// commit and wants it shown. The row may be past the loaded page, in which case
// `scrollTo` finds nothing and the detail panel alone answers the question.
watch(
  () => store.revealing?.seq,
  async () => {
    const oid = store.revealing?.oid
    if (!oid) return
    await nextTick()
    scrollTo(oid)
  }
)

watch(
  () => store.query,
  async () => {
    hit.value = 0
    const first = matches.value[0]
    if (first) {
      await nextTick()
      scrollTo(first.oid)
    }
  }
)

/**
 * The search bar is summoned rather than resident: it costs a row of the window
 * and is wanted for seconds at a time. Closing always clears the query, because
 * a filter still applied but no longer on screen is a trap.
 */
async function openSearch() {
  searchOpen.value = true
  await nextTick()
  searchBox.value?.focus()
  searchBox.value?.select()
}

function closeSearch() {
  searchOpen.value = false
  store.query = ''
  searchBox.value?.blur()
}

/** Every row the selection can land on, the working-changes row included. */
const selectable = computed(() => [WIP, ...store.rows.map((row) => row.oid)])

/**
 * Walks the selection up and down the list.
 *
 * The graph is a list, and a list you can only click through is a list you
 * cannot read with your hands on the keyboard. The working-changes row counts
 * as the first entry, since that is where the eye starts.
 */
function move(by: number) {
  const list = selectable.value
  if (!list.length) return
  const at = list.indexOf(store.selected)
  const next = list[Math.min(list.length - 1, Math.max(0, at < 0 ? 0 : at + by))]
  if (!next || next === store.selected) return
  git.select(next)
  if (next === WIP) viewport.value?.scrollTo({ top: 0, behavior: 'smooth' })
  else scrollTo(next)
}

/** True when the keystroke belongs to whatever is being written in. */
function typing(event: KeyboardEvent) {
  const element = event.target as HTMLElement | null
  if (!element) return false
  return element.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(element.tagName)
}

/**
 * True while something sits on top of the graph.
 *
 * A dialog, a menu and a picker all draw their own scrim, and the conflict
 * resolver its own overlay. Whichever it is, the keys belong to it and not to
 * the list underneath.
 */
function covered() {
  return !!document.querySelector('.scrim, .overlay')
}

// Search and its two step keys are the graph's, but they are wanted from
// wherever the hands are, so they are bound centrally rather than here. The
// rest below move the selection and belong to the list itself.
useShortcuts({
  'graph.search': () => openSearch(),
  'graph.next': () => step(1),
  'graph.previous': () => step(-1)
})

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape' && searchOpen.value) {
    closeSearch()
  }

  if (typing(event) || covered() || event.metaKey || event.ctrlKey || event.altKey) return
  // A page is what the window shows, less a row so the eye keeps its place.
  const page = Math.max(1, Math.floor(height.value / ROW) - 1)
  switch (event.key) {
    case 'ArrowDown':
      event.preventDefault()
      move(1)
      break
    case 'ArrowUp':
      event.preventDefault()
      move(-1)
      break
    case 'PageDown':
      event.preventDefault()
      move(page)
      break
    case 'PageUp':
      event.preventDefault()
      move(-page)
      break
    case 'Home':
      event.preventDefault()
      move(-selectable.value.length)
      break
    case 'End':
      event.preventDefault()
      move(selectable.value.length)
      break
  }
}

/**
 * What a hard reset would write over: everything staged, and the tracked files
 * changed but not staged.
 *
 * Untracked files are not in it. `git reset --hard` leaves them exactly where
 * they are, so counting them made the question sound like it was about work
 * that was never at risk.
 */
const atRisk = computed(() => {
  const staged = store.status?.staged.length ?? 0
  const changed = (store.status?.unstaged ?? []).filter(
    (entry) => entry.kind !== 'untracked'
  ).length
  return staged + changed
})

/**
 * Moves the branch, asking first only where there is something to lose.
 *
 * A reset moves a branch. The commits it takes off are not deleted — undo puts
 * the branch back in a keystroke, and git keeps them reachable long after
 * that — so the only thing a reset can really destroy is work that is not in a
 * commit at all, which is what a hard one writes over. With none of that on
 * disk there is nothing to warn about, whichever mode was picked, and a dialog
 * in front of it is a page of reading to confirm something reversible. Reading
 * that every time is how people learn to click through the one that matters.
 *
 * Two cases still ask, because the reset is not the whole of what they undo:
 * one that takes a running merge or rebase apart, and one on a detached HEAD,
 * where there is no branch to bring back.
 */
async function openReset(oid: string, mode: ResetMode) {
  const asking =
    mode === 'hard' && (atRisk.value > 0 || isRunning(store.progress) || !!store.repo?.detached)
  if (asking) {
    resetTarget.value = oid
    return
  }
  // A soft or mixed one says nothing when it works: the branch chip lands on
  // the row that was reset to and what came off it appears in the working
  // list, both in the window the click was made in. A hard one changes files
  // on disk without either list moving, so that one says what it did — and
  // what undoes it.
  const said = await git.reset(oid, mode)
  if (said !== null && mode === 'hard') git.note(`${said}. Undo puts it back.`)
}

// --- ref chips

/**
 * Which branch each row in the list belongs to.
 *
 * Recomputed for the whole list rather than per row, because the answer for
 * one row depends on every row above it.
 */
const lineOwners = computed(() => lineChips(store.rows))

/**
 * Which row's branch column is unfolded, and which way it went.
 *
 * Held here rather than left to `:hover`, for the pause before it opens: the
 * pointer crosses a dozen rows on its way anywhere, and a column that unfolds
 * under each one in turn is a list that will not sit still. A rest of a third
 * of a second says the pointer stopped rather than passed.
 *
 * The direction is a measurement, and there is no asking CSS for one. It grows
 * downwards over the rows below, which runs out of window on the last few of
 * them — and a name half cut off by the bottom edge is the thing this was built
 * to stop — so those unfold upwards instead. Measured when it opens rather than
 * followed as the list scrolls: which way a row would open is only ever a
 * question about the row being pointed at.
 */
const UNFOLD_AFTER = 350

const unfolded = ref<{ oid: string; up: boolean } | null>(null)
let unfoldTimer: number | undefined

function unfold(event: MouseEvent, oid: string, count: number) {
  cancelUnfold()
  if (!count) return
  const cell = event.currentTarget as HTMLElement
  unfoldTimer = window.setTimeout(() => {
    const box = viewport.value?.getBoundingClientRect()
    const rect = cell.getBoundingClientRect()
    // A chip and the gap under it, plus the padding the box adds top and bottom.
    const needed = count * 19 + 12
    unfolded.value = { oid, up: !!box && rect.top + needed > box.bottom }
  }, UNFOLD_AFTER)
}

function cancelUnfold() {
  window.clearTimeout(unfoldTimer)
  if (unfolded.value) unfolded.value = null
}

/** What checking a ref out would do, said before it happens. */
function chipHint(chip: RefChip): string {
  if (chip.kind === 'remote') return 'creates a local branch'
  if (chip.kind === 'tag') return 'detaches HEAD'
  return ''
}

/** The icon standing for where a ref lives. */
function chipIcon(chip: RefChip) {
  return chip.kind === 'tag' ? Tag : chip.kind === 'remote' ? Cloud : MonitorDot
}

/**
 * A branch chip wears the colour of the line it names.
 *
 * The chip, the leader running out of it and the lane it lands in are then one
 * colour and read as one thing, so the strip on the left says which line each
 * name belongs to without the eye having to trace the leader across. Colouring
 * every branch the same blue instead made the column a list of names that
 * happened to be near the graph.
 *
 * Tags keep their own gold. A tag is not a line — it is a marker left on a
 * commit — and painting it the colour of whichever line the commit landed in
 * would be saying something about it that is not true.
 */
function chipStyle(row: GraphRow, chip: RefChip) {
  if (chip.kind === 'tag') return undefined
  return {
    background: laneTint(row.color, chip.head ? 0.28 : 0.15),
    color: chip.head ? '#fff' : laneColor(row.color),
    // An outline rather than a box-shadow, drawn just inside the edge so it
    // costs no layout: box-shadow is what the hover ring is made of, and an
    // inline one would win against it and leave the hover with nothing to say.
    outline: `1px solid ${laneTint(row.color, chip.head ? 0.9 : 0.3)}`,
    outlineOffset: '-1px'
  }
}

/**
 * Checks a ref out, unless it is already the one we are on.
 *
 * Shared by the double-click on a chip and the menu rows, so both refuse the
 * same no-op rather than one of them running a checkout that changes nothing.
 *
 * Nothing else is refused here. Standing this off while the app was busy looked
 * careful and was not: `busy` counts every call in flight, and the first click
 * of the double-click is itself one — it asks for the commit to show on the
 * right. So the guard was true by the time the second click arrived, every
 * time, and checking a branch out from its label stopped working altogether.
 */
function checkoutRef(chip: RefChip) {
  if (chip.head) return
  git.checkout(chip.name)
}

/** Lists every ref on a commit, each one checkout-able. */
function refsMenu(event: MouseEvent, row: GraphRow) {
  const chips = refChips(row)
  menu.show(
    event,
    chips.map((chip) => ({
      label: chip.name,
      icon: chipIcon(chip),
      hint: chip.head ? 'checked out' : chipHint(chip) || (chip.remotes.length ? 'pushed' : ''),
      disabled: chip.head,
      action: () => checkoutRef(chip)
    })),
    `${chips.length} refs on ${row.short}`
  )
}

// --- selecting several commits

/**
 * Plain click selects one and clears any marks; ctrl adds or removes one; shift
 * takes everything between the anchor and here. The same three gestures every
 * list on the desktop uses.
 */
function onRowClick(event: MouseEvent, row: GraphRow) {
  if (row.oid === WIP) {
    clearMarks()
    git.select(row.oid)
    return
  }

  if (event.shiftKey && anchor.value) {
    const oids = store.rows.map((r) => r.oid)
    const from = oids.indexOf(anchor.value)
    const to = oids.indexOf(row.oid)
    if (from >= 0 && to >= 0) {
      const [lo, hi] = from < to ? [from, to] : [to, from]
      marked.value = oids.slice(lo, hi + 1).filter((oid) => oid !== WIP)
    }
  } else if (event.ctrlKey || event.metaKey) {
    marked.value = markedSet.value.has(row.oid)
      ? marked.value.filter((oid) => oid !== row.oid)
      : [...marked.value, row.oid]
    anchor.value = row.oid
  } else {
    clearMarks()
    anchor.value = row.oid
  }

  git.select(row.oid)
}

function clearMarks() {
  marked.value = []
  anchor.value = null
}

/** Right-clicking outside the marks acts on the row under the pointer. */
function subjects(row: GraphRow): string[] {
  return markedSet.value.has(row.oid) && marked.value.length > 1 ? marked.value : [row.oid]
}

// A reload replaces the rows, so marks pointing at them are stale.
watch(() => store.repo?.path, clearMarks)

// --- context menus

function commitMenu(event: MouseEvent, row: GraphRow) {
  // A stash row is not a commit of the history: cherry-picking it, resetting
  // to it or branching from it are all questions about the wrong thing.
  if (row.stash !== null) {
    const at = row.stash
    menu.show(
      event,
      [
        {
          label: 'Apply and keep',
          icon: Archive,
          action: () => git.stashApply(at)
        },
        {
          label: 'Pop (apply and remove)',
          icon: Archive,
          action: () => git.stashPop(at)
        },
        { separator: true, label: '' },
        {
          label: 'Drop this stash…',
          icon: Trash2,
          danger: true,
          action: () => {
            dropping.value = at
          }
        }
      ],
      row.summary
    )
    return
  }

  const isHead = row.labels.some((label) => label.kind === 'local' && label.name === store.repo?.head)
  const picked = subjects(row)
  // A commit carrying refs can be reached by name, and switching to the branch
  // is nearly always what was meant by "go here" — it keeps HEAD attached,
  // where checking the commit out by hash does not. So the named refs go above
  // the hash-level entries, without replacing them.
  // Branch names on this commit, local first: what someone means by "copy the
  // branch name" is the branch, not the tag sitting beside it.
  const branchNames = row.labels
    .filter((label) => label.kind === 'local' || label.kind === 'remote')
    .map((label) => label.name)

  const namedCheckouts = refChips(row)
    .filter((chip) => !chip.head)
    .map((chip) => ({
      label: `Checkout ${chip.name}`,
      icon: chipIcon(chip),
      hint: chipHint(chip),
      action: () => checkoutRef(chip)
    }))
  menu.show(
    event,
    [
      ...namedCheckouts,
      ...(namedCheckouts.length ? [{ separator: true, label: '' }] : []),
      // Branching is the safe way to work from an old commit, so it goes first:
      // checking the commit out directly detaches HEAD, and anything committed
      // afterwards belongs to no branch.
      {
        label: 'Branch from here…',
        icon: GitBranchPlus,
        action: () => (branchTarget.value = row)
      },
      {
        label: 'Checkout this commit',
        icon: Check,
        hint: 'detaches HEAD',
        action: () => git.checkout(row.oid)
      },
      { label: 'Tag this commit…', icon: Tag, action: () => (tagTarget.value = row) },
      { separator: true, label: '' },
      {
        // The commits above this one are the ones a plan can act on: this one
        // is the ground they are replayed onto, so it is not in the list.
        label: 'Rebase the commits above this…',
        icon: GitBranch,
        disabled: isHead || store.repo?.detached,
        hint: isHead
          ? 'nothing above it'
          : store.repo?.detached
            ? 'not on a branch'
            : 'reorder, squash, drop',
        action: () => rebase.planFrom(row.oid, row.short)
      },
      {
        // The one-gesture version of the plan above: a run of commits already
        // sitting together, folded into one without a list to arrange. Picking
        // a single commit means it and the one under it, which is what "squash
        // this into its parent" is for.
        label: picked.length > 1 ? `Squash ${picked.length} commits into one…` : 'Squash into the commit below…',
        icon: GitMerge,
        disabled: picked.length === 1 && !row.parents.length,
        hint:
          picked.length > 1
            ? 'one commit, one message'
            : row.parents.length
              ? `with ${row.parents[0]!.slice(0, 7)}`
              : 'nothing below it',
        action: () => {
          squashing.value = picked.length > 1 ? picked : [row.parents[0]!, row.oid]
        }
      },
      { separator: true, label: '' },
      {
        label:
          picked.length > 1
            ? `Cherry-pick ${picked.length} commits onto ${store.repo?.head ?? 'this branch'}`
            : 'Cherry-pick onto current branch',
        icon: GitCommitHorizontal,
        hint: picked.length > 1 ? 'oldest first' : '',
        action: async () => {
          if (await git.cherryPick(picked)) clearMarks()
        }
      },
      {
        label: picked.length > 1 ? `Cherry-pick ${picked.length} without committing` : 'Cherry-pick without committing',
        icon: GitCommitHorizontal,
        hint: 'stages the changes',
        action: async () => {
          if (await git.cherryPick(picked, { no_commit: true })) clearMarks()
        }
      },
      {
        label: 'Cherry-pick, recording the origin',
        icon: GitCommitHorizontal,
        hint: 'adds "cherry picked from"',
        action: async () => {
          if (await git.cherryPick(picked, { record_origin: true })) clearMarks()
        }
      },
      {
        label: 'Revert this commit',
        icon: Undo2,
        hint: 'adds a commit',
        action: () => git.revert(row.oid)
      },
      // What each reset mode keeps is the whole question, so the three are
      // offered by name rather than left for the dialog to explain after the
      // fact. Each still opens the dialog, which names what would be lost.
      {
        label: `Reset ${store.repo?.head ?? 'branch'} to this commit`,
        icon: ArrowDownToLine,
        disabled: isHead,
        children: [
          {
            label: 'Soft',
            icon: ArrowDownToLine,
            hint: 'keep all changes, staged',
            action: () => openReset(row.oid, 'soft')
          },
          {
            label: 'Mixed',
            icon: ArrowDownToLine,
            hint: 'keep all changes, unstaged',
            action: () => openReset(row.oid, 'mixed')
          },
          {
            label: 'Hard',
            icon: ArrowDownToLine,
            danger: true,
            hint: 'discard all changes',
            action: () => openReset(row.oid, 'hard')
          }
        ]
      },
      { separator: true, label: '' },
      {
        label: 'Copy full hash',
        icon: Copy,
        action: () => copyText(row.oid, 'Hash')
      },
      {
        label: 'Copy short hash',
        icon: Copy,
        hint: row.short,
        action: () => copyText(row.short, 'Hash')
      },
      // Only for a commit that carries refs, since only then is there a name to
      // copy. One ref copies itself; several ask which, rather than guessing and
      // being wrong half the time.
      ...(branchNames.length === 1
        ? [
            {
              label: 'Copy branch name',
              icon: Copy,
              hint: branchNames[0]!,
              action: () => copyText(branchNames[0]!, 'Branch')
            }
          ]
        : []),
      ...(branchNames.length > 1
        ? [
            {
              label: 'Copy branch name',
              icon: Copy,
              children: branchNames.map((name) => ({
                label: name,
                icon: Copy,
                action: () => copyText(name, 'Branch')
              }))
            }
          ]
        : []),
      {
        label: 'Copy message',
        icon: FileText,
        action: async () => {
          const text = await git.commitMessageText(row.oid)
          if (text) copyText(text, 'Message')
        }
      },
      {
        label: 'Copy patch',
        icon: FileText,
        action: async () => {
          const text = await git.commitPatch(row.oid)
          if (text) copyText(text, 'Patch')
        }
      },
      { separator: true, label: '' },
      {
        label: 'Search for this hash',
        icon: Search,
        action: () => (store.query = row.short)
      }
    ],
    picked.length > 1
      ? `${picked.length} commits selected`
      : `${row.short} · ${row.summary.slice(0, 44)}`
  )
}

function wipMenu(event: MouseEvent) {
  menu.show(
    event,
    [
      { label: 'Stage everything', icon: Check, action: () => git.stageAll() },
      {
        label: 'Stash everything',
        icon: ArrowDownToLine,
        action: () => git.stashPush()
      },
      {
        label: 'Discard all unstaged changes',
        icon: X,
        danger: true,
        disabled: !(store.status?.unstaged.length ?? 0),
        action: () => git.discard((store.status?.unstaged ?? []).map((e) => e.path))
      }
    ],
    'Uncommitted changes'
  )
}

/**
 * Picks a row up: a commit to cherry-pick, or a stash to apply.
 *
 * The sidebar has taken both of these on a branch since it was written — it is
 * the drop half of a gesture whose other half was never here, so dragging a
 * commit did nothing at all. The row already answers to a branch dropped on
 * it; now it can be picked up as well.
 */
function beginDrag(event: DragEvent, row: GraphRow) {
  if (row.stash !== null) {
    const stash = store.stashes.find((one) => one.index === row.stash)
    drag.begin(event, {
      kind: 'stash',
      index: row.stash,
      message: stash?.message ?? row.summary
    })
    return
  }
  drag.begin(event, {
    kind: 'commit',
    oid: row.oid,
    short: row.short,
    summary: row.summary
  })
}

/**
 * Dropping a branch on a commit moves the branch there.
 *
 * A mixed reset, which is why it asks nothing: every change on disk is left
 * exactly as it is — staged ones simply stop being staged — and undo puts the
 * branch back where it was. Dragging up the list is the way back by hand.
 *
 * Only the branch you are on, though. `git reset` moves whichever branch HEAD
 * points at, so dropping another one here moved the current branch instead and
 * said nothing about it: the wrong branch, silently, on a gesture meant to be
 * quick.
 */
function onDropOnRow(row: GraphRow) {
  const payload = drag.take(['branch'])
  if (!payload || payload.kind !== 'branch' || payload.remote) return
  // A stash is not a place a branch belongs.
  if (row.stash !== null) return
  if (payload.name !== store.repo?.head) {
    git.note(`Check out ${payload.name} first — this moves the branch you are on`, 'error')
    return
  }
  // Mid-merge or mid-rebase the reset would take the operation apart under it,
  // which is not what dragging a branch a few rows means.
  if (isRunning(store.progress)) {
    git.note('Finish or abort what is running before moving the branch', 'error')
    return
  }
  openReset(row.oid, 'mixed')
}

const observer = new ResizeObserver(measure)
onMounted(() => {
  measure()
  if (viewport.value) observer.observe(viewport.value)
  window.addEventListener('keydown', onKey)
})
onUnmounted(() => {
  observer.disconnect()
  window.removeEventListener('keydown', onKey)
  window.clearTimeout(unfoldTimer)
})
</script>

<template>
  <section class="graph">
    <!-- Summoned with ⌘F rather than always on screen: it is wanted for
         seconds at a time and costs a row of the window the whole session. -->
    <div v-if="searchOpen" class="head">
      <span class="search">
        <Search :size="13" class="faint" />
        <!-- The arrows walk the matches, which is what the eye expects of a
             field with a "1 of 9" beside it. Enter does the same, since in a
             search box it has nothing else to do. -->
        <input
          ref="searchBox"
          v-model="store.query"
          type="search"
          placeholder="Search messages, authors, hashes"
          @keydown.down.prevent="step(1)"
          @keydown.up.prevent="step(-1)"
          @keydown.enter.prevent="step($event.shiftKey ? -1 : 1)"
        />
        <template v-if="store.query.trim()">
          <span class="count" :class="{ none: !matches.length }">
            {{ matches.length ? `${hit + 1} of ${matches.length}` : 'no matches' }}
          </span>
          <button class="step" :disabled="!matches.length" title="Previous (↑ or ⇧⌘G)" @click="step(-1)">
            <ChevronUp :size="13" />
          </button>
          <button class="step" :disabled="!matches.length" title="Next (↓ or ⌘G)" @click="step(1)">
            <ChevronDown :size="13" />
          </button>
        </template>
        <button class="step" title="Close (Esc)" @click="closeSearch">
          <X :size="13" />
        </button>
      </span>
    </div>

    <!-- Column headings, so the branch strip on the left reads as a column
         rather than as labels that drifted away from their commits. -->
    <div class="colhead" @contextmenu="columnMenu($event)">
      <template v-if="cols.state.shown.refs">
        <span class="col-refs" :style="box('refs')">Branch / tag</span>
        <span class="grip" :class="{ active: resizing === 'refs' }" title="Drag to resize, double-click to reset"
              @pointerdown="startResize($event, 'refs', 1)"
              @dblclick="cols.resetWidth('refs')" />
      </template>
      <template v-if="cols.state.shown.graph">
        <span class="cell-head" :style="box('graph')">Graph</span>
        <span class="grip" :class="{ active: resizing === 'graph' }" title="Drag to resize, double-click to reset"
              @pointerdown="startResize($event, 'graph', 1)"
              @dblclick="cols.resetWidth('graph')" />
      </template>
      <span class="col-msg">Commit message</span>
      <!-- Lives here rather than beside the search box, which is not always on
           screen to hold it. -->
      <button v-if="marked.length > 1" class="marks" @click="clearMarks()">
        {{ marked.length }} selected
        <X :size="11" />
      </button>
      <template v-if="cols.state.shown.author">
        <span class="grip" :class="{ active: resizing === 'author' }" title="Drag to resize, double-click to reset"
              @pointerdown="startResize($event, 'author', -1)"
              @dblclick="cols.resetWidth('author')" />
        <span class="col-author" :style="box('author')">Author</span>
      </template>
      <template v-if="cols.state.shown.date">
        <span class="grip" :class="{ active: resizing === 'date' }" title="Drag to resize, double-click to reset"
              @pointerdown="startResize($event, 'date', -1)"
              @dblclick="cols.resetWidth('date')" />
        <span class="col-date" :style="box('date')">Date</span>
      </template>

      <!-- The two things you had to know a keystroke to find. Right-clicking
           the headings still opens the same column menu; this is the same door
           with a handle on it. -->
      <span class="head-tools">
        <button
          class="tool"
          :class="{ on: searchOpen }"
          :title="`Search commits (${keyLabel('mod+f')})`"
          @click.stop="searchOpen ? closeSearch() : openSearch()"
        >
          <Search :size="13" />
        </button>
        <button class="tool" title="Which columns to show" @click.stop="columnMenu($event)">
          <Settings2 :size="13" />
        </button>
      </span>
    </div>

    <div ref="viewport" class="viewport" @scroll.passive="onScroll">
      <!-- The working tree: the first row of the list rather than a strip
             pinned above it. It scrolls away like anything else, and being an
             ordinary row is what keeps its dot in line with the dots below. -->
      <div
        class="wip"
        :class="{ on: store.selected === WIP }"
        @click="git.select(WIP)"
        @contextmenu="wipMenu($event)"
      >
        <span v-if="cols.state.shown.refs" class="col-refs" :style="box('refs')" />
        <span v-if="cols.state.shown.graph" class="cell-box" :style="box('graph')">
        <svg class="cell" :width="graphWidth" :height="ROW" :viewBox="`0 0 ${graphWidth} ${ROW}`">
          <path
            v-if="store.rows.length"
            :d="`M${x(headLane)},${ROW / 2} L${x(headLane)},${ROW}`"
            :stroke="laneColor(headColor)"
            stroke-width="2"
            stroke-dasharray="2 3"
            stroke-linecap="round"
            fill="none"
          />
          <!-- Dotted whether or not anything is changed: it is not a commit, and
               the ring says so. Amber only says there is something in it. -->
          <circle
            :cx="x(headLane)"
            :cy="ROW / 2"
            r="4"
            fill="var(--bg)"
            :stroke="dirty || conflicts ? 'var(--warning)' : 'var(--fg-subtle)'"
            stroke-width="1.8"
            stroke-dasharray="2 2"
          />
        </svg>
        </span>
        <!-- One line rather than a badge and a line. The row is already the one
             at the top, already selected, and already the only one with a hollow
             ring on the graph; a coloured pill on top of that was the fourth way
             of saying the same thing. -->
        <span class="col-msg">
          <span class="summary truncate" :class="{ quiet: !dirty && !conflicts }">
            <template v-if="conflicts">
              <strong class="count bad">{{ conflicts }}</strong> conflicted —
              resolve before committing
            </template>
            <template v-else-if="dirty">
              <strong class="count">{{ dirty }}</strong> uncommitted
              {{ dirty === 1 ? 'change' : 'changes' }}
            </template>
            <template v-else>No local changes</template>
          </span>
        </span>
        <!-- Whoever a commit made here would be authored by, rather than "you":
             with a profile per context, which identity is in force is the thing
             worth showing. -->
        <span v-if="cols.state.shown.author" class="col-author faint truncate" :style="box('author')">
          {{ store.repo?.author || 'no author set' }}
        </span>
        <span v-if="cols.state.shown.date" class="col-date faint" :style="box('date')">now</span>
      </div>

      <div class="spacer" :style="{ height: `${total * ROW}px` }">
        <div
          v-for="item in window_"
          :key="item.row.oid"
          class="row"
          :class="{
            on: store.selected === item.row.oid,
            marked: markedSet.has(item.row.oid),
            hit: matchIds.has(item.row.oid),
            dim: store.query.trim() && !matchIds.has(item.row.oid),
            drop: drag.state.over === `commit:${item.row.oid}`
          }"
          :style="{ top: `${item.top}px` }"
          draggable="true"
          @click="onRowClick($event, item.row)"
          @contextmenu="commitMenu($event, item.row)"
          @dragstart="beginDrag($event, item.row)"
          @dragend="drag.end()"
          @dragover="drag.hover($event, `commit:${item.row.oid}`, ['branch'])"
          @dragleave="drag.leave($event, `commit:${item.row.oid}`)"
          @drop.prevent="onDropOnRow(item.row)"
        >
          <!-- Refs live in their own column with a line running to the node,
               so a tip is found by scanning one narrow strip rather than by
               reading the start of every message. -->
          <span
            v-if="cols.state.shown.refs"
            class="col-refs"
            :class="{
              open: unfolded?.oid === item.row.oid,
              up: unfolded?.oid === item.row.oid && unfolded.up
            }"
            :style="box('refs')"
            @mouseenter="unfold($event, item.row.oid, item.refs.length)"
            @mouseleave="cancelUnfold"
          >
            <!-- Every ref the commit carries, though only the first is on
                 show: the rest are folded away behind a counter, so a commit
                 with five refs takes the same width in the column as one with a
                 single branch.

                 Resting on the column unfolds it. The names are cut to fit and
                 the counter says only that there is more, so the column can ask
                 a question it cannot answer; the set grows to its full width
                 over the graph beside it, which is empty space the moment you
                 are reading names rather than lines. It is the same set of
                 chips throughout — grown, not replaced — so nothing moves under
                 the pointer and the one you were looking at stays where it was.

                 Done with `:hover` rather than by opening something: a panel
                 drawn over the cell takes the pointer off the cell that
                 summoned it, and closes itself. A descendant cannot — the cell
                 is still hovered while the pointer is anywhere inside it. -->
            <span v-if="item.refs.length" class="refs-set">
              <span
                v-for="(chip, at) in item.refs"
                :key="chip.key"
                class="chip"
                :class="[
                  `chip-${chip.kind}`,
                  { 'chip-current': chip.head, 'chip-live': !chip.head, folded: at > 0 }
                ]"
                :style="chipStyle(item.row, chip)"
                @dblclick.stop="checkoutRef(chip)"
              >
                <Check v-if="chip.head" :size="11" :stroke-width="3" class="glyph" />
                <!-- Cut in the middle. Four chips reading `origin/ASANA-1216293…`
                     are the same chip as far as the eye is concerned; the digits
                     that differ are at the end. -->
                <MidTruncate :text="chip.name" />
                <component
                  :is="chip.kind === 'remote' ? Cloud : chip.kind === 'tag' ? Tag : MonitorDot"
                  :size="11"
                  class="glyph"
                />
                <!-- A local branch that is also on its remote says so here rather
                     than by growing a second chip with the same name in it. -->
                <Cloud
                  v-if="chip.kind === 'local' && chip.remotes.length"
                  :size="11"
                  class="glyph"
                />
              </span>
              <!-- After the chips, not before them. In front, the counter
                   indented the name behind it by its own width, so the one row
                   in ten that carries several refs was the one row whose name
                   did not start where every other name in the column starts. -->
              <button
                v-if="item.hidden.length"
                class="more-refs"
                @click.stop="refsMenu($event, item.row)"
              >
                +{{ item.hidden.length }}
              </button>
            </span>
            <!-- The branch this commit is on, for the rows that are not the tip
                 of anything. Hidden until the pointer is on the row: printed at
                 full strength it would be the same name repeated down a hundred
                 rows, which is noise rather than an answer.

                 The same chip as a real ref otherwise, and checked out by the
                 same double-click — one gesture for "put me on that branch"
                 wherever the name is read, rather than a second one to learn
                 for the rows in between. The tick is left off: this commit is
                 on that branch, but it is not where the branch is. -->
            <template
              v-for="chip in (item.ghost ? [item.ghost] : [])"
              :key="chip.key"
            >
              <span
                class="chip ghost"
                :class="[`chip-${chip.kind}`, { 'chip-live': !chip.head }]"
                :style="chipStyle(item.row, { ...chip, head: false })"
                :title="ghostTitle(chip)"
                @dblclick.stop="checkoutRef(chip)"
              >
                <MidTruncate :text="chip.name" />
                <component :is="chipIcon(chip)" :size="11" class="glyph" />
              </span>
              <span
                class="ghost-leader"
                :style="{ background: laneColor(item.row.color) }"
              />
            </template>
            <!-- Carries the leader on from the chip to the edge of the column,
                 where the graph's own line picks it up and runs to the node. -->
            <span
              v-if="item.row.labels.length"
              class="leader"
              :style="{ background: laneColor(item.row.color) }"
            />
          </span>

          <span v-if="cols.state.shown.graph" class="cell-box" :style="box('graph')">
          <svg
            class="cell"
            :width="graphWidth"
            :height="ROW"
            :viewBox="`0 0 ${graphWidth} ${ROW}`"
          >
            <!-- The thread back to where you are standing: the working tree at
                 the top of the list, down its lane, to the commit HEAD is on.
                 Drawn under everything else, so where a real line already
                 occupies the lane the real line is what you see and the dots
                 only show through the stretches that are empty. -->
            <path
              v-if="headTrace(item.index)"
              :d="`M${x(headLane)},0 L${x(headLane)},${headTrace(item.index)}`"
              :stroke="laneColor(headColor)"
              stroke-width="2"
              stroke-dasharray="2 3"
              stroke-linecap="round"
              opacity="0.55"
              fill="none"
            />
            <!-- The ghost label's half of the leader, carrying it across to the
                 node so the name and the line it names are joined up. -->
            <path
              v-if="item.ghost"
              class="ghost-leader"
              :d="`M0,${ROW / 2} L${x(item.row.lane)},${ROW / 2}`"
              :stroke="laneColor(item.row.color)"
              stroke-width="1.2"
              fill="none"
            />
            <!-- The leader from the column to the node. Drawn here rather than
                 in the column because only the graph knows which lane the
                 commit landed in. -->
            <path
              v-if="item.row.labels.length"
              :d="`M0,${ROW / 2} L${x(item.row.lane)},${ROW / 2}`"
              :stroke="laneColor(item.row.color)"
              stroke-width="1.2"
              opacity="0.45"
              fill="none"
            />
            <path
              v-for="(segment, i) in item.row.segments"
              :key="i"
              :d="path(segment)"
              :stroke="laneColor(segment.color)"
              fill="none"
              stroke-width="2"
              stroke-linecap="round"
              :stroke-dasharray="segment.dashed ? '3 3' : undefined"
              :opacity="segment.dashed ? 0.75 : undefined"
            />
            <!-- A commit the upstream does not have yet wears a ring. Colour
                 alone will not do it: the first lane is already the accent
                 colour, so the boundary between what the remote has and what is
                 still only here has to be a difference in shape. -->
            <circle
              v-if="item.row.unpushed && !item.junction"
              :cx="x(item.row.lane)"
              :cy="ROW / 2"
              :r="NODE + 3"
              fill="none"
              :stroke="laneColor(item.row.color)"
              stroke-width="1.5"
              opacity="0.55"
            />
            <!-- Where the history forked or came back together. Hollow while
                 the commit is only here, filled once the remote has it, which
                 is the same distinction the bigger nodes draw as a ring. -->
            <g v-if="item.junction">
              <circle
                :cx="x(item.row.lane)"
                :cy="ROW / 2"
                :r="JOINT"
                :fill="item.row.unpushed ? 'var(--bg)' : laneColor(item.row.color)"
                :stroke="laneColor(item.row.color)"
                stroke-width="2"
              />
              <title>
                {{ item.row.author }} · a branch joins or parts here{{
                  item.row.unpushed ? ' · not pushed yet' : ''
                }}
              </title>
            </g>
            <!-- A stash is not part of the history, so it is not drawn as one:
                 the box it is kept in, on the broken line back to the commit
                 it was made on. -->
            <g v-else-if="item.row.stash !== null">
              <circle
                :cx="x(item.row.lane)"
                :cy="ROW / 2"
                :r="NODE"
                fill="var(--bg)"
              />
              <g
                :transform="`translate(${x(item.row.lane) - 6.5}, ${ROW / 2 - 6.5})`"
                fill="none"
                :stroke="laneColor(item.row.color)"
                stroke-width="1.6"
                stroke-linecap="round"
                stroke-linejoin="round"
              >
                <rect x="1" y="4.5" width="11" height="7.5" rx="1" />
                <path d="M0.5 1.5h12v3h-12z" />
                <path d="M5.5 8h2" />
              </g>
              <title>{{ item.row.summary }} · a stash, not a commit</title>
            </g>
            <!-- The node is the author's face. Who wrote a run of commits is
                 then read down the column at a glance, rather than one line of
                 the author column at a time.

                 The lane's colour stays, as the ring around it: it is what ties
                 the node to the lines running into it. -->
            <g v-else>
              <clipPath :id="`node-${item.row.oid}`">
                <circle :cx="x(item.row.lane)" :cy="ROW / 2" :r="NODE" />
              </clipPath>
              <!-- The lines behind are knocked out first, so a face is never
                   drawn over by whatever passes through its lane. -->
              <circle
                :cx="x(item.row.lane)"
                :cy="ROW / 2"
                :r="NODE"
                :fill="item.picture ? 'var(--bg)' : item.tint"
              />
              <image
                v-if="item.picture"
                :href="item.picture ?? undefined"
                :x="x(item.row.lane) - NODE"
                :y="ROW / 2 - NODE"
                :width="NODE * 2"
                :height="NODE * 2"
                :clip-path="`url(#node-${item.row.oid})`"
                preserveAspectRatio="xMidYMid slice"
              />
              <text
                v-else-if="item.letters"
                :x="x(item.row.lane)"
                :y="ROW / 2"
                text-anchor="middle"
                dominant-baseline="central"
                font-size="7.5"
                font-weight="700"
                fill="#fff"
              >
                {{ item.letters }}
              </text>
              <!-- Every node is the same size, merge or not. A merge used to be
                   drawn hollow, back when a node was a dot; a second ring inside
                   this one only crops the face and makes it look shrunken, and
                   the two lines running into the node already say it is a
                   merge. -->
              <circle
                :cx="x(item.row.lane)"
                :cy="ROW / 2"
                :r="NODE"
                fill="none"
                :stroke="laneColor(item.row.color)"
                stroke-width="2"
              />
              <title>
                {{ item.row.author }}{{ item.row.unpushed ? ' · not pushed yet' : '' }}
              </title>
            </g>
          </svg>
          </span>

          <span class="col-msg">
            <!-- Signed, and what that was worth. Nothing at all for the
                 commits nobody signed, which in most repositories is all of
                 them: a column of grey marks would be noise standing in for
                 information. Off unless the setting asks for it. -->
            <component
              :is="signatureLook(store.signatures[item.row.oid]?.verdict)?.icon"
              v-if="store.signatures[item.row.oid]"
              :size="13"
              :stroke-width="2.2"
              class="sig"
              :class="signatureLook(store.signatures[item.row.oid]?.verdict)?.tone"
              :title="
                signatureTitle(
                  store.signatures[item.row.oid]?.verdict,
                  store.signatures[item.row.oid]?.signer
                )
              "
            />
            <!-- The message in full on hover. A narrowed message column cuts
                 most summaries off, and the alternative to reading it here is
                 selecting the commit to see it in the panel — which throws away
                 whatever was selected to answer a question about a row you were
                 only passing over. -->
            <span class="summary truncate" :title="item.row.summary">
              <span
                v-for="(part, i) in item.parts"
                :key="i"
                :class="{ mark: part.hit }"
                >{{ part.text }}</span
              >
            </span>
          </span>
          <span v-if="cols.state.shown.author" class="col-author truncate" :style="box('author')">
            {{ item.row.author }}
          </span>
          <span
            v-if="cols.state.shown.date"
            class="col-date faint"
            :style="box('date')"
            :title="item.whenFull"
          >
            {{ item.when }}
          </span>
        </div>
      </div>

      <div v-if="store.hasMore" class="more">
        <button class="btn btn-ghost" :disabled="store.busy" @click="git.loadMore()">
          Load older commits
        </button>
      </div>
      <div v-else-if="total === 0" class="empty dim">No commits yet.</div>
    </div>

    <!-- Where you are, when where you are has scrolled away. Checking out
         something old and then reading history leaves the one row that answers
         "and where am I in all this?" off the screen, with nothing but the
         scrollbar to say which direction it went. -->
    <button
      v-if="headOffScreen && headRow"
      class="to-head"
      :style="{
        color: laneColor(headColor),
        backgroundImage: `linear-gradient(${laneTint(headColor, 0.18)}, ${laneTint(headColor, 0.18)})`,
        boxShadow: `inset 0 0 0 1px ${laneTint(headColor, 0.4)}`
      }"
      :title="`Scroll to ${headRow.short} — the commit you are on`"
      @click="scrollTo(headRow.oid)"
    >
      <component :is="headBelow ? ChevronDown : ChevronUp" :size="12" />
      <span class="truncate">{{ store.repo?.detached ? 'HEAD' : store.repo?.head }}</span>
    </button>

    <ResetDialog v-if="resetTarget" :oid="resetTarget" @close="resetTarget = null" />
    <TagDialog v-if="tagTarget" :row="tagTarget" @close="tagTarget = null" />
    <DropStashDialog v-if="dropping !== null" :index="dropping" @close="dropping = null" />
    <SquashDialog
      v-if="squashing"
      :oids="squashing"
      @done="clearMarks()"
      @close="squashing = null"
    />
    <BranchDialog
      v-if="branchTarget"
      :start="branchTarget.oid"
      :start-label="`${branchTarget.short} · ${branchTarget.summary.slice(0, 40)}`"
      @close="branchTarget = null"
    />
  </section>
</template>

<style scoped>
/* A flex column rather than a grid with named row tracks. The search bar comes
   and goes, so the number of children varies, and a grid told how many rows to
   expect drops the extra child into an implicit row at the bottom — which is
   what made the whole graph appear at the foot of an empty panel. Flex stacks
   whatever is there and gives the rest to the list. */
.graph {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  /* The column headings and the rows are laid out from stated widths, so in a
     narrow window they are wider than the space there is for them. Positioned,
     as this is, they were painted over the panel beside it rather than being
     cut off at its own edge. */
  overflow: hidden;
  background: var(--bg);
  /* Holds the "back to HEAD" pill, which floats over the list rather than
     inside it: a child of the scroller would scroll away with everything else,
     and it is wanted precisely when things have scrolled away. */
  position: relative;
}

.to-head {
  position: absolute;
  right: 14px;
  bottom: 12px;
  z-index: 2;
  display: flex;
  align-items: center;
  gap: 4px;
  max-width: 180px;
  padding: 3px 9px 3px 6px;
  border-radius: 11px;
  font-size: 11px;
  font-weight: 600;
  /* Sits over rows of text, so it needs a ground of its own to be read against.
     The lane tint bound to it is translucent by design and goes on top of this
     as a background image, rather than replacing it. */
  background-color: var(--bg-panel);
}

.to-head:hover {
  filter: brightness(1.25);
}

.head {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: none;
  padding: 5px 12px 5px 8px;
  border-bottom: 1px solid var(--line);
  user-select: none;
}

.search {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding: 0 8px;
  background: var(--bg-panel);
  border: 1px solid var(--line);
  border-radius: 5px;
}

.search input {
  flex: 1;
  min-width: 0;
  border: none;
  background: none;
  padding: 4px 0;
  font-size: 12px;
}

.search input:focus {
  outline: none;
}

/* The browser's own clear button for a search field, next to ours, offering
   the same thing in a different shape. */
.search input::-webkit-search-cancel-button {
  display: none;
}

.count {
  font-size: 10.5px;
  color: var(--text-dim);
  white-space: nowrap;
}

.count.none {
  color: var(--amber);
}

.step {
  display: grid;
  place-items: center;
  width: 18px;
  height: 18px;
  border-radius: 4px;
  color: var(--text-faint);
}

.step:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
}

.step:disabled {
  opacity: 0.35;
}

.marks {
  display: flex;
  align-items: center;
  gap: 5px;
  flex: none;
  padding: 2px 8px;
  border-radius: 9px;
  font-size: 11px;
  font-weight: 600;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 16%, transparent);
}

/* The same box as `.row`, so the graph runs through both without a step in
   it. A border here would eat a pixel of the row's height and lift everything
   drawn in it above the dots underneath. */
.wip {
  display: flex;
  align-items: center;
  gap: 10px;
  height: var(--row-h);
  padding: 0 12px 0 8px;
  cursor: default;
  user-select: none;
}

.wip:hover {
  background: var(--bg-hover);
}

.wip.on {
  background: var(--bg-active);
}

.viewport {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  position: relative;
}

.spacer {
  position: relative;
}

.row {
  position: absolute;
  left: 0;
  right: 0;
  height: 27px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 12px 0 8px;
  cursor: default;
  user-select: none;
}

.row:hover {
  background: var(--bg-hover);
}

.row.on {
  background: var(--bg-active);
}

/* A marked row reads as part of a set without competing with the one selected
   row, which is the one the right panel is showing. */
.row.marked {
  background: color-mix(in srgb, var(--accent) 13%, transparent);
  box-shadow: inset 2px 0 0 var(--accent);
}

.row.marked.on {
  background: color-mix(in srgb, var(--accent) 20%, transparent);
}

.row.dim {
  opacity: 0.36;
}

.row.hit {
  background: var(--warning-bg);
}

.row.drop {
  outline: 1px solid var(--accent);
  outline-offset: -1px;
  background: color-mix(in srgb, var(--accent) 14%, transparent);
}

.cell {
  flex: none;
  display: block;
}

/* Holds the graph at whatever width the user gave the column. The svg keeps its
   own size — scaling it would thin the lines and move the nodes off the rows —
   so a narrowed column hides the lanes on the right rather than squeezing them
   together. */
.cell-box {
  flex: none;
  overflow: hidden;
  display: block;
  height: 100%;
}

/* The branch strip. The chips start at the left edge, where the eye already is
   for every other column, and a leader carries the line from the chip across to
   the graph rather than the chips being pushed over to meet it. */
.col-refs {
  flex: none;
  /* The width is set on the element: it is the user's, and the heading, the
     working-tree row and every commit row have to agree on it. */
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 4px;
  overflow: hidden;
  /* Cancels the row's gap on this side alone, so the leader line — which starts
     at the left edge of the graph cell — begins against the chip instead of
     after a gap it cannot reach across. */
  margin-right: -10px;
  padding-right: 0;
}

.leader {
  flex: 1;
  min-width: 4px;
  height: 1.2px;
  opacity: 0.45;
}

/* The ghost's own leader rather than `.leader` with a modifier beside it: the
   two want different opacities, they are the same weight of selector, and the
   one declared later wins whatever the markup says. Sharing the class put a
   visible leader on every row in the list. */
.ghost-leader {
  flex: 1;
  min-width: 4px;
  height: 1.2px;
  opacity: 0;
  transition: opacity 90ms ease;
}

.row:hover .ghost-leader {
  opacity: 0.3;
}

/* The chips, and the box that grows to hold all of them.
 *
 * At rest it is an ordinary flex item clipped to the column. Under the pointer
 * it is taken out of the flow and given its natural width, so it grows to the
 * right over the graph rather than widening the column and shoving every other
 * column along with it.
 *
 * Absolute, and `.col-refs` is deliberately left `static`, so the containing
 * block is the row: an absolutely positioned element is clipped by an
 * ancestor's `overflow` only from its containing block inwards, and the column
 * is exactly the `overflow: hidden` it has to escape. */
.refs-set {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
}

/* Folded away until asked for. `display: none` rather than a width of zero:
   a chip with no width still takes its gap, and four of them put the one chip
   on show a quarter of the column from the left. */
.refs-set .chip.folded {
  display: none;
}

/* Down the column, not along it. Stacked, the names are read the way a list is
   read and each one is as long as it needs to be; in a row they were a line of
   chips competing for the same width, which is what the column already was.

   The padding is what puts the first chip back exactly where it sits at rest,
   so the one you were pointing at does not move as the rest appear under it. */
.col-refs.open .refs-set {
  position: absolute;
  left: 0;
  top: 0;
  z-index: 6;
  flex-direction: column;
  align-items: flex-start;
  width: max-content;
  /* Never wider than the row it is in, however many refs a release commit
     ended up carrying: the list scroller would answer with a horizontal
     scrollbar under the whole graph. */
  max-width: min(720px, 100%);
  overflow: hidden;
  /* No padding on the left and no shift with it: the box starts exactly where
     the column does and grows only to the right. Insetting it to sit the chips
     off its edge meant starting six pixels to the left of the column, which on
     the leftmost column in the window is six pixels off the edge of it. */
  padding: 6px 8px 6px 0;
  border-radius: 5px;
  background: var(--bg-hover);
  box-shadow: 0 3px 14px var(--shadow);
}

/* The last rows in the window have no room below them, so they grow the other
   way. Reversed as well as anchored, so the chip that was on show stays against
   its own row and the rest pile up above it rather than the order flipping. */
.col-refs.open.up .refs-set {
  top: auto;
  bottom: 0;
  flex-direction: column-reverse;
}

.row.on .col-refs.open .refs-set {
  background: var(--bg-active);
}

/* Stacked, a chip is as wide as its own name; without this each one stretches
   to the width of the longest and the short names read as empty boxes. */
.col-refs.open .refs-set .chip {
  flex: none;
}

.col-refs.open .refs-set .chip.folded {
  display: inline-flex;
}

/* The whole point: the name is not cut once there is room for it. */
.col-refs.open .refs-set .chip {
  max-width: none;
}

/* It has been answered — the chips it stood for are all on show. */
.col-refs.open .more-refs {
  display: none;
}

/* The counter for the refs not on show. Deliberately quiet: it is a way in,
   not a label competing with the branch beside it. */
.more-refs {
  flex: none;
  padding: 0 5px;
  border-radius: 3px;
  font-size: 10px;
  font-weight: 600;
  color: var(--text-faint);
  background: var(--bg-raised);
}

.more-refs:hover {
  color: var(--text);
  background: var(--bg-active);
}

.colhead {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  flex: none;
  padding: 3px 12px 3px 8px;
  font-size: 10px;
  letter-spacing: 0.07em;
  text-transform: uppercase;
  color: var(--text-faint);
  border-bottom: 1px solid var(--line);
  user-select: none;
}

.cell-head {
  flex: none;
  overflow: hidden;
}

/* Over the right-hand end of the headings rather than in the flow of them: a
   button that took part in the layout would push the date heading out of line
   with the dates underneath it. */
.head-tools {
  position: absolute;
  right: 6px;
  top: 0;
  bottom: 0;
  display: flex;
  align-items: center;
  gap: 2px;
  padding-left: 10px;
  background: linear-gradient(to right, transparent, var(--surface) 10px);
}

.tool {
  display: grid;
  place-items: center;
  width: 22px;
  height: 20px;
  border-radius: var(--radius-sm);
  color: var(--fg-subtle);
}

.tool:hover {
  color: var(--fg);
  background: var(--hover);
}

.tool.on {
  color: var(--primary);
}

/* The strip between two headings. It is wider than it looks: a four-pixel
   target is a fight, so it reaches into the gap on both sides and draws a line
   only when the pointer is on it. */
.grip {
  flex: none;
  width: 9px;
  margin: 0 -9px 0 -1px;
  align-self: stretch;
  cursor: col-resize;
  position: relative;
  z-index: 2;
}

.grip::after {
  content: '';
  position: absolute;
  inset: 2px auto 2px 4px;
  width: 1px;
  background: var(--text-faint);
  opacity: 0;
}

.grip:hover::after,
.grip.active::after {
  opacity: 0.7;
}

.grip.active::after {
  background: var(--accent);
  opacity: 1;
}

/* The heading reads left to right; only the chips below it hug the graph. */
/* The author and date columns carry a size and colour of their own for the
   rows. In the heading they were inheriting those instead of the heading's, so
   two of the five column names were drawn larger and lighter than the rest. */
.colhead .col-author,
.colhead .col-date {
  font-size: inherit;
  color: inherit;
}

/* Room for the two buttons at the end of the row. */
.colhead .col-date {
  padding-right: 50px;
}

.col-msg {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 5px;
  overflow: hidden;
}

.summary {
  min-width: 0;
}

/* Quiet when it is fine, loud only when it is wrong: a bad signature is the
   one case here that needs a decision. */
.sig {
  flex: none;
}

.sig.good {
  color: var(--green);
}

.sig.warn {
  color: var(--amber);
}

.sig.bad {
  color: var(--red);
}

.summary.quiet {
  color: var(--text-faint);
}

.mark {
  background: var(--warning-line);
  border-radius: 2px;
  color: var(--amber-soft);
}

.col-author {
  flex: none;
  color: var(--text-dim);
  font-size: 12px;
}

.col-date {
  flex: none;
  text-align: right;
  font-size: 12px;
}

.chip {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  flex: none;
  padding: 1px 6px;
  border-radius: 3px;
  font-size: 10px;
  font-weight: 600;
  max-width: 180px;
  overflow: hidden;
  white-space: nowrap;
  /* Double-clicking a chip checks it out; without this the second click
     selects the name instead of reading as a gesture. */
  user-select: none;
}

/* In the branch column the chip is whatever the column leaves it, rather than
   a fixed 180px that a narrowed column would clip mid-word. The 180 still holds
   for the chips in the message column, which are labels of a known length. */
.col-refs .chip {
  flex: 0 1 auto;
  min-width: 0;
  max-width: 100%;
  /* Held a shade back at rest and brought forward under the pointer. A column
     of names at full strength beside a column of commit messages competes with
     the messages for the eye; a column held back reads as an index you glance
     at, and the row you are actually on still comes forward. */
  opacity: 0.8;
  transition: opacity 90ms ease;
}

.row:hover .col-refs .chip,
.row.on .col-refs .chip {
  opacity: 1;
}

/* The branch a commit is on, for the rows that mark no tip of their own. The
   same chip as a real ref, one step back: it is the answer to a question asked
   by pointing, and it must not be mistaken at a glance for a ref that is
   actually sitting on this commit.

   Written at the weight of `.col-refs .chip` and after it, because a plain
   `.ghost` loses to it and the label would then be printed down every row in
   the list. */
.col-refs .ghost {
  opacity: 0;
  font-weight: 500;
}

.row:hover .col-refs .ghost {
  opacity: 0.62;
}

/* Under the pointer it is a button and says so, at full strength. */
.col-refs .ghost:hover {
  opacity: 1;
}

/* A ref that is not the one we are on can be checked out from here, so it
   answers the pointer: the hand and a lift in brightness say the name is a
   way in, not a caption. The chip's own colour is left alone — the hover is
   an outline and a step up in weight, so a branch, a remote and a tag each
   stay recognisable while lit. */
.chip-live {
  cursor: pointer;
  transition: box-shadow 90ms ease, filter 90ms ease;
}

.chip-live:hover {
  filter: brightness(1.28);
  box-shadow: inset 0 0 0 1px currentColor;
}

.chip-live:active {
  filter: brightness(1.1);
}

/* The branch you are on: brighter, outlined, and ticked. Everything else on
   the same commit stays flat, so the eye lands on this one. */
.chip-current {
  background: color-mix(in srgb, var(--accent) 32%, transparent);
  color: var(--accent-soft);
  box-shadow: inset 0 0 0 1px var(--accent);
}

/* The tick and the kind glyph either side of the name: a tick for the branch
   you are on, then a screen, a cloud or a tag for where the ref lives. */
.chip .glyph {
  flex: none;
  opacity: 0.75;
}

.chip-current .glyph {
  opacity: 1;
}

.chip-local {
  background: color-mix(in srgb, var(--accent) 18%, transparent);
  color: var(--accent-soft);
}

.chip-remote {
  background: var(--info-bg);
  color: var(--purple-soft);
}

.chip-tag {
  background: var(--warning-bg);
  color: var(--amber-soft);
}

.chip-head {
  background: var(--success-bg);
  color: var(--green-soft);
}

/* The number is the news on that row; the words around it are grammar. */
.count {
  color: var(--warning-soft);
  font-weight: 600;
}

.count.bad {
  color: var(--danger-soft);
}

.more,
.empty {
  display: flex;
  justify-content: center;
  padding: 14px;
}
</style>
