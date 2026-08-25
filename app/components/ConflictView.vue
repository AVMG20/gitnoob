<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ChevronDown, ChevronRight, Sparkles } from 'lucide-vue-next'
import { useGit, type ConflictBlock, type ConflictFile, type Resolution } from '~/composables/useGit'
import { useAi } from '~/composables/useAi'
import { highlightWhole, languageFor } from '~/composables/useHighlight'

const git = useGit()
const store = git.store
const ai = useAi()

/** Which conflict region the model is currently working on. */
const thinking = ref<number | null>(null)

const path = ref<string | null>(null)
const file = ref<ConflictFile | null>(null)
const choices = ref<Resolution[]>([])
const result = ref('')
const showBase = ref(false)
const loading = ref(false)

const files = computed(() => store.status?.conflicted ?? [])
const conflicts = computed(
  () =>
    (file.value?.blocks ?? []).filter(
      (block): block is Extract<ConflictBlock, { kind: 'conflict' }> => block.kind === 'conflict'
    )
)
const hasBase = computed(() => conflicts.value.some((block) => block.has_base))
/** Regions where the user has turned both sides off, which deletes them. */
const dropped = computed(
  () => choices.value.filter((choice) => !choice.take_ours && !choice.take_theirs).length
)
const oursLabel = computed(() => conflicts.value[0]?.ours_label || 'ours')
const theirsLabel = computed(() => conflicts.value[0]?.theirs_label || 'theirs')

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

/** One line as it is drawn: its number down the side, its code coloured. */
interface Row {
  num: number
  html: string
}

/** A run of lines that belong together: either untouched context, or one side
    of one conflict, which carries the index its checkbox acts on. */
interface Segment {
  kind: 'context' | 'chunk'
  index: number
  rows: Row[]
}

/**
 * One side of the file, coloured and numbered.
 *
 * The whole side is highlighted in one pass rather than a line at a time,
 * because the things a line cannot know about itself — an open block comment, a
 * string that runs on, the `<script>` block that turns a `.vue` file into
 * JavaScript — are exactly the ones a merge lands in the middle of.
 *
 * The numbers are that side's own: the context and this side's chunks are
 * precisely the file as it stands on that branch, so counting them off gives
 * the line number you would find there.
 */
function sideOf(side: 'ours' | 'theirs' | 'base'): Segment[] {
  const blocks = file.value?.blocks ?? []
  const segments: Segment[] = []
  const source: string[] = []

  for (const block of blocks) {
    const lines = block.kind === 'context' ? block.lines : block[side]
    segments.push({
      kind: block.kind === 'context' ? 'context' : 'chunk',
      index: block.kind === 'context' ? -1 : block.index,
      rows: lines.map((line) => ({ num: 0, html: line }))
    })
    source.push(...lines)
  }

  const coloured = highlightWhole(source.join('\n'), language.value)
  let at = 0
  let number = 0
  for (const segment of segments) {
    for (const row of segment.rows) {
      row.num = ++number
      row.html = coloured[at++] ?? ''
    }
  }
  return segments
}

const ourSide = computed(() => sideOf('ours'))
const theirSide = computed(() => sideOf('theirs'))
const baseSide = computed(() => (showBase.value && hasBase.value ? sideOf('base') : []))

/** The result pane, coloured and numbered the same way. */
const resultRows = computed<Row[]>(() => {
  if (!result.value) return []
  const lines = result.value.replace(/\n$/, '').split('\n')
  const coloured = highlightWhole(lines.join('\n'), language.value)
  return lines.map((_, i) => ({ num: i + 1, html: coloured[i] ?? '' }))
})

/** The result pane folds away for anyone who would rather have the height. */
const showResult = ref(true)

async function keepAsIs() {
  if (!path.value) return
  await git.conflictResolveAsIs(path.value)
  const next = store.status?.conflicted.find((p) => p !== path.value)
  if (next) await load(next)
  else {
    path.value = null
    file.value = null
    store.resolving = null
  }
}

async function load(target: string) {
  path.value = target
  // The overlay is driven by this, so keep the two in step.
  store.resolving = target
  loading.value = true
  file.value = await git.conflictRead(target)
  // Start from "keep ours", the same default as reading the file top to bottom.
  choices.value = Array.from({ length: file.value?.conflict_count ?? 0 }, () => ({
    take_ours: true,
    take_theirs: false,
    ours_first: true,
    custom: null
  }))
  loading.value = false
  await preview()
}

async function preview() {
  if (!path.value) return
  result.value = (await git.conflictPreview(path.value, choices.value)) ?? ''
}

function set(index: number, patch: Partial<Resolution>) {
  const next = { ...choices.value[index], ...patch }
  choices.value = choices.value.map((choice, i) => (i === index ? next : choice))
}

function takeAll(side: 'ours' | 'theirs' | 'both') {
  choices.value = choices.value.map((choice) => ({
    ...choice,
    take_ours: side !== 'theirs',
    take_theirs: side !== 'ours'
  }))
}

async function markResolved() {
  if (!path.value) return
  const target = path.value
  await git.conflictResolve(target, choices.value)
  // Move on to whatever is still conflicted, or clear the view when done.
  const next = (store.status?.conflicted ?? []).find((p) => p !== target)
  if (next) await load(next)
  else {
    path.value = null
    file.value = null
    result.value = ''
    // Nothing left to resolve, so close the resolver.
    store.resolving = null
  }
}

async function takeWholeFile(side: 'ours' | 'theirs') {
  if (!path.value) return
  await git.conflictResolveWhole(path.value, side)
  const next = store.status?.conflicted[0]
  if (next) await load(next)
  else {
    path.value = null
    file.value = null
  }
}

/**
 * Asks the model for one region and stores its answer as a hand edit, so it
 * shows up in the result pane like any other choice and can still be overridden.
 */
async function aiResolve(index: number) {
  if (!path.value) return
  thinking.value = index
  try {
    const lines = await ai.resolveConflict(path.value, index)
    if (lines) {
      set(index, { custom: lines, take_ours: true, take_theirs: true })
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

watch(choices, preview, { deep: true })

// Open the first conflicted file as soon as there is one.
watch(
  files,
  (list) => {
    if (!path.value && list.length) load(list[0])
    else if (path.value && !list.includes(path.value)) {
      path.value = list[0] ?? null
      if (path.value) load(path.value)
      else {
        file.value = null
        result.value = ''
      }
    }
  },
  { immediate: true }
)
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
          <span class="truncate">{{ name.split('/').pop() }}</span>
          <span class="faint truncate small">{{ name }}</span>
        </button>
      </div>

      <div class="work">
        <div class="toolbar">
          <span class="file-name mono truncate">{{ path }}</span>
          <span class="stat">
            <template v-if="wholeFile">whole file</template>
            <template v-else>
              {{ conflicts.length }} {{ conflicts.length === 1 ? 'conflict' : 'conflicts' }}
              <template v-if="dropped">
                · <span class="warn">{{ dropped }} set to be dropped</span>
              </template>
            </template>
          </span>
          <span class="spacer" />
          <template v-if="!wholeFile">
            <button class="btn tiny" @click="takeAll('ours')">All ours</button>
            <button class="btn tiny" @click="takeAll('theirs')">All theirs</button>
            <button class="btn tiny" @click="takeAll('both')">All both</button>
          </template>
          <button
            v-if="!wholeFile && ai.configured.value"
            class="btn tiny ai"
            :disabled="thinking !== null"
            title="Ask the model to resolve every region in this file"
            @click="aiResolveAll"
          >
            <Spinner v-if="thinking !== null" :size="12" />
            <Sparkles v-else :size="12" />
            AI resolve all
          </button>
          <label v-if="hasBase" class="tiny check">
            <input v-model="showBase" type="checkbox" />
            Base
          </label>
          <span class="sep" />
          <button class="btn tiny" :disabled="store.busy" @click="takeWholeFile('ours')">
            {{ wholeFile && !stages?.ours ? 'Leave it deleted' : 'Whole file: ours' }}
          </button>
          <button class="btn tiny" :disabled="store.busy" @click="takeWholeFile('theirs')">
            {{ wholeFile && !stages?.theirs ? 'Let it go' : 'Whole file: theirs' }}
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
          <button
            v-else
            class="btn btn-primary tiny"
            :disabled="store.busy"
            @click="markResolved"
          >
            Mark resolved
          </button>
        </div>

        <p v-if="explanation" class="explain">{{ explanation }}</p>

        <div class="panes" :class="{ 'with-base': showBase && hasBase }">
          <!-- Pane 1: our side. -->
          <div class="pane">
            <div class="pane-head ours">
              Ours <span class="mono faint">{{ oursLabel }}</span>
            </div>
            <div class="pane-body">
              <div v-if="wholeFile && !stages?.ours" class="gone">
                This file is not on this side — it was deleted.
              </div>
              <template
                v-for="(segment, si) in wholeFile && !stages?.ours ? [] : ourSide"
                :key="`o${si}`"
              >
                <div v-if="segment.kind === 'context'" class="ctx">
                  <div v-for="row in segment.rows" :key="row.num" class="line">
                    <span class="num">{{ row.num }}</span>
                    <code v-html="row.html || ' '" />
                  </div>
                </div>
                <div v-else class="chunk" :class="{ off: !choices[segment.index]?.take_ours }">
                  <label class="chunk-head">
                    <input
                      type="checkbox"
                      :checked="choices[segment.index]?.take_ours"
                      @change="set(segment.index, { take_ours: ($event.target as HTMLInputElement).checked })"
                    />
                    Take ours
                    <button
                      v-if="choices[segment.index]?.take_ours && choices[segment.index]?.take_theirs && !choices[segment.index]?.custom"
                      class="order"
                      title="Swap the order the two sides are written in"
                      @click.prevent="set(segment.index, { ours_first: !choices[segment.index].ours_first })"
                    >
                      {{ choices[segment.index].ours_first ? 'first' : 'second' }}
                    </button>
                    <button
                      v-if="ai.configured.value"
                      class="order ai"
                      :disabled="thinking !== null"
                      title="Ask the model to merge these two sides"
                      @click.prevent="aiResolve(segment.index)"
                    >
                      <Spinner v-if="thinking === segment.index" :size="10" />
                      <Sparkles v-else :size="10" />
                      AI
                    </button>
                    <button
                      v-if="choices[segment.index]?.custom"
                      class="order"
                      title="Drop the edit and go back to the checkboxes"
                      @click.prevent="set(segment.index, { custom: null })"
                    >
                      undo edit
                    </button>
                  </label>
                  <div v-for="row in segment.rows" :key="row.num" class="line ours-line">
                    <span class="num">{{ row.num }}</span>
                    <code v-html="row.html || ' '" />
                  </div>
                  <div v-if="!segment.rows.length" class="line empty faint">
                    <span class="num" />
                    <code>(nothing on this side)</code>
                  </div>
                </div>
              </template>
            </div>
          </div>

          <!-- Optional middle pane: the merge base, when git wrote diff3 markers. -->
          <div v-if="showBase && hasBase" class="pane">
            <div class="pane-head base">Base <span class="faint">merge base</span></div>
            <div class="pane-body">
              <template v-for="(segment, si) in baseSide" :key="`b${si}`">
                <div v-if="segment.kind === 'context'" class="ctx">
                  <div v-for="row in segment.rows" :key="row.num" class="line">
                    <span class="num">{{ row.num }}</span>
                    <code v-html="row.html || ' '" />
                  </div>
                </div>
                <div v-else class="chunk neutral">
                  <div class="chunk-head faint">Before either change</div>
                  <div v-for="row in segment.rows" :key="row.num" class="line">
                    <span class="num">{{ row.num }}</span>
                    <code v-html="row.html || ' '" />
                  </div>
                  <div v-if="!segment.rows.length" class="line empty faint">
                    <span class="num" />
                    <code>(added on both sides)</code>
                  </div>
                </div>
              </template>
            </div>
          </div>

          <!-- Pane 2: their side. -->
          <div class="pane">
            <div class="pane-head theirs">
              Theirs <span class="mono faint">{{ theirsLabel }}</span>
            </div>
            <div class="pane-body">
              <div v-if="wholeFile && !stages?.theirs" class="gone">
                This file is not on this side — it was deleted.
              </div>
              <template
                v-for="(segment, si) in wholeFile && !stages?.theirs ? [] : theirSide"
                :key="`t${si}`"
              >
                <div v-if="segment.kind === 'context'" class="ctx">
                  <div v-for="row in segment.rows" :key="row.num" class="line">
                    <span class="num">{{ row.num }}</span>
                    <code v-html="row.html || ' '" />
                  </div>
                </div>
                <div v-else class="chunk" :class="{ off: !choices[segment.index]?.take_theirs }">
                  <label class="chunk-head">
                    <input
                      type="checkbox"
                      :checked="choices[segment.index]?.take_theirs"
                      @change="set(segment.index, { take_theirs: ($event.target as HTMLInputElement).checked })"
                    />
                    Take theirs
                    <button
                      v-if="choices[segment.index]?.take_ours && choices[segment.index]?.take_theirs"
                      class="order"
                      title="Swap the order the two sides are written in"
                      @click.prevent="set(segment.index, { ours_first: !choices[segment.index].ours_first })"
                    >
                      {{ choices[segment.index].ours_first ? 'second' : 'first' }}
                    </button>
                  </label>
                  <div v-for="row in segment.rows" :key="row.num" class="line theirs-line">
                    <span class="num">{{ row.num }}</span>
                    <code v-html="row.html || ' '" />
                  </div>
                  <div v-if="!segment.rows.length" class="line empty faint">
                    <span class="num" />
                    <code>(nothing on this side)</code>
                  </div>
                </div>
              </template>
            </div>
          </div>
        </div>

        <!-- Pane 3: exactly what will be written to disk. -->
        <div class="output" :class="{ folded: !showResult }">
          <button class="pane-head result" @click="showResult = !showResult">
            <component :is="showResult ? ChevronDown : ChevronRight" :size="12" />
            Result <span class="faint">what gets written</span>
            <span v-if="resultRows.length" class="faint count">
              {{ resultRows.length }} lines
            </span>
            <span v-if="choices.some((c) => c.custom)" class="edited">includes AI or hand edits</span>
          </button>
          <div v-if="showResult" class="pane-body out-body">
            <div v-for="row in resultRows" :key="row.num" class="line">
              <span class="num">{{ row.num }}</span>
              <code v-html="row.html || ' '" />
            </div>
          </div>
        </div>
      </div>
    </template>
  </section>
</template>

<style scoped>
/* The one thing the panes cannot say, said above them. */
.explain {
  margin: 0;
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
  flex-direction: column;
  gap: 1px;
  align-items: flex-start;
  padding: 5px 10px;
  text-align: left;
  font-size: 12px;
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

/* Laid out as a column rather than fixed rows: the explanation above the panes
   comes and goes with the file, and a grid with three rows and four children
   silently gave the fourth a row of its own — which is what pushed the result
   pane out of the window. */
.work {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--line);
  background: var(--bg-panel);
}

.file-name {
  max-width: 280px;
  color: var(--text-dim);
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

.check {
  display: flex;
  align-items: center;
  gap: 5px;
  color: var(--text-dim);
  cursor: pointer;
}

.toolbar,
.explain {
  flex: none;
}

.panes {
  display: grid;
  grid-template-columns: 1fr 1fr;
  /* The row is stated for the same reason the columns are. Left implicit it is
     `auto`, which means as tall as the file — so each side grew to its full
     length, never scrolled, and hung down over the result pane instead. */
  grid-template-rows: minmax(0, 1fr);
  flex: 1;
  min-height: 0;
  overflow: hidden;
  border-bottom: 1px solid var(--line);
}

.panes.with-base {
  grid-template-columns: 1fr 1fr 1fr;
}

.pane {
  display: grid;
  /* Stated, so a long conflicted line scrolls rather than widening the pane. */
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: auto minmax(0, 1fr);
  min-width: 0;
  border-right: 1px solid var(--line);
}

.pane:last-child {
  border-right: none;
}

.pane-head {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 4px 10px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  border-bottom: 1px solid var(--line-soft);
  background: var(--bg-raised);
}

.pane-head.ours {
  color: var(--accent-soft);
}

.pane-head.theirs {
  color: var(--purple-soft);
}

.pane-head.base {
  color: var(--text-dim);
}

.pane-head.result {
  color: var(--green);
}

.pane-head .faint {
  text-transform: none;
  letter-spacing: 0;
  font-weight: 400;
}

.pane-body {
  overflow: auto;
  min-height: 0;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.5;
}

/* The number sits in a column of its own so the code starts at the same place
   on both sides, and it is not selectable: copying a side out of the resolver
   should give code, not code with a number welded to every line. */
.line {
  display: flex;
  align-items: flex-start;
  tab-size: 4;
}

.num {
  flex: none;
  width: 44px;
  padding-right: 10px;
  text-align: right;
  color: var(--text-faint);
  opacity: 0.6;
  user-select: none;
}

.line code {
  flex: 1;
  min-width: 0;
  padding-right: 10px;
  white-space: pre;
  font: inherit;
}

.ctx .line code {
  color: var(--text-dim);
}

.line.empty code {
  font-style: italic;
}

.chunk {
  margin: 3px 0;
  border-top: 1px solid var(--line);
  border-bottom: 1px solid var(--line);
}

.chunk.off {
  opacity: 0.38;
}

.chunk-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 10px;
  font-family: var(--font);
  font-size: 11px;
  background: var(--bg-raised);
  cursor: pointer;
  user-select: none;
}

.chunk.neutral .chunk-head {
  cursor: default;
}

.order {
  margin-left: auto;
  padding: 0 6px;
  border: 1px solid var(--line);
  border-radius: 8px;
  font-size: 10px;
  color: var(--text-dim);
}

.order:hover:not(:disabled) {
  color: var(--text);
  border-color: var(--text-faint);
}

.order.ai,
.tiny.ai {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  color: var(--purple);
  border-color: rgba(169, 123, 240, 0.45);
}

.order:disabled {
  opacity: 0.5;
}

.ours-line {
  background: color-mix(in srgb, var(--accent) 10%, transparent);
}

.theirs-line {
  background: rgba(169, 123, 240, 0.1);
}

/* The result keeps a third of the height and folds away to its heading, which
   is the whole point of it sitting at the bottom: it is there to be glanced at
   while the choices above it are being made. */
.output {
  display: flex;
  flex-direction: column;
  flex: 0 1 34%;
  min-height: 0;
}

.output.folded {
  flex: none;
}

.pane-head.result {
  width: 100%;
  text-align: left;
  cursor: pointer;
}

.pane-head.result:hover {
  background: var(--bg-active);
}

.count {
  text-transform: none;
  letter-spacing: 0;
  font-weight: 400;
}

.edited {
  margin-left: auto;
  text-transform: none;
  letter-spacing: 0;
  font-weight: 400;
  font-size: 10.5px;
  color: var(--purple);
}

.out-body {
  padding: 4px 0;
}
</style>
