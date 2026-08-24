<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import {
  ArrowDownToLine,
  Check,
  ChevronDown,
  ChevronUp,
  Cloud,
  Copy,
  FileText,
  GitBranchPlus,
  GitCommitHorizontal,
  MonitorDot,
  Search,
  Tag,
  Undo2,
  X
} from 'lucide-vue-next'
import {
  WIP,
  copyText,
  highlight,
  laneColor,
  relativeTime,
  rowMatches,
  useGit,
  type GraphRow,
  type ResetMode,
  type Segment
} from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useDragDrop } from '~/composables/useDragDrop'

const git = useGit()
const store = git.store
const menu = useContextMenu()
const drag = useDragDrop()

const ROW = 27
const LANE = 14
const OVERSCAN = 12
const MAX_LANES = 14

const viewport = ref<HTMLElement | null>(null)
const searchBox = ref<HTMLInputElement | null>(null)
const searchOpen = ref(false)
const scrollTop = ref(0)
const height = ref(600)
const hit = ref(0)
const resetTarget = ref<{ oid: string; mode: ResetMode } | null>(null)
const tagTarget = ref<GraphRow | null>(null)
const branchTarget = ref<GraphRow | null>(null)

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
const lanes = computed(() =>
  Math.min(MAX_LANES, Math.max(2, ...store.rows.map((row) => row.width)))
)
const graphWidth = computed(() => lanes.value * LANE + 8)

const matches = computed(() =>
  store.query.trim() ? store.rows.filter((row) => rowMatches(row, store.query)) : []
)
const matchIds = computed(() => new Set(matches.value.map((row) => row.oid)))

const first = computed(() => Math.max(0, Math.floor(scrollTop.value / ROW) - OVERSCAN))
const last = computed(() =>
  Math.min(total.value, Math.ceil((scrollTop.value + height.value) / ROW) + OVERSCAN)
)
const window_ = computed(() =>
  store.rows
    .slice(first.value, last.value)
    .map((row, i) => ({ row, top: (first.value + i) * ROW }))
)

const dirty = computed(
  () => (store.status?.staged.length ?? 0) + (store.status?.unstaged.length ?? 0)
)
const conflicts = computed(() => store.status?.conflicted.length ?? 0)
/** The WIP node sits on whichever lane the newest commit is on. */
const headLane = computed(() => store.rows[0]?.lane ?? 0)
const headColor = computed(() => store.rows[0]?.color ?? 0)

const x = (lane: number) => Math.min(lane, MAX_LANES - 1) * LANE + LANE / 2
const y = (level: number) => (level === 0 ? 0 : level === 1 ? ROW / 2 : ROW)

function path(segment: Segment) {
  const x1 = x(segment.x1)
  const x2 = x(segment.x2)
  const y1 = y(segment.y1)
  const y2 = y(segment.y2)
  if (x1 === x2) return `M${x1},${y1} L${x2},${y2}`
  const mid = (y1 + y2) / 2
  return `M${x1},${y1} C${x1},${mid} ${x2},${mid} ${x2},${y2}`
}

function onScroll() {
  if (viewport.value) scrollTop.value = viewport.value.scrollTop
}

function measure() {
  if (viewport.value) height.value = viewport.value.clientHeight
}

function scrollTo(oid: string) {
  const index = store.rows.findIndex((row) => row.oid === oid)
  if (index < 0 || !viewport.value) return
  const target = index * ROW - height.value / 2 + ROW
  viewport.value.scrollTo({ top: Math.max(0, target), behavior: 'smooth' })
}

function step(by: number) {
  if (!matches.value.length) return
  hit.value = (hit.value + by + matches.value.length) % matches.value.length
  const row = matches.value[hit.value]
  git.select(row.oid)
  scrollTo(row.oid)
}

watch(
  () => store.query,
  async () => {
    hit.value = 0
    if (matches.value.length) {
      await nextTick()
      scrollTo(matches.value[0].oid)
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

function onKey(event: KeyboardEvent) {
  if ((event.metaKey || event.ctrlKey) && event.key === 'f') {
    event.preventDefault()
    openSearch()
  }
  if (event.key === 'Escape' && searchOpen.value) {
    closeSearch()
  }
  if ((event.metaKey || event.ctrlKey) && event.key === 'g') {
    event.preventDefault()
    step(event.shiftKey ? -1 : 1)
  }
}

/** Opens the reset dialog with a mode already chosen. */
function openReset(oid: string, mode: ResetMode) {
  resetTarget.value = { oid, mode }
}

// --- ref chips

/** One label in the branch column, after folding and ordering. */
interface RefChip {
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
 * Order puts the checked-out branch first, then the other locals, then
 * remote-only branches, then tags — which is also the order of how likely you
 * are to be looking for it.
 */
function refChips(row: GraphRow): RefChip[] {
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
    remotes: []
  }))

  return [
    ...fromLocals.filter((c) => c.head),
    ...fromLocals.filter((c) => !c.head),
    ...orphanRemotes,
    ...others
  ]
}

/** What one chip is, spelled out, for a tooltip or a menu row. */
function describeChip(chip: RefChip): string {
  const where = chip.remotes.length ? ` — also on ${chip.remotes.join(', ')}` : ''
  if (chip.head) return `${chip.name} — checked out${where}`
  return `${chip.name}${where}`
}

/** The refs a commit carries beyond the one on show. */
function hiddenRefs(row: GraphRow) {
  return refChips(row).slice(1)
}

/** Lists every ref on a commit, each one checkout-able. */
function refsMenu(event: MouseEvent, row: GraphRow) {
  const chips = refChips(row)
  menu.show(
    event,
    chips.map((chip) => ({
      label: chip.name,
      icon: chip.kind === 'tag' ? Tag : chip.kind === 'remote' ? Cloud : MonitorDot,
      hint: chip.head ? 'checked out' : chip.remotes.length ? 'pushed' : '',
      disabled: chip.head,
      action: () => git.checkout(chip.name)
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
  const isHead = row.labels.some((label) => label.kind === 'local' && label.name === store.repo?.head)
  const picked = subjects(row)
  menu.show(
    event,
    [
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

/** Dropping a branch on a commit moves that branch there. */
function onDropOnRow(row: GraphRow) {
  const payload = drag.take(['branch'])
  if (!payload || payload.kind !== 'branch' || payload.remote) return
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
})
</script>

<template>
  <section class="graph">
    <!-- Summoned with ⌘F rather than always on screen: it is wanted for
         seconds at a time and costs a row of the window the whole session. -->
    <div v-if="searchOpen" class="head">
      <span class="search">
        <Search :size="13" class="faint" />
        <input
          ref="searchBox"
          v-model="store.query"
          type="search"
          placeholder="Search messages, authors, hashes"
        />
        <template v-if="store.query.trim()">
          <span class="count" :class="{ none: !matches.length }">
            {{ matches.length ? `${hit + 1} of ${matches.length}` : 'no matches' }}
          </span>
          <button class="step" :disabled="!matches.length" title="Previous (⇧⌘G)" @click="step(-1)">
            <ChevronUp :size="13" />
          </button>
          <button class="step" :disabled="!matches.length" title="Next (⌘G)" @click="step(1)">
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
    <div class="colhead">
      <span class="col-refs">Branch / tag</span>
      <span class="cell-head" :style="{ width: `${graphWidth}px` }">Graph</span>
      <span class="col-msg">Commit message</span>
      <!-- Lives here rather than beside the search box, which is not always on
           screen to hold it. -->
      <button v-if="marked.length > 1" class="marks" @click="clearMarks()">
        {{ marked.length }} selected
        <X :size="11" />
      </button>
      <span class="col-author">Author</span>
      <span class="col-date">Date</span>
    </div>

    <!-- The working tree, always the top row and selected by default. -->
    <div
      class="wip"
      :class="{ on: store.selected === WIP }"
      @click="git.select(WIP)"
      @contextmenu="wipMenu($event)"
    >
      <span class="col-refs" />
      <svg class="cell" :width="graphWidth" :height="ROW" :viewBox="`0 0 ${graphWidth} ${ROW}`">
        <path
          v-if="store.rows.length"
          :d="`M${x(headLane)},${ROW / 2} L${x(headLane)},${ROW}`"
          :stroke="laneColor(headColor)"
          stroke-width="1.6"
          stroke-dasharray="2 2"
          fill="none"
        />
        <circle
          :cx="x(headLane)"
          :cy="ROW / 2"
          r="4"
          fill="var(--bg)"
          :stroke="dirty || conflicts ? 'var(--amber)' : 'var(--text-faint)'"
          stroke-width="1.8"
          :stroke-dasharray="dirty || conflicts ? '' : '2 2'"
        />
      </svg>
      <span class="col-msg">
        <span v-if="conflicts" class="chip chip-conflict">{{ conflicts }} conflicted</span>
        <span v-else-if="dirty" class="chip chip-wip">uncommitted</span>
        <span class="summary truncate" :class="{ quiet: !dirty && !conflicts }">
          <template v-if="conflicts">Resolve conflicts before committing</template>
          <template v-else-if="dirty">
            {{ dirty }} {{ dirty === 1 ? 'change' : 'changes' }} in your working tree
          </template>
          <template v-else>No local changes</template>
        </span>
      </span>
      <!-- Whoever a commit made here would be authored by, rather than "you":
           with a profile per context, which identity is in force is the thing
           worth showing. -->
      <span class="col-author faint truncate">{{ store.repo?.author || 'no author set' }}</span>
      <span class="col-date faint">now</span>
    </div>

    <div ref="viewport" class="viewport" @scroll.passive="onScroll">
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
          @dragstart="
            drag.begin($event, {
              kind: 'commit',
              oid: item.row.oid,
              short: item.row.short,
              summary: item.row.summary
            })
          "
          @dragend="drag.end()"
          @dragover="drag.hover($event, `commit:${item.row.oid}`, ['branch'])"
          @dragleave="drag.leave(`commit:${item.row.oid}`)"
          @drop.prevent="onDropOnRow(item.row)"
        >
          <!-- Refs live in their own column with a line running to the node,
               so a tip is found by scanning one narrow strip rather than by
               reading the start of every message. -->
          <span class="col-refs">
            <!-- Only the first chip is drawn; the rest live behind a counter
                 that lists them, so a commit with five refs takes the same
                 width as one with a single branch. -->
            <button
              v-if="hiddenRefs(item.row).length"
              class="more-refs"
              :title="hiddenRefs(item.row).map(describeChip).join('\n')"
              @click.stop="refsMenu($event, item.row)"
            >
              +{{ hiddenRefs(item.row).length }}
            </button>
            <span
              v-for="chip in refChips(item.row).slice(0, 1)"
              :key="chip.key"
              class="chip"
              :class="[`chip-${chip.kind}`, { 'chip-current': chip.head }]"
              :title="describeChip(chip)"
            >
              <Check v-if="chip.head" :size="11" :stroke-width="3" class="glyph" />
              <span class="truncate">{{ chip.name }}</span>
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
          </span>

          <svg
            class="cell"
            :width="graphWidth"
            :height="ROW"
            :viewBox="`0 0 ${graphWidth} ${ROW}`"
          >
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
              stroke-width="1.6"
            />
            <!-- A commit the upstream does not have yet wears a ring. Colour
                 alone will not do it: the first lane is already the accent
                 colour, so the boundary between what the remote has and what is
                 still only here has to be a difference in shape. -->
            <circle
              v-if="item.row.unpushed"
              :cx="x(item.row.lane)"
              :cy="ROW / 2"
              r="6.5"
              fill="none"
              :stroke="laneColor(item.row.color)"
              stroke-width="1.5"
              opacity="0.55"
            />
            <circle
              :cx="x(item.row.lane)"
              :cy="ROW / 2"
              :r="item.row.parents.length > 1 ? 3.4 : 4"
              :fill="item.row.parents.length > 1 ? 'var(--bg)' : laneColor(item.row.color)"
              :stroke="laneColor(item.row.color)"
              stroke-width="1.8"
            >
              <title v-if="item.row.unpushed">Not pushed yet</title>
            </circle>
          </svg>

          <span class="col-msg">
            <span class="summary truncate">
              <span
                v-for="(part, i) in highlight(item.row.summary, store.query)"
                :key="i"
                :class="{ mark: part.hit }"
                >{{ part.text }}</span
              >
            </span>
          </span>
          <span class="col-author truncate">{{ item.row.author }}</span>
          <span class="col-date faint" :title="new Date(item.row.time * 1000).toLocaleString()">
            {{ relativeTime(item.row.time) }}
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

    <ResetDialog
      v-if="resetTarget"
      :oid="resetTarget.oid"
      :mode="resetTarget.mode"
      @close="resetTarget = null"
    />
    <TagDialog v-if="tagTarget" :row="tagTarget" @close="tagTarget = null" />
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
  background: var(--bg);
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
  background: rgba(79, 156, 249, 0.16);
}

.wip {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: none;
  height: 27px;
  padding: 0 12px 0 8px;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
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
  background: rgba(79, 156, 249, 0.13);
  box-shadow: inset 2px 0 0 var(--accent);
}

.row.marked.on {
  background: rgba(79, 156, 249, 0.2);
}

.row.dim {
  opacity: 0.36;
}

.row.hit {
  background: rgba(240, 168, 60, 0.08);
}

.row.drop {
  outline: 1px solid var(--accent);
  outline-offset: -1px;
  background: rgba(79, 156, 249, 0.14);
}

.cell {
  flex: none;
  display: block;
}

/* The branch strip. Right-aligned so every chip ends against the graph and the
   leader lines all start from the same edge; when a commit carries more refs
   than fit, the ones nearest the graph win, which are the ones the line is
   pointing at. */
.col-refs {
  flex: none;
  width: 124px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 4px;
  overflow: hidden;
  /* Cancels the row's gap on this side alone, so the leader line — which starts
     at the left edge of the graph cell — begins against the chip instead of
     after a gap it cannot reach across. */
  margin-right: -10px;
  padding-right: 0;
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
}

/* The heading reads left to right; only the chips below it hug the graph. */
.colhead .col-refs {
  justify-content: flex-start;
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

.summary.quiet {
  color: var(--text-faint);
}

.mark {
  background: rgba(240, 168, 60, 0.32);
  border-radius: 2px;
  color: #fff;
}

.col-author {
  width: 130px;
  flex: none;
  color: var(--text-dim);
  font-size: 12px;
}

.col-date {
  width: 88px;
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
}

/* The branch you are on: brighter, outlined, and ticked. Everything else on
   the same commit stays flat, so the eye lands on this one. */
.chip-current {
  background: rgba(79, 156, 249, 0.32);
  color: #cfe4ff;
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
  background: rgba(79, 156, 249, 0.18);
  color: #8dc0fb;
}

.chip-remote {
  background: rgba(169, 123, 240, 0.16);
  color: #c4a4f6;
}

.chip-tag {
  background: rgba(240, 168, 60, 0.16);
  color: #f2bd6e;
}

.chip-head {
  background: rgba(87, 193, 132, 0.18);
  color: #7ed3a5;
}

.chip-wip {
  background: rgba(240, 168, 60, 0.16);
  color: #f2bd6e;
}

.chip-conflict {
  background: rgba(224, 87, 109, 0.2);
  color: #ef8d9c;
}

.more,
.empty {
  display: flex;
  justify-content: center;
  padding: 14px;
}
</style>
