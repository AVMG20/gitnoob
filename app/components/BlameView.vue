<script setup lang="ts">
import { computed } from 'vue'
import { relativeTime, useGit, type BlameRun, type FileDiff } from '~/composables/useGit'
import { highlightWhole, languageFor } from '~/composables/useHighlight'
import { CODE_ROW, windowOf } from '~/composables/useCode'
import { tint } from '~/composables/useAvatars'

/**
 * The file with the commit that last touched each line beside it.
 *
 * Drawn as runs rather than as lines: consecutive lines that came in together
 * share one chip, which is what makes the column readable — four hundred
 * identical rows say nothing, a dozen chips say when the file was built.
 */
const props = defineProps<{
  diff: FileDiff | null
  /** The whole file as it stands on the side being shown. */
  text: string | null
  runs: BlameRun[]
  loading?: boolean
  error?: string | null
  /** Where the box that scrolls this is scrolled to, and how tall it is. */
  top?: number
  view?: number
}>()

const git = useGit()

const language = computed(() => (props.diff ? languageFor(props.diff.path) : null))
const source = computed(() => (props.text === null ? [] : props.text.split('\n')))

const painted = computed(() =>
  props.text === null ? [] : highlightWhole(props.text, language.value)
)

/** The run each line belongs to, so a window into the middle still knows. */
const runOf = computed(() => {
  const map: BlameRun[] = []
  for (const run of props.runs) {
    for (let at = 0; at < run.lines; at++) map[run.start + at - 1] = run
  }
  return map
})

interface Row {
  number: number
  html: string
  run: BlameRun | undefined
  /** True on the first line of a run: the only row that draws the chip. */
  heads: boolean
}

const rows = computed<Row[]>(() =>
  source.value.map((_, at) => {
    const run = runOf.value[at]
    return {
      number: at + 1,
      html: painted.value[at] ?? '',
      run,
      heads: !!run && run.start === at + 1
    }
  })
)

// --- only what is on screen
const shown = computed(() => windowOf(rows.value.length, props.top ?? 0, props.view ?? 0))
const visible = computed(() => rows.value.slice(shown.value.first, shown.value.last))

/** Holds the view open sideways while only a screenful of rows is drawn. */
const longest = computed(() => {
  let found = ''
  for (const text of source.value) if (text.length > found.length) found = text
  return found
})

/**
 * How old a run is, on a scale of nothing to one.
 *
 * The oldest line in the file is the far end of the scale rather than some
 * fixed number of years: a file written last month and a file written in 2014
 * both want their earliest lines to read as the earliest, and a fixed scale
 * would paint one of them entirely one colour.
 */
const ages = computed(() => {
  const times = props.runs.filter((run) => !run.uncommitted).map((run) => run.time)
  const newest = Math.max(...times, 0)
  const oldest = Math.min(...times, newest)
  const span = newest - oldest
  return { oldest, span }
})

function ageOf(run: BlameRun | undefined) {
  if (!run || run.uncommitted) return 1
  const { oldest, span } = ages.value
  return span > 0 ? (run.time - oldest) / span : 1
}

/** Newer lines stand out; older ones fade into the page. */
function chipStyle(run: BlameRun | undefined) {
  const age = ageOf(run)
  return { opacity: `${0.45 + age * 0.55}` }
}

function faceStyle(run: BlameRun | undefined) {
  return { background: run?.uncommitted ? 'var(--text-faint)' : tint(run?.email || run?.author || '') }
}

function when(run: BlameRun | undefined) {
  if (!run) return ''
  return run.uncommitted ? 'now' : relativeTime(run.time)
}

function title(run: BlameRun | undefined) {
  if (!run) return ''
  if (run.uncommitted) return 'Not committed yet — this line is your own working copy.'
  return [
    run.summary,
    `${run.author} <${run.email}>`,
    relativeTime(run.time),
    run.oid,
    'Click to open this commit.'
  ].join('\n')
}

/** A chip is a way into the commit that put the line there. */
function openCommit(run: BlameRun | undefined) {
  if (!run || run.uncommitted) return
  git.revealCommit(run.oid)
}

const ROW = CODE_ROW
</script>

<template>
  <div class="blame">
    <p v-if="props.loading" class="note dim">Reading who touched what…</p>
    <p v-else-if="props.error" class="note dim">{{ props.error }}</p>
    <p v-else-if="props.diff?.binary" class="note dim">Binary file — nothing to read.</p>
    <p v-else-if="props.text === null" class="note dim">Select a file.</p>
    <p v-else-if="!props.runs.length" class="note dim">
      Nothing to blame — this file has no history yet.
    </p>

    <template v-else>
      <!-- Not painted, and no height of its own; it is here to be measured. -->
      <div class="line gauge" aria-hidden="true">
        <span class="chip"><span class="who">MMMMMMMMMMMM</span></span>
        <span class="no">{{ rows.length }}</span>
        <span class="text">{{ longest }}</span>
      </div>

      <div
        class="lines"
        :style="{
          paddingTop: `${shown.first * ROW}px`,
          paddingBottom: `${(rows.length - shown.last) * ROW}px`
        }"
      >
        <div
          v-for="row in visible"
          :key="row.number"
          class="line"
          :class="{ heads: row.heads, mine: row.run?.uncommitted }"
        >
          <!-- One chip per run, on the line the run starts at. The rest of the
               run carries the rule down its left instead, which is what ties
               the lines to the chip above them. -->
          <button
            v-if="row.heads"
            class="chip"
            :style="chipStyle(row.run)"
            :title="title(row.run)"
            :disabled="row.run?.uncommitted"
            @click="openCommit(row.run)"
          >
            <span class="face" :style="faceStyle(row.run)" />
            <span class="who truncate">{{ row.run?.uncommitted ? 'Uncommitted' : row.run?.author }}</span>
            <span class="ago">{{ when(row.run) }}</span>
          </button>
          <span v-else class="chip rule" />

          <span class="no">{{ row.number }}</span>
          <span class="text" v-html="row.html || ' '" />
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.blame {
  min-width: min-content;
  padding: 6px 0 40px;
  font-family: var(--mono);
  font-size: 12px;
}

.note {
  padding: 12px 14px;
  font-family: var(--font);
  font-size: 12px;
}

.line {
  display: flex;
  align-items: stretch;
  height: v-bind('`${ROW}px`');
  line-height: v-bind('`${ROW}px`');
  white-space: pre;
}

.gauge {
  height: 0;
  overflow: hidden;
  visibility: hidden;
}

/* The blame column: wide enough for a name and a date, and no wider — the
   code is what the eye should land on. */
.chip {
  display: flex;
  align-items: center;
  gap: 5px;
  flex: none;
  width: 186px;
  padding: 0 8px 0 6px;
  font-family: var(--font);
  font-size: 11px;
  color: var(--text-dim);
  text-align: left;
  border-right: 1px solid var(--line-soft);
  /* The rule that ties a run together, drawn in the same place the chip's own
     left edge would be. */
  box-shadow: inset 2px 0 0 var(--line-soft);
}

.chip.rule {
  cursor: default;
}

.line.heads .chip {
  box-shadow: inset 2px 0 0 var(--line);
}

.line.heads:hover .chip,
.line.mine .chip {
  box-shadow: inset 2px 0 0 var(--accent);
}

button.chip:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
}

button.chip:disabled {
  cursor: default;
}

.face {
  flex: none;
  width: 8px;
  height: 8px;
  border-radius: 50%;
}

.who {
  flex: 1;
  min-width: 0;
}

.ago {
  flex: none;
  font-size: 10px;
  color: var(--text-faint);
}

.no {
  flex: none;
  width: 52px;
  padding-right: 12px;
  text-align: right;
  color: var(--text-faint);
  user-select: none;
}

.text {
  flex: 1;
  padding-right: 24px;
}

/* Lines you have not committed: marked, because they are the ones blame
   cannot really answer for. */
.line.mine .text {
  background: color-mix(in srgb, var(--accent) 7%, transparent);
}
</style>
