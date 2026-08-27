<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  ChevronDown,
  ChevronRight,
  ChevronsDown,
  ChevronsUp,
  FileCheck2,
  Layers,
  ListChecks,
  Sparkles,
  Trash2,
  Undo2
} from 'lucide-vue-next'
import {
  useGit,
  type ConflictBlock,
  type ConflictFile,
  type Resolution
} from '~/composables/useGit'
import { useAi } from '~/composables/useAi'
import { highlightWhole, languageFor } from '~/composables/useHighlight'
import { windowOf, type Mark } from '~/composables/useCode'
import { usePanes } from '~/composables/usePanes'
import { useContextMenu, type MenuItem } from '~/composables/useContextMenu'
import { useToasts } from '~/composables/useToasts'
import {
  HEAD,
  ROW,
  buildGrid,
  conflictMarks,
  freshPick,
  mapTop,
  originMarks,
  pickedLines,
  resolutionOf,
  resultAnchors,
  resultOrigins,
  rowWindow,
  sideLines,
  stanceOf,
  type GridRow,
  type Pick,
  type Side,
  type Stance
} from '~/composables/useConflictGrid'

const git = useGit()
const store = git.store
const ai = useAi()
const { layout } = usePanes()
const menu = useContextMenu()
const toasts = useToasts()

type Conflict = Extract<ConflictBlock, { kind: 'conflict' }>

/** What the bar's four buttons ask for, on behalf of the whole file. */
type Wanted = 'ours' | 'theirs' | 'both' | 'none'

/** Which conflict region the model is currently working on. */
const thinking = ref<number | null>(null)

/** How far a run of the model over every file has got, while one is running. */
const sweeping = ref<{ file: string; at: number; of: number } | null>(null)

/** Set while the confirmation for that run is on screen. */
const askingSweep = ref(false)

/**
 * The conflicted files that still have markers in them.
 *
 * Read rather than guessed at: a file can be finished in an editor while this
 * page is open, and "stage every file as it stands" is only safe once none of
 * them reads as a conflict any more.
 */
const stillMarked = ref<string[]>([])

const path = ref<string | null>(null)
const file = ref<ConflictFile | null>(null)
const picks = ref<Pick[]>([])
/**
 * The answer given for the whole file, or `null` when there is none.
 *
 * The bar's four buttons say something about the file rather than about a
 * region, so what lights one is having been pressed — and any change to any
 * region afterwards puts the bar back to nothing selected, because the claim
 * stopped being true. `edit` is the one way a region changes, so that is where
 * it is cleared.
 *
 * Read rather than derived from the regions, because the regions cannot always
 * answer it: "take theirs" where their side deleted those lines leaves a region
 * with nothing in it, which reads as dropped and is indistinguishable from
 * having pressed Neither. The file was answered; only the file knows how.
 */
const asked = ref<Wanted | null>(null)
const result = ref('')
const showBase = ref(false)
const showResult = ref(true)
const loading = ref(false)
/** The region being worked on: what the arrows walk and the strip points at. */
const active = ref(0)

const files = computed(() => store.status?.conflicted ?? [])
const blocks = computed(() => file.value?.blocks ?? [])
const conflicts = computed(
  () => blocks.value.filter((block): block is Conflict => block.kind === 'conflict')
)
const hasBase = computed(() => conflicts.value.some((block) => block.has_base))
const withBase = computed(() => showBase.value && hasBase.value)
const oursLabel = computed(() => conflicts.value[0]?.ours_label || 'ours')
const theirsLabel = computed(() => conflicts.value[0]?.theirs_label || 'theirs')

/** How far through the file the reader is: regions they have actually decided. */
const reviewed = computed(() => picks.value.filter((pick) => pick.touched).length)
const dropped = computed(
  () => picks.value.filter((pick) => stanceOf(pick) === 'dropped').length
)
const left = computed(() => conflicts.value.length - reviewed.value)

/**
 * A conflict with no markers in the file.
 *
 * Git only writes markers when both sides have the file and it had to merge
 * them line by line. When one side deleted it, or a merge driver took a whole
 * side, the file on disk reads normally and the conflict lives entirely in the
 * index — so the side-by-side view has nothing to show and the per-region
 * buttons have nothing to act on. Saying which case it is beats two identical
 * panes above three buttons that quietly do nothing.
 */
const stages = computed(() => file.value?.stages ?? null)
const wholeFile = computed(() => !!file.value && file.value.conflict_count === 0)
const deletedBy = computed(() => {
  if (!wholeFile.value || !stages.value) return null
  if (!stages.value.ours && stages.value.theirs) return 'ours'
  if (stages.value.ours && !stages.value.theirs) return 'theirs'
  return null
})
const explanation = computed(() => {
  if (!wholeFile.value) return null
  if (deletedBy.value === 'ours') {
    return `This file is gone on the branch you are on, and changed on the other side. There are no lines to merge — either bring it back with those changes, or leave it deleted.`
  }
  if (deletedBy.value === 'theirs') {
    return `The other side deleted this file and you changed it. There are no lines to merge — either keep your version, or let it go.`
  }
  return `Git could not merge this file line by line — it is binary, or a merge driver took a whole side — so there is nothing to pick through here. Choose a side, or keep the file exactly as it stands on disk.`
})

const language = computed(() => (path.value ? languageFor(path.value) : null))

/**
 * Each side coloured as one piece rather than a line at a time.
 *
 * The things a line cannot know about itself — an open block comment, a string
 * that runs on, the `<script>` block that turns a `.vue` file into JavaScript —
 * are exactly the ones a merge lands in the middle of.
 */
function paint(side: Side) {
  const lines = sideLines(blocks.value, side)
  return highlightWhole(lines.join('\n'), language.value)
}

const painted = computed(() => ({
  ours: paint('ours'),
  theirs: paint('theirs'),
  base: withBase.value ? paint('base') : []
}))

/** One row model, so the sides cannot drift apart. */
const grid = computed(() => buildGrid(blocks.value, painted.value, withBase.value))

/** What each side's widest line is, which is what holds the panes open sideways. */
const widest = computed(() => {
  const found = { ours: '', theirs: '', base: '' }
  for (const side of ['ours', 'theirs', 'base'] as Side[]) {
    for (const line of sideLines(blocks.value, side)) {
      if (line.length > found[side].length) found[side] = line
    }
  }
  return found
})

const panes = computed(() => {
  const list: { side: Side; label: string; branch: string }[] = [
    { side: 'ours', label: 'Ours', branch: oursLabel.value }
  ]
  if (withBase.value) list.push({ side: 'base', label: 'Base', branch: 'merge base' })
  list.push({ side: 'theirs', label: 'Theirs', branch: theirsLabel.value })
  return list
})

// --- scrolling
//
// Every pane is drawn from the same rows, so one scroll position means the same
// place in all of them and they can simply be held equal — sideways as well as
// down, since a long conflicted line is exactly the one you want to compare.
// The result is a different length by design, so it is pinned to the panes at
// the block boundaries and read off proportionally in between.
const bodies = new Map<Side, HTMLElement>()
const outBody = ref<HTMLElement | null>(null)
/** Our side's box, which is the one the strip beside the scrollbar measures. */
const rulerBox = ref<HTMLElement | null>(null)
const top = ref(0)
const boxHeight = ref(0)
const outTop = ref(0)
const outHeight = ref(0)
/**
 * Which group of boxes the reader is actually scrolling.
 *
 * The panes and the result move each other, so each one's scroll event arrives
 * back as an echo of the other's. Naming the driver settles which of the two to
 * believe: whoever moved last keeps the wheel until they stop for a moment, and
 * the other side's events are the echo and are dropped.
 */
let driver: 'panes' | 'out' | null = null
let droveAt = 0
const HANDOVER = 150

function claim(who: 'panes' | 'out') {
  const now = performance.now()
  if (driver !== who && now < droveAt + HANDOVER) return false
  driver = who
  droveAt = now
  return true
}
/** Until when a scroll is one we caused, and so says nothing about intent. */
let held = 0
let framed = { panes: false, out: false }
let sizer: ResizeObserver | null = null

function setBody(side: Side, element: unknown) {
  const box = element as HTMLElement | null
  const old = bodies.get(side)
  if (old && old !== box) {
    old.removeEventListener('scroll', onPaneScroll)
    sizer?.unobserve(old)
  }
  if (!box) {
    bodies.delete(side)
    if (side === 'ours') rulerBox.value = null
    return
  }
  if (old === box) return
  bodies.set(side, box)
  if (side === 'ours') rulerBox.value = box
  box.addEventListener('scroll', onPaneScroll, { passive: true })
  sizer?.observe(box)
  // A pane that has just appeared — the base one, or a file that has just been
  // opened — starts where the others already are.
  box.scrollTop = top.value
  measure()
}

/** Writing a value a box already has fires nothing, which is what ends the echo. */
function follow(box: HTMLElement, down: number, across: number) {
  if (Math.abs(box.scrollTop - down) > 0.5) box.scrollTop = down
  if (Math.abs(box.scrollLeft - across) > 0.5) box.scrollLeft = across
}

/** Scroll events outrun frames; the drawn window only has to be right once. */
function mark(from: HTMLElement, out = false) {
  const key = out ? 'out' : 'panes'
  if (framed[key]) return
  framed[key] = true
  requestAnimationFrame(() => {
    framed[key] = false
    if (out) outTop.value = from.scrollTop
    else {
      top.value = from.scrollTop
      follows()
    }
  })
}

/**
 * Keeps "conflict 3 of 8" honest while the file is being scrolled by hand.
 *
 * Scrolling past a region and then pressing the arrow key should carry on from
 * where the eye is, not from wherever the last button click left off. Held off
 * for a moment after a jump, because the scroll a jump causes lands here too —
 * and two regions close together would answer it with the wrong one.
 */
function follows() {
  const spots = grid.value.spots
  if (!spots.length || performance.now() < held) return
  const at = spots.findIndex((spot) => spot.top + spot.height > top.value + HEAD)
  active.value = (at === -1 ? spots[spots.length - 1]! : spots[at]!).index
}

function onPaneScroll(event: Event) {
  const from = event.currentTarget as HTMLElement
  mark(from)
  // The panes are one grid: they are held equal whichever of them was touched,
  // and that costs nothing when they already are.
  for (const box of bodies.values()) {
    if (box !== from) follow(box, from.scrollTop, from.scrollLeft)
  }
  const out = outBody.value
  if (!out || !showResult.value || !claim('panes')) return
  follow(out, Math.round(mapTop(anchors.value, from.scrollTop)), out.scrollLeft)
}

function onResultScroll(event: Event) {
  const from = event.currentTarget as HTMLElement
  mark(from, true)
  if (!claim('out')) return
  const to = Math.round(mapTop(anchors.value, from.scrollTop, true))
  for (const box of bodies.values()) follow(box, to, box.scrollLeft)
}

function measure() {
  const first = bodies.values().next().value
  if (first) boxHeight.value = first.clientHeight
  if (outBody.value) outHeight.value = outBody.value.clientHeight
}

/** Puts every pane at the same place, and the result pane alongside it. */
function scrollAllTo(down: number) {
  const at = Math.max(0, down)
  // This is the panes moving, whatever was scrolled last.
  driver = 'panes'
  droveAt = performance.now()
  for (const box of bodies.values()) box.scrollTop = at
  top.value = at
  const out = outBody.value
  if (out && showResult.value) out.scrollTop = Math.round(mapTop(anchors.value, at))
}

// --- what is actually drawn
const shown = computed(() => rowWindow(grid.value.rows, top.value, boxHeight.value))
const visible = computed(() => grid.value.rows.slice(shown.value.first, shown.value.last))
const padTop = computed(() => grid.value.rows[shown.value.first]?.top ?? 0)
const padBottom = computed(() => {
  const last = grid.value.rows[shown.value.last - 1]
  return last ? grid.value.height - (last.top + last.height) : 0
})

/** The result pane, coloured and numbered the same way. */
const resultRows = computed(() => {
  if (!result.value) return [] as { num: number; html: string }[]
  const lines = result.value.replace(/\n$/, '').split('\n')
  const coloured = highlightWhole(lines.join('\n'), language.value)
  return lines.map((_, at) => ({ num: at + 1, html: coloured[at] ?? '' }))
})
/**
 * Which lines of the result are the resolution, and where each came from.
 *
 * The result is mostly the file as it already was; the lines that answer a
 * conflict are the few worth looking at, and until they were marked they read
 * exactly like the thousands that were never in question.
 */
const origins = computed(() => resultOrigins(blocks.value, (index) => picks.value[index]))

const outShown = computed(() =>
  windowOf(resultRows.value.length, outTop.value, outHeight.value)
)
const outVisible = computed(() =>
  resultRows.value.slice(outShown.value.first, outShown.value.last).map((row) => ({
    ...row,
    // `num` is one-based, and a resolution that came out shorter than the
    // preview it was rendered from simply has nothing to say about a line.
    origin: origins.value[row.num - 1] ?? null
  }))
)

/** The same strip beside the result, showing what it wrote where. */
const outMarks = computed<Mark[]>(() =>
  originMarks(origins.value, (index) => {
    const pick = picks.value[index]
    if (!pick) return 'open'
    if (stanceOf(pick) === 'dropped') return 'dropped'
    return pick.touched ? 'settled' : 'open'
  })
)
const outWidest = computed(() => {
  let found = ''
  for (const line of result.value.split('\n')) if (line.length > found.length) found = line
  return found
})

/** Where the panes and the result agree about what line is what. */
const anchors = computed(
  () =>
    resultAnchors(blocks.value, grid.value, (index) => {
      const pick = picks.value[index]
      const block = conflicts.value[index]
      return pick && block ? pickedLines(pick, block) : 0
    }).anchors
)

/** The strip beside the scrollbar: one bar per conflict, coloured by its state. */
const marks = computed<Mark[]>(() =>
  conflictMarks(grid.value.spots, grid.value.height, (index) => {
    const pick = picks.value[index]
    if (!pick) return 'open'
    if (stanceOf(pick) === 'dropped') return 'dropped'
    return pick.touched ? 'settled' : 'open'
  })
)

// --- the choices
function pickAt(index: number) {
  return picks.value[index]
}

function edit(index: number, change: (pick: Pick) => void) {
  const pick = picks.value[index]
  if (!pick) return
  // One region changing is the file no longer being whatever was asked for.
  asked.value = null
  const next: Pick = {
    ours: [...pick.ours],
    theirs: [...pick.theirs],
    ours_first: pick.ours_first,
    custom: pick.custom,
    touched: true
  }
  change(next)
  picks.value = picks.value.map((old, at) => (at === index ? next : old))
  active.value = index
}

const stance = (index: number): Stance | null => {
  const pick = picks.value[index]
  return pick ? stanceOf(pick) : null
}

/** Whether one line of one side goes into the result. */
function lineOn(index: number, side: Side, at: number) {
  const pick = picks.value[index]
  if (!pick || at < 0) return false
  if (pick.custom) return true
  return side === 'theirs' ? !!pick.theirs[at] : !!pick.ours[at]
}

function toggleLine(index: number, side: Side, at: number) {
  if (side === 'base' || at < 0) return
  edit(index, (pick) => {
    // A hand or AI edit is a different answer entirely; going back to picking
    // lines drops it rather than pretending the two can be mixed.
    pick.custom = null
    const flags = side === 'ours' ? pick.ours : pick.theirs
    flags[at] = !flags[at]
  })
}

/** The head checkbox: on unless the whole side is already in. */
function sideOn(index: number, side: Side) {
  const pick = picks.value[index]
  if (!pick) return false
  if (pick.custom) return true
  const flags = side === 'theirs' ? pick.theirs : pick.ours
  return flags.length > 0 && flags.every(Boolean)
}

function sideSome(index: number, side: Side) {
  const pick = picks.value[index]
  if (!pick || pick.custom) return false
  const flags = side === 'theirs' ? pick.theirs : pick.ours
  return flags.some(Boolean) && !flags.every(Boolean)
}

function toggleSide(index: number, side: Side) {
  if (side === 'base') return
  const want = !sideOn(index, side)
  edit(index, (pick) => {
    pick.custom = null
    const flags = side === 'ours' ? pick.ours : pick.theirs
    for (let at = 0; at < flags.length; at++) flags[at] = want
  })
}

function swapOrder(index: number) {
  edit(index, (pick) => {
    pick.ours_first = !pick.ours_first
  })
}

function undoEdit(index: number) {
  edit(index, (pick) => {
    pick.custom = null
  })
}

/**
 * The answer for every conflict in the file at once.
 *
 * This is what the four buttons in the bar do. Answering one region at a time
 * is what the checkboxes in the panes are for, and the common case by far is
 * wanting one side of the whole file — so the buttons that sit in a fixed place
 * on screen are the ones that say it once.
 */
function takeAll(want: Wanted) {
  asked.value = want
  picks.value = picks.value.map((pick, index) => {
    const block = conflicts.value[index]
    if (!block) return pick
    return {
      ours: block.ours.map(() => want === 'ours' || want === 'both'),
      theirs: block.theirs.map(() => want === 'theirs' || want === 'both'),
      ours_first: pick.ours_first,
      custom: null,
      touched: true
    }
  })
}

/**
 * The ways out for a file that has no regions to pick through.
 *
 * One side deleted it, or a merge driver took a whole side, or it is binary:
 * there is nothing to lay out side by side, so the answer is which side the
 * file should come from — or that what is on disk is already it.
 */
function forFile(event: MouseEvent) {
  const gone = { ours: !stages.value?.ours, theirs: !stages.value?.theirs }
  const items: MenuItem[] = [
    {
      label: gone.ours ? 'Leave it deleted' : 'Use our whole file',
      icon: gone.ours ? Trash2 : FileCheck2,
      danger: gone.ours,
      disabled: store.busy,
      action: () => takeWholeFile('ours')
    },
    {
      label: gone.theirs ? 'Let it go — delete the file' : 'Use their whole file',
      icon: gone.theirs ? Trash2 : FileCheck2,
      danger: gone.theirs,
      disabled: store.busy,
      action: () => takeWholeFile('theirs')
    }
  ]
  items.push(
    { separator: true, label: '' },
    {
      label: 'Keep the file exactly as it is on disk',
      icon: FileCheck2,
      disabled: store.busy,
      action: keepAsIs
    }
  )
  menu.show(event, items, path.value ?? '')
}

// --- walking the conflicts
function goTo(index: number) {
  const spots = grid.value.spots
  if (!spots.length) return
  const at = Math.min(Math.max(index, 0), spots.length - 1)
  held = performance.now() + 250
  active.value = spots[at]!.index
  // A few lines of context above it, so the region has somewhere to sit.
  scrollAllTo(spots[at]!.top - HEAD * 2)
}

function step(by: number) {
  goTo(active.value + by)
}

/** The next region nobody has looked at, which is what "review" means here. */
function nextOpen() {
  const total = picks.value.length
  for (let step = 1; step <= total; step++) {
    const at = (active.value + step) % total
    if (!picks.value[at]?.touched) {
      goTo(at)
      return
    }
  }
}

async function keepAsIs() {
  if (!path.value) return
  kept.delete(path.value)
  await git.conflictResolveAsIs(path.value)
  const next = store.status?.conflicted.find((name) => name !== path.value)
  if (next) await load(next)
  else clear()
}

function clear() {
  path.value = null
  file.value = null
  result.value = ''
  store.resolving = null
}

/**
 * What was chosen in each file, for as long as this page is open.
 *
 * Nothing is written until a file is marked resolved, and the view holds one
 * file's regions at a time — so switching to another file and back used to hand
 * back a file nobody had ever answered, with the work silently gone. Keyed by
 * path, dropped as each file is finished.
 */
const kept = new Map<string, { picks: Pick[]; asked: Wanted | null }>()

/** Whether remembered choices still describe the file that was just read. */
function fits(saved: Pick[], blocks: Conflict[]): boolean {
  if (saved.length !== blocks.length) return false
  return saved.every((pick, at) => {
    const block = blocks[at]!
    return pick.ours.length === block.ours.length && pick.theirs.length === block.theirs.length
  })
}

async function load(target: string) {
  // Hold on to what the file being left had, before its picks are replaced.
  if (path.value && path.value !== target && picks.value.length) {
    kept.set(path.value, { picks: picks.value, asked: asked.value })
  }
  path.value = target
  // The overlay is driven by this, so keep the two in step.
  store.resolving = target
  loading.value = true
  file.value = await git.conflictRead(target)
  const blocks = (file.value?.blocks ?? []).filter(
    (block): block is Conflict => block.kind === 'conflict'
  )
  const saved = kept.get(target)
  // A file that changed on disk since it was last looked at is a different
  // file, and answers made about the old one no longer line up with it.
  const usable = saved && fits(saved.picks, blocks) ? saved : null
  picks.value = usable ? usable.picks : blocks.map((block) => freshPick(block))
  asked.value = usable ? usable.asked : null
  active.value = 0
  loading.value = false
  await preview()
  // The first conflict is what the file was opened for; the top of it rarely is.
  await nextTick()
  goTo(0)
}

/** What the backend is told, in the order it numbers the regions. */
function choicesOf(): Resolution[] {
  return picks.value.flatMap((pick, index) => {
    const block = conflicts.value[index]
    return block ? [resolutionOf(pick, block)] : []
  })
}

async function preview() {
  if (!path.value) return
  result.value = (await git.conflictPreview(path.value, choicesOf())) ?? ''
}

async function markResolved() {
  if (!path.value) return
  const target = path.value
  await git.conflictResolve(target, choicesOf())
  kept.delete(target)
  // Move on to whatever is still conflicted, or clear the view when done.
  const next = (store.status?.conflicted ?? []).find((name) => name !== target)
  if (next) await load(next)
  else clear()
}

async function takeWholeFile(side: 'ours' | 'theirs') {
  if (!path.value) return
  kept.delete(path.value)
  await git.conflictResolveWhole(path.value, side)
  const next = store.status?.conflicted[0]
  if (next) await load(next)
  else clear()
}

/**
 * Asks the model for one region and stores its answer as a hand edit, so it
 * shows up in the result pane like any other choice and can still be dropped.
 */
async function aiResolve(index: number) {
  if (!path.value) return
  thinking.value = index
  try {
    const lines = await ai.resolveConflict(path.value, index)
    if (lines) {
      edit(index, (pick) => {
        pick.custom = lines
      })
      git.note(`Model resolved conflict ${index + 1} — check it before accepting`)
    }
  } catch (error) {
    git.note(`AI resolve: ${String(error)}`, 'error')
  } finally {
    thinking.value = null
  }
}

async function aiResolveAll() {
  for (const block of conflicts.value) {
    await aiResolve(block.index)
  }
}

// --- every file at once

/**
 * What to do once a whole-merge action has finished.
 *
 * Whatever is still conflicted becomes the file on screen; when nothing is, the
 * page has no reason to exist and closes, which is the point of these actions.
 */
async function afterAll(said: string | null) {
  if (said === null) return
  kept.clear()
  const next = store.status?.conflicted[0]
  if (next) await load(next)
  else clear()
  toasts.info(said)
}

/** Takes one side in every conflicted file, not just this one. */
async function resolveEvery(side: 'ours' | 'theirs') {
  await afterAll(await git.conflictResolveAll(side))
}

/** Stages every conflicted file as it stands. Refused while markers remain. */
async function stageEvery() {
  await afterAll(await git.conflictStageAll())
}

/**
 * Walks every conflicted file, asking the model for each region in turn.
 *
 * Each file is read, answered region by region and written once, rather than
 * written after each answer: a half-answered file staged in the middle of a run
 * that then fails is worse than one nobody has touched. A file with no regions
 * — one side deleted it, or it is binary — has nothing to ask about and is left
 * for a person.
 */
async function aiEveryFile() {
  askingSweep.value = false
  const targets = [...files.value]
  let done = 0
  for (const [at, name] of targets.entries()) {
    sweeping.value = { file: name, at: at + 1, of: targets.length }
    const read = await git.conflictRead(name)
    const blocks = (read?.blocks ?? []).filter(
      (block): block is Conflict => block.kind === 'conflict'
    )
    if (!blocks.length) continue
    const choices: Resolution[] = []
    for (const block of blocks) {
      const lines = await ai.resolveConflict(name, block.index).catch((error) => {
        git.note(`AI resolve: ${String(error)}`, 'error')
        return null
      })
      // One refusal ends the run: the rest would fail the same way, and the
      // files answered so far are already written and staged.
      if (!lines) {
        sweeping.value = null
        await afterAll(`The model answered ${done} of ${targets.length} files`)
        return
      }
      choices.push({ take_ours: true, take_theirs: true, ours_first: true, custom: lines })
    }
    if ((await git.conflictResolve(name, choices)) === null) break
    done += 1
  }
  sweeping.value = null
  await afterAll(`The model answered ${done} of ${targets.length} files — read them before committing`)
}

/** The model, over more than the one region on screen. */
/** The whole merge, answered from one menu. */
function everyFile(event: MouseEvent) {
  const count = files.value.length
  const many = count === 1 ? 'file' : 'files'
  const items: MenuItem[] = [
    {
      label: `Take ours in all ${count} ${many}`,
      icon: ListChecks,
      disabled: store.busy,
      action: () => resolveEvery('ours')
    },
    {
      label: `Take theirs in all ${count} ${many}`,
      icon: ListChecks,
      disabled: store.busy,
      action: () => resolveEvery('theirs')
    }
  ]
  if (ai.configured.value) {
    items.push(
      { separator: true, label: '' },
      {
        label: `Ask the model to resolve all ${count} ${many}`,
        icon: Sparkles,
        hint: 'one call per conflict',
        disabled: store.busy || thinking.value !== null || sweeping.value !== null,
        action: () => {
          askingSweep.value = true
        }
      }
    )
  }
  items.push(
    { separator: true, label: '' },
    {
      label: `Stage all ${count} ${many} as they stand`,
      icon: FileCheck2,
      hint: stillMarked.value.length
        ? `${stillMarked.value.length} still have markers`
        : 'for a merge finished elsewhere',
      disabled: store.busy || stillMarked.value.length > 0,
      action: stageEvery
    }
  )
  menu.show(event, items, `${count} conflicted ${many}`)
}

// --- keys
function typing(event: KeyboardEvent) {
  const element = event.target as HTMLElement | null
  if (!element) return false
  return (
    element.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(element.tagName)
  )
}

function onKey(event: KeyboardEvent) {
  if (typing(event) || event.metaKey || event.ctrlKey || event.altKey) return
  if (document.querySelector('.scrim')) return
  if (!conflicts.value.length) return
  if (event.key === 'ArrowDown') {
    event.preventDefault()
    step(1)
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    step(-1)
  } else if (event.key === 'Tab') {
    event.preventDefault()
    nextOpen()
  }
}

const cellOf = (row: GridRow, side: Side) =>
  side === 'ours' ? row.ours : side === 'theirs' ? row.theirs : row.base

const atOf = (row: GridRow, side: Side) => (side === 'theirs' ? row.theirAt : row.ourAt)

const countOf = (index: number, side: Side) => {
  const block = conflicts.value[index]
  if (!block) return 0
  return side === 'ours' ? block.ours.length : side === 'theirs' ? block.theirs.length : block.base.length
}

watch(picks, preview, { deep: true })
// The base pane changes how tall every region is, so the panes have to be put
// back where they were rather than left at a now-meaningless offset.
watch(withBase, async () => {
  await nextTick()
  goTo(active.value)
})
watch(showResult, async (open) => {
  await nextTick()
  measure()
  if (open) scrollAllTo(top.value)
})

// Open the first conflicted file as soon as there is one.
watch(
  files,
  async (list) => {
    if (!path.value && list.length) load(list[0]!)
    else if (path.value && !list.includes(path.value)) {
      const next = list[0] ?? null
      if (next) load(next)
      else clear()
    }
    // Which of them still read as conflicts decides whether staging the lot is
    // offered at all, and that changes with every write in the work tree.
    stillMarked.value = list.length ? ((await git.conflictMarked()) ?? list) : []
  },
  { immediate: true }
)

// The result pane comes and goes with its fold, and so does the box that has
// to be measured for it.
watch(outBody, (box, old) => {
  if (old) sizer?.unobserve(old)
  if (box) sizer?.observe(box)
  measure()
})

onMounted(() => {
  window.addEventListener('keydown', onKey)
  sizer = new ResizeObserver(measure)
  for (const box of bodies.values()) sizer.observe(box)
  if (outBody.value) sizer.observe(outBody.value)
  measure()
})

onBeforeUnmount(() => {
  for (const box of bodies.values()) box.removeEventListener('scroll', onPaneScroll)
  sizer?.disconnect()
})

onUnmounted(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <section class="conflicts">
    <div v-if="!files.length" class="clear">
      <div>
        <h3>No conflicts</h3>
        <p class="dim">
          When a merge stops with conflicts, each file shows up here with both sides side by side.
        </p>
      </div>
    </div>

    <template v-else>
      <div class="rail">
        <div class="section-title">Conflicted files</div>
        <button
          v-for="name in files"
          :key="name"
          class="rail-file"
          :class="{ on: name === path }"
          :title="name"
          @click="load(name)"
        >
          <span class="dot" />
          <span class="rail-text">
            <span class="truncate">{{ name.split('/').pop() }}</span>
            <span class="faint truncate small">{{ name }}</span>
          </span>
        </button>
      </div>

      <div class="work">
        <!-- What file this is and how it ends: the two things that never change
             while the regions below are being picked through. -->
        <div class="toolbar">
          <span class="file-name mono truncate" :title="path ?? ''">{{ path }}</span>
          <span v-if="loading" class="chip">reading…</span>
          <span v-else-if="wholeFile" class="chip">whole file</span>
          <template v-else>
            <span class="chip">
              {{ conflicts.length }} {{ conflicts.length === 1 ? 'conflict' : 'conflicts' }}
            </span>
            <span class="progress" :title="`${reviewed} of ${conflicts.length} decided`">
              <span
                class="fill"
                :style="{ width: `${(reviewed / Math.max(1, conflicts.length)) * 100}%` }"
              />
            </span>
            <span class="stat">
              <template v-if="left">{{ left }} left to look at</template>
              <template v-else>all {{ conflicts.length }} decided</template>
              <template v-if="dropped">
                · <span class="warn">{{ dropped }} dropped</span>
              </template>
            </span>
          </template>
          <span v-if="sweeping" class="chip busy">
            <Spinner :size="11" />
            asking the model — file {{ sweeping.at }} of {{ sweeping.of }}
          </span>
          <span class="spacer" />
          <!-- Only where the panes cannot answer it. A file with regions is
               answered by the buttons below, which say the same thing without a
               menu; one with none — a side deleted it, or it is binary — has
               nothing to pick through and this is the only way to say what
               should happen to it. -->
          <button
            v-if="wholeFile"
            class="btn tiny ghosty"
            title="Answer for the whole file at once"
            @click="forFile"
          >
            Whole file
            <ChevronDown :size="11" />
          </button>
          <!-- The whole merge, for when reading it file by file is not what
               today is for. -->
          <button
            class="btn tiny ghosty"
            :title="`Answer every conflicted file at once`"
            @click="everyFile"
          >
            <Layers :size="12" />
            All {{ files.length }} {{ files.length === 1 ? 'file' : 'files' }}
            <ChevronDown :size="11" />
          </button>
          <button
            v-if="wholeFile"
            class="btn btn-primary tiny"
            :disabled="store.busy"
            title="Stage the file exactly as it is on disk"
            @click="keepAsIs"
          >
            Keep as it is
          </button>
          <button v-else class="btn btn-primary tiny" :disabled="store.busy" @click="markResolved">
            Mark resolved
          </button>
        </div>

        <!-- Where you are, and what to do about it. The buttons here answer the
             whole file at once, in a fixed place on screen; picking a region
             apart is done in the panes, on the region itself. A button is lit
             only while every region agrees with it. -->
        <div v-if="!wholeFile" class="guide">
          <span class="walk">
            <button
              class="step"
              :disabled="active <= 0"
              title="Previous conflict (Up)"
              @click="step(-1)"
            >
              <ChevronsUp :size="13" />
            </button>
            <span class="place">
              Conflict <strong>{{ active + 1 }}</strong> of {{ conflicts.length }}
            </span>
            <button
              class="step"
              :disabled="active >= conflicts.length - 1"
              title="Next conflict (Down)"
              @click="step(1)"
            >
              <ChevronsDown :size="13" />
            </button>
          </span>
          <button v-if="left" class="btn tiny ghosty" title="Jump to the next one nobody has decided (Tab)" @click="nextOpen">
            Next undecided
          </button>

          <span class="sep" />

          <span class="seg-group" role="group" aria-label="What to keep in every conflict">
            <button
              class="seg ours"
              :class="{ on: asked === 'ours' }"
              title="Keep our side of every conflict in this file"
              @click="takeAll('ours')"
            >
              Ours
            </button>
            <button
              class="seg theirs"
              :class="{ on: asked === 'theirs' }"
              title="Keep their side of every conflict in this file"
              @click="takeAll('theirs')"
            >
              Theirs
            </button>
            <button
              class="seg both"
              :class="{ on: asked === 'both' }"
              title="Keep both sides of every conflict, one after the other"
              @click="takeAll('both')"
            >
              Both
            </button>
            <button
              class="seg none"
              :class="{ on: asked === 'none' }"
              title="Drop every conflicted region from the file"
              @click="takeAll('none')"
            >
              Neither
            </button>
          </span>
          <span v-if="stance(active) === 'mixed'" class="chip mixed">picked line by line</span>
          <span v-else-if="stance(active) === 'edited'" class="chip edited-chip">AI or hand edit</span>
          <!-- The whole file, like every button to its left. It used to do the
               one region on the click and hide the file behind a chevron, which
               made it the only control on this row that meant something
               narrower than the others. Per-region is still where the region
               itself is, on its own AI button. -->
          <button
            v-if="ai.configured.value"
            class="btn tiny ai"
            :disabled="thinking !== null || sweeping !== null"
            title="Ask the model to resolve every conflict in this file"
            @click="aiResolveAll"
          >
            <Spinner v-if="thinking !== null || sweeping?.file === path" :size="12" />
            <Sparkles v-else :size="12" />
            Resolve with AI
          </button>

          <span class="spacer" />

          <label v-if="hasBase" class="tiny check" title="Show what both sides started from">
            <input v-model="showBase" type="checkbox" />
            Base
          </label>
        </div>

        <p v-if="explanation" class="explain">{{ explanation }}</p>

        <div class="panes">
          <div v-for="pane in panes" :key="pane.side" class="pane" :class="`side-${pane.side}`">
            <div class="pane-head">
              <span class="tag">{{ pane.label }}</span>
              <span class="mono faint truncate">{{ pane.branch }}</span>
            </div>

            <div
              v-if="wholeFile && pane.side !== 'base' && !stages?.[pane.side]"
              class="pane-body gone"
            >
              This file is not on this side — it was deleted.
            </div>

            <div
              v-else
              :ref="(element) => setBody(pane.side, element)"
              class="pane-body"
            >
              <!-- Not painted, and no height of its own; it is here to be
                   measured, so the pane's width does not change under the
                   pointer as rows scroll in and out of the drawn window. -->
              <div class="row gauge" aria-hidden="true">
                <span class="num">{{ grid.rows.length }}</span>
                <code>{{ widest[pane.side] }}</code>
              </div>

              <div
                class="rows"
                :style="{ paddingTop: `${padTop}px`, paddingBottom: `${padBottom}px` }"
              >
                <template v-for="(row, at) in visible" :key="shown.first + at">
                  <!-- The bar above a region, and the only place its whole side
                       can be taken or dropped in one go. -->
                  <div
                    v-if="row.kind === 'head'"
                    class="head"
                    :class="{
                      now: row.conflict === active,
                      off: pane.side !== 'base' && !sideOn(row.conflict, pane.side) && !sideSome(row.conflict, pane.side)
                    }"
                    @click="active = row.conflict"
                  >
                    <template v-if="pane.side === 'base'">
                      <span class="head-label faint">Before either change</span>
                      <span class="faint count">{{ countOf(row.conflict, 'base') }}</span>
                    </template>
                    <template v-else>
                      <input
                        class="box"
                        type="checkbox"
                        :checked="sideOn(row.conflict, pane.side)"
                        :indeterminate="sideSome(row.conflict, pane.side)"
                        :title="`Take all of ${pane.side} here`"
                        @click.stop
                        @change="toggleSide(row.conflict, pane.side)"
                      />
                      <span class="head-label">
                        {{ pane.side === 'ours' ? 'Take ours' : 'Take theirs' }}
                      </span>
                      <span class="count">
                        {{ countOf(row.conflict, pane.side) }}
                        {{ countOf(row.conflict, pane.side) === 1 ? 'line' : 'lines' }}
                      </span>
                      <span class="head-space" />
                      <button
                        v-if="pickAt(row.conflict)?.custom && pane.side === 'ours'"
                        class="pill-btn"
                        title="Drop the edit and go back to picking lines"
                        @click.stop="undoEdit(row.conflict)"
                      >
                        <Undo2 :size="10" /> edit
                      </button>
                      <button
                        v-else-if="stance(row.conflict) === 'both' || stance(row.conflict) === 'mixed'"
                        class="pill-btn"
                        title="Swap the order the two sides are written in"
                        @click.stop="swapOrder(row.conflict)"
                      >
                        {{
                          (pickAt(row.conflict)?.ours_first ?? true) === (pane.side === 'ours')
                            ? 'first'
                            : 'second'
                        }}
                      </button>
                      <button
                        v-if="pane.side === 'ours' && ai.configured.value"
                        class="pill-btn ai"
                        :disabled="thinking !== null"
                        title="Ask the model to merge these two sides"
                        @click.stop="aiResolve(row.conflict)"
                      >
                        <Spinner v-if="thinking === row.conflict" :size="10" />
                        <Sparkles v-else :size="10" />
                      </button>
                    </template>
                  </div>

                  <!-- A line of a region, with its own checkbox: a conflict is
                       often two edits to the same block where the answer is some
                       of each, and picking whole sides cannot say that. -->
                  <div
                    v-else-if="row.conflict >= 0"
                    class="row line"
                    :class="{
                      now: row.conflict === active,
                      on: lineOn(row.conflict, pane.side, atOf(row, pane.side)),
                      off:
                        pane.side !== 'base' &&
                        !!cellOf(row, pane.side) &&
                        !lineOn(row.conflict, pane.side, atOf(row, pane.side)),
                      filler: !cellOf(row, pane.side)
                    }"
                  >
                    <span class="num">
                      <input
                        v-if="pane.side !== 'base' && cellOf(row, pane.side)"
                        class="box line-box"
                        type="checkbox"
                        :checked="lineOn(row.conflict, pane.side, atOf(row, pane.side))"
                        :disabled="!!pickAt(row.conflict)?.custom"
                        :title="`Take this line from ${pane.side}`"
                        @change="toggleLine(row.conflict, pane.side, atOf(row, pane.side))"
                      />
                      <span class="no">{{ cellOf(row, pane.side)?.num ?? '' }}</span>
                    </span>
                    <code v-if="cellOf(row, pane.side)" v-html="cellOf(row, pane.side)!.html || ' '" />
                    <code v-else class="hatch" />
                  </div>

                  <!-- Context: what both sides already agree on. -->
                  <div v-else class="row ctx">
                    <span class="num"><span class="no">{{ cellOf(row, pane.side)?.num ?? '' }}</span></span>
                    <code v-html="cellOf(row, pane.side)?.html || ' '" />
                  </div>
                </template>
              </div>
            </div>
          </div>

          <!-- Where the conflicts are in the file as a whole: amber for a region
               nobody has looked at, green once it has been decided, red where it
               is set to be dropped. -->
          <ChangeRuler
            v-if="!wholeFile"
            :container="rulerBox"
            :marks="marks"
            :active="active"
            hint="Where the conflicts are — click to go there"
          />
        </div>

        <!-- Exactly what will be written to disk, kept alongside the panes and as
             tall as the reader wants it — up to a share of the window, so the
             sides never get squeezed away entirely. -->
        <ResizeHandle v-if="showResult" side="result" />
        <div
          class="output"
          :class="{ folded: !showResult }"
          :style="showResult ? { height: `min(${layout.result}px, 70%)` } : undefined"
        >
          <button class="out-head" @click="showResult = !showResult">
            <component :is="showResult ? ChevronDown : ChevronRight" :size="12" />
            <span class="tag">Result</span>
            <span class="faint">what gets written</span>
            <span v-if="resultRows.length" class="faint count">{{ resultRows.length }} lines</span>
            <span v-if="picks.some((pick) => pick.custom)" class="edited">
              includes AI or hand edits
            </span>
          </button>
          <div v-if="showResult" class="out-pane">
            <div ref="outBody" class="pane-body out-body" @scroll="onResultScroll">
              <div class="row gauge" aria-hidden="true">
                <span class="num">{{ resultRows.length }}</span>
                <code>{{ outWidest }}</code>
              </div>
              <div
                class="rows"
                :style="{
                  paddingTop: `${outShown.first * ROW}px`,
                  paddingBottom: `${(resultRows.length - outShown.last) * ROW}px`
                }"
              >
                <!-- A line that answers a conflict carries the colour of the
                     side it came from, so the resolution can be read out of the
                     file rather than counted against the panes. -->
                <div
                  v-for="row in outVisible"
                  :key="row.num"
                  class="row"
                  :class="[
                    row.origin ? `from-${row.origin.from}` : '',
                    { now: row.origin?.conflict === active }
                  ]"
                >
                  <span class="num"><span class="no">{{ row.num }}</span></span>
                  <code v-html="row.html || ' '" />
                </div>
              </div>
            </div>
            <ChangeRuler
              :container="outBody"
              :marks="outMarks"
              hint="Where the resolved lines ended up — click to go there"
            />
          </div>
        </div>
      </div>
    </template>

    <!-- One question before a run that costs money and rewrites every file in
         the merge, since neither of those is undone by closing the page. -->
    <AppModal
      v-if="askingSweep"
      title="Ask the model to resolve the whole merge?"
      :width="420"
      @close="askingSweep = false"
    >
      <p class="ask">
        Every conflict in all {{ files.length }} {{ files.length === 1 ? 'file' : 'files' }} goes to
        the model, one call each, and each file is written and staged as its answers come back.
      </p>
      <p class="ask dim">
        A merge is exactly where a model is most likely to be confidently wrong. Read what it wrote
        before you commit it.
      </p>
      <template #footer>
        <button class="btn btn-ghost" @click="askingSweep = false">Cancel</button>
        <button class="btn btn-primary" @click="aiEveryFile">Ask the model</button>
      </template>
    </AppModal>
  </section>
</template>

<style scoped>
/* A grid row is `auto` by default, which means "as tall as what is in it" —
   so the panes grew to the height of the file and pushed the result pane off
   the bottom of the window instead of scrolling. Every box between here and a
   pane's own scrollbar has to be allowed to be shorter than its contents. */
.conflicts {
  display: grid;
  grid-template-columns: 210px minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr);
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.clear {
  grid-column: 1 / -1;
  display: grid;
  place-items: center;
  text-align: center;
}

.clear h3 {
  margin: 0 0 6px;
}

.clear p {
  margin: 0;
  max-width: 340px;
  font-size: 12px;
}

.rail {
  border-right: 1px solid var(--line);
  background: var(--bg-panel);
  overflow-y: auto;
}

.rail-file {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 10px;
  text-align: left;
  font-size: 12px;
}

.rail-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.rail-file .dot {
  flex: none;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--amber);
}

.rail-file:hover {
  background: var(--bg-hover);
}

.rail-file.on {
  background: var(--bg-active);
}

.rail-file .small {
  font-size: 10px;
  max-width: 100%;
}

.work {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.toolbar,
.guide {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: none;
  padding: 6px 10px;
  border-bottom: 1px solid var(--line);
  background: var(--bg-panel);
}

.guide {
  background: var(--bg-raised);
}

.file-name {
  max-width: 260px;
  color: var(--text);
}

.chip {
  padding: 1px 7px;
  border: 1px solid var(--line);
  border-radius: 9px;
  font-size: 10.5px;
  color: var(--text-dim);
  white-space: nowrap;
}

.chip.mixed {
  color: var(--amber-soft);
  border-color: color-mix(in srgb, var(--amber) 45%, transparent);
}

.chip.edited-chip {
  color: var(--purple);
  border-color: color-mix(in srgb, var(--purple) 45%, transparent);
}

/* How much of the file has been answered, which is the one thing a count of
   conflicts cannot say. */
.progress {
  width: 90px;
  height: 4px;
  border-radius: 3px;
  background: var(--line);
  overflow: hidden;
}

.progress .fill {
  display: block;
  height: 100%;
  background: var(--green);
  transition: width 0.15s ease-out;
}

.stat {
  font-size: 11px;
  color: var(--text-faint);
  white-space: nowrap;
}

.warn {
  color: var(--amber);
}

.spacer {
  flex: 1;
}

.sep {
  width: 1px;
  height: 16px;
  background: var(--line);
  margin: 0 3px;
}

.tiny {
  font-size: 11px;
  padding: 3px 7px;
}

.ghosty {
  border: 1px solid var(--line);
}

.check {
  display: flex;
  align-items: center;
  gap: 5px;
  color: var(--text-dim);
  cursor: pointer;
}

.ai {
  color: var(--purple);
}

/* --- walking the conflicts */
.walk {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 1px 3px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--bg-panel);
}

.step {
  display: grid;
  place-items: center;
  padding: 2px 4px;
  border-radius: 4px;
  color: var(--text-dim);
}

.step:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
}

.step:disabled {
  opacity: 0.35;
}

.place {
  padding: 0 4px;
  font-size: 11px;
  color: var(--text-dim);
  white-space: nowrap;
}

.place strong {
  color: var(--text);
}

/* The one answer for the region being worked on: four states, one of them on. */
.seg-group {
  display: flex;
  border: 1px solid var(--line);
  border-radius: 6px;
  overflow: hidden;
}

.seg {
  padding: 3px 9px;
  font-size: 11px;
  color: var(--text-faint);
  border-right: 1px solid var(--line);
}

.seg:last-child {
  border-right: none;
}

.seg:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.seg.ours.on {
  background: color-mix(in srgb, var(--accent) 26%, transparent);
  color: var(--accent-soft);
}

.seg.theirs.on {
  background: color-mix(in srgb, var(--purple) 26%, transparent);
  color: var(--purple-soft);
}

.seg.both.on {
  background: color-mix(in srgb, var(--green) 24%, transparent);
  color: var(--green-soft);
}

.seg.none.on {
  background: color-mix(in srgb, var(--red) 24%, transparent);
  color: var(--red-soft);
}

/* The one thing the panes cannot say, said above them. */
.explain {
  margin: 0;
  flex: none;
  padding: 8px 12px;
  font-size: 12px;
  line-height: 1.55;
  color: var(--text-dim);
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
}

.gone {
  padding: 14px 12px;
  font-size: 12px;
  font-style: italic;
  color: var(--text-faint);
}

/* --- the panes */
.panes {
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  border-bottom: 1px solid var(--line);
}

.pane {
  display: grid;
  /* Stated, so a long conflicted line scrolls rather than widening the pane. */
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: auto minmax(0, 1fr);
  flex: 1 1 0;
  min-width: 0;
  border-right: 1px solid var(--line);
}

.pane-head {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 4px 10px;
  font-size: 11px;
  border-bottom: 1px solid var(--line-soft);
  background: var(--bg-raised);
}

.tag {
  font-weight: 600;
  letter-spacing: 0.05em;
  text-transform: uppercase;
}

/* Each side keeps one colour throughout — heading, chunk edge, taken lines —
   so which pane you are looking at is never a matter of reading the label. */
.side-ours .tag {
  color: var(--accent-soft);
}

.side-theirs .tag {
  color: var(--purple-soft);
}

.side-base .tag {
  color: var(--text-dim);
}

.side-ours {
  --side: var(--accent);
}

.side-theirs {
  --side: var(--purple);
}

.side-base {
  --side: var(--text-faint);
}

.pane-body {
  overflow: auto;
  min-height: 0;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.5;
}

.rows {
  min-width: max-content;
}

/* Laid out and not painted: it is here to hold the pane's width still. */
.gauge {
  height: 0;
  visibility: hidden;
  overflow: hidden;
}

.row {
  display: flex;
  align-items: flex-start;
  height: 18px;
  tab-size: 4;
}

/* The gutter: a line's own checkbox, then its number. The padding lines the
   checkbox up with the one on the region's head bar rather than leaving it
   pressed against the pane's edge. */
.num {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
  width: 44px;
  height: 18px;
  padding: 0 8px 0 6px;
  color: var(--text-faint);
  user-select: none;
}

.no {
  flex: 1;
  text-align: right;
  opacity: 0.6;
}

.row code {
  flex: 1;
  min-width: 0;
  padding-right: 10px;
  white-space: pre;
  font: inherit;
}

.ctx code {
  color: var(--text-dim);
}

/* --- a region */
.head {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 22px;
  padding: 0 8px 0 6px;
  font-family: var(--font);
  font-size: 10.5px;
  cursor: pointer;
  user-select: none;
  background: color-mix(in srgb, var(--side) 18%, var(--bg-raised));
  border-top: 1px solid color-mix(in srgb, var(--side) 55%, transparent);
  border-left: 3px solid var(--side);
  color: var(--text);
}

.head.off {
  background: var(--bg-raised);
  border-left-color: var(--line);
  color: var(--text-faint);
}

.head-label {
  font-weight: 600;
}

.head .count {
  color: var(--text-faint);
}

.head-space {
  flex: 1;
}

/* The region being worked on, marked in every pane at once. */
.head.now {
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--side) 65%, transparent);
  background: color-mix(in srgb, var(--side) 30%, var(--bg-raised));
  color: var(--text);
}

.line {
  border-left: 3px solid color-mix(in srgb, var(--side) 40%, transparent);
}

.line.now {
  border-left-color: var(--side);
}

/* Taken lines carry their side's colour; dropped ones fade out of the way but
   stay readable, because what you did not take is still worth checking. */
.line.on {
  background: color-mix(in srgb, var(--side) 12%, transparent);
}

.line.off {
  opacity: 0.4;
  background: transparent;
}

.line.off .no {
  text-decoration: line-through;
}

/* Where one side simply has fewer lines than the other. Drawn rather than
   left blank: the gap is what tells you the sides are still lined up. */
.filler code.hatch {
  height: 18px;
  background-image: repeating-linear-gradient(
    135deg,
    color-mix(in srgb, var(--text-faint) 22%, transparent) 0 1px,
    transparent 1px 6px
  );
}

.ctx {
  border-left: 3px solid transparent;
}

.box {
  flex: none;
  width: 11px;
  height: 11px;
  margin: 0;
  accent-color: var(--side);
  cursor: pointer;
}

.line-box {
  opacity: 0.45;
}

.line.on .line-box,
.line:hover .line-box,
.line.now .line-box {
  opacity: 1;
}

.pill-btn {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 0 5px;
  border: 1px solid var(--line);
  border-radius: 8px;
  font-size: 9.5px;
  color: var(--text-dim);
}

.pill-btn:hover:not(:disabled) {
  color: var(--text);
  border-color: var(--text-faint);
}

.pill-btn.ai {
  color: var(--purple);
  border-color: color-mix(in srgb, var(--purple) 45%, transparent);
}

.pill-btn:disabled {
  opacity: 0.5;
}

/* --- the result
   It keeps a third of the height and folds away to its heading, which is the
   whole point of it sitting at the bottom: it is there to be glanced at while
   the choices above it are being made. */
.output {
  display: flex;
  flex-direction: column;
  flex: none;
  min-height: 0;
}

.output.folded {
  height: auto;
}

.out-pane {
  display: flex;
  min-height: 0;
  flex: 1;
}

.out-head {
  display: flex;
  align-items: center;
  gap: 7px;
  width: 100%;
  padding: 4px 10px;
  text-align: left;
  font-size: 11px;
  background: var(--bg-raised);
  border-bottom: 1px solid var(--line-soft);
  cursor: pointer;
}

.out-head:hover {
  background: var(--bg-active);
}

.out-head .tag {
  color: var(--green);
}

.out-head .count {
  font-weight: 400;
}

.edited {
  margin-left: auto;
  font-size: 10.5px;
  color: var(--purple);
}

.out-body {
  flex: 1;
  min-width: 0;
  padding: 4px 0;
}

/* The result's own three colours: our side, their side, and something neither
   of them wrote. Context is left alone — it is the thing being read past. */
.out-body .row {
  border-left: 3px solid transparent;
}

.ask {
  margin: 0 0 10px;
  font-size: 12.5px;
  line-height: 1.55;
}

.chip.busy {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--accent);
}

.out-body .from-ours {
  border-left-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}

.out-body .from-theirs {
  border-left-color: var(--purple);
  background: color-mix(in srgb, var(--purple) 12%, transparent);
}

.out-body .from-edit {
  border-left-color: var(--purple-soft);
  background: color-mix(in srgb, var(--purple-soft) 14%, transparent);
}

.out-body .from-ours .no,
.out-body .from-theirs .no,
.out-body .from-edit .no {
  opacity: 1;
  color: var(--text-dim);
}

/* The region being worked on, so the panes above and the file below agree
   about which conflict is under discussion. */
.out-body .row.now {
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--text) 18%, transparent);
}
</style>
