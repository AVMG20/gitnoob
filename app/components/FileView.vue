<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Users } from 'lucide-vue-next'
import { relativeTime, useGit, type BlameRun, type FileDiff } from '~/composables/useGit'
import { highlightWhole, languageFor } from '~/composables/useHighlight'
import { CODE_ROW, markedLines, windowOf, type Line } from '~/composables/useCode'
import { useContextMenu } from '~/composables/useContextMenu'
import { tint } from '~/composables/useAvatars'

const props = defineProps<{
  diff: FileDiff | null
  /** The whole file as it stands on the side being shown. */
  text: string | null
  /** The file has been deleted, so what is shown is the copy git still has. */
  gone?: boolean
  loading?: boolean
  error?: string | null
  /** Where the box that scrolls this is scrolled to, and how tall it is. */
  top?: number
  view?: number
  /** Who last touched each line, drawn beside the numbers when it is asked for. */
  runs?: BlameRun[]
  blame?: boolean
  blameLoading?: boolean
  blameError?: string | null
}>()

const emit = defineEmits<{ (event: 'toggle-blame'): void }>()

const git = useGit()
const menu = useContextMenu()

const language = computed(() => (props.diff ? languageFor(props.diff.path) : null))

const lines = computed(() => markedLines(props.text, props.diff?.hunks ?? []))

const counts = computed(() => ({
  marked: lines.value.filter((line) => line.mark).length,
  gaps: lines.value.filter((line) => line.removed.length).length
}))

// --- what it was before
//
// The marks are the only record in this view of what the file used to say, so
// they answer for it: clicking one shows the lines it stands in for. The panel
// is anchored to the mark rather than to the pointer, the way an editor does
// it, so the old text lands beside the new and the two can be read together.
const open = ref<{ line: number; kind: 'was' | 'gone' } | null>(null)

const isOpen = (line: Line, kind: 'was' | 'gone') =>
  open.value?.line === line.number && open.value.kind === kind

function show(line: Line, kind: 'was' | 'gone') {
  open.value = isOpen(line, kind) ? null : { line: line.number, kind }
}

/** The old lines, coloured as a piece so a block comment reads as one. */
function paintOld(text: string[]) {
  return highlightWhole(text.join('\n'), language.value)
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape' && open.value) open.value = null
}

/** Anywhere but the panel and the mark that opened it closes it. */
function onDown(event: MouseEvent) {
  const target = event.target as HTMLElement | null
  if (target?.closest('.gutter, .before')) return
  open.value = null
}

onMounted(() => {
  window.addEventListener('keydown', onKey)
  window.addEventListener('mousedown', onDown)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKey)
  window.removeEventListener('mousedown', onDown)
})

/**
 * The file coloured in one pass, one entry per line.
 *
 * Whole rather than line by line, which is what the diff view has to settle
 * for: with the file in hand, a block comment reads as a comment throughout and
 * the script inside a single-file component reads as script.
 */
const painted = computed(() =>
  props.text === null ? [] : highlightWhole(props.text, language.value)
)

const paint = (at: number) => painted.value[at - 1] ?? ''

// --- only what is on screen
const shown = computed(() => windowOf(lines.value.length, props.top ?? 0, props.view ?? 0))
const visible = computed(() => lines.value.slice(shown.value.first, shown.value.last))

/**
 * A hidden copy of the longest line, which is what holds the view open
 * sideways.
 *
 * A row is as wide as its own content, so with only a screenful of them drawn
 * the width of the whole view would be the width of whatever happened to be on
 * screen — and it would change under the pointer while scrolling, taking the
 * horizontal scrollbar with it. One row carrying the longest line, laid out and
 * not painted, keeps it still. Measured in characters because the font is
 * monospace, so the longest line is a matter of counting rather than of asking
 * the engine.
 */
const source = computed(() => (props.text === null ? [] : props.text.split('\n')))

const longest = computed(() => {
  let found = ''
  for (const text of source.value) if (text.length > found.length) found = text
  return found
})

// --- who touched what
//
// A column rather than a view of its own: the file is the thing being read
// either way, and the question "who wrote this" is asked about a line you are
// already looking at. Turned on from the numbers it sits against, and the
// answer is remembered for every file after.
const runs = computed(() => props.runs ?? [])

/** The run each line belongs to, so a window into the middle still knows. */
const runOf = computed(() => {
  const map: BlameRun[] = []
  for (const run of runs.value) {
    for (let at = 0; at < run.lines; at++) map[run.start + at - 1] = run
  }
  return map
})

const runAt = (number: number) => runOf.value[number - 1]

/** True on the first line of a run: the only row that draws the chip. */
const heads = (number: number) => {
  const run = runAt(number)
  return !!run && run.start === number
}

/**
 * How old a run is, on a scale of nothing to one.
 *
 * The oldest line in the file is the far end of the scale rather than some
 * fixed number of years: a file written last month and a file written in 2014
 * both want their earliest lines to read as the earliest, and a fixed scale
 * would paint one of them entirely one colour.
 */
const ages = computed(() => {
  const times = runs.value.filter((run) => !run.uncommitted).map((run) => run.time)
  const newest = Math.max(...times, 0)
  const oldest = Math.min(...times, newest)
  return { oldest, span: newest - oldest }
})

function ageOf(run: BlameRun | undefined) {
  if (!run || run.uncommitted) return 1
  const { oldest, span } = ages.value
  return span > 0 ? (run.time - oldest) / span : 1
}

/** Newer lines stand out; older ones fade into the page. */
function chipStyle(run: BlameRun | undefined) {
  return { opacity: `${0.45 + ageOf(run) * 0.55}` }
}

function faceStyle(run: BlameRun | undefined) {
  return {
    background: run?.uncommitted ? 'var(--text-faint)' : tint(run?.email || run?.author || '')
  }
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

/** The numbers answer for the column beside them: right-click turns it on. */
function onNumbers(event: MouseEvent) {
  menu.show(event, [
    {
      label: props.blame ? 'Hide blame' : 'Show blame',
      icon: Users,
      hint: 'Who last touched each line',
      action: () => emit('toggle-blame')
    }
  ])
}

const ROW = CODE_ROW
</script>

<template>
  <div class="file" :class="{ waiting: props.blame && props.blameLoading }">
    <p v-if="props.loading" class="note dim">Loading file…</p>
    <p v-else-if="props.error" class="note dim">{{ props.error }}</p>
    <p v-else-if="props.diff?.binary" class="note dim">Binary file — nothing to read.</p>
    <p v-else-if="props.text === null" class="note dim">Select a file.</p>

    <template v-else>
      <!-- A deletion has no file left to show, so this is the copy git kept.
           Said plainly, because a page of code for a file that is not there
           any more reads as a bug otherwise. -->
      <p v-if="props.gone" class="note dim">
        Deleted — this is the copy git still has, as it was before it went.
      </p>
      <p v-else-if="!counts.marked && !counts.gaps" class="note dim">
        Nothing changed in this file — it is shown as it stands.
      </p>
      <!-- Blame is a column of this view, so when it cannot be read the file
           is still here to read; it says so and stays out of the way. -->
      <p v-if="props.blame && props.blameError" class="note dim">{{ props.blameError }}</p>
      <!-- Not painted, and no height of its own; it is here to be measured. -->
      <div class="line gauge" aria-hidden="true">
        <span v-if="props.blame" class="chip"><span class="who">MMMMMMMMMMMM</span></span>
        <span class="no">{{ lines.length }}</span>
        <span class="gutter" />
        <span class="text">{{ longest }}</span>
      </div>

      <div
        class="lines"
        :style="{
          paddingTop: `${shown.first * ROW}px`,
          paddingBottom: `${(lines.length - shown.last) * ROW}px`
        }"
      >
      <div v-for="line in visible" :key="line.number" class="line" :class="line.mark ?? ''">
        <!-- One chip per run, on the line the run starts at. The rest of the
             run carries the rule down its left instead, which is what ties the
             lines to the chip above them. -->
        <template v-if="props.blame">
          <button
            v-if="heads(line.number)"
            class="chip"
            :style="chipStyle(runAt(line.number))"
            :title="title(runAt(line.number))"
            :disabled="runAt(line.number)?.uncommitted"
            @click="openCommit(runAt(line.number))"
          >
            <span class="face" :style="faceStyle(runAt(line.number))" />
            <span class="who truncate">{{
              runAt(line.number)?.uncommitted ? 'Uncommitted' : runAt(line.number)?.author
            }}</span>
            <span class="ago">{{ when(runAt(line.number)) }}</span>
          </button>
          <span v-else class="chip rule" />
        </template>
        <span class="no" @contextmenu="onNumbers">{{ line.number }}</span>
        <!-- The bar an editor draws between the numbers and the code: solid
             where a line is new or changed, and a wedge where lines were taken
             out and nothing put in their place, since a removal has no line of
             its own to colour. Both answer a click with what used to be there;
             a line that is new has nothing to answer with, so it stays a mark. -->
        <span
          class="gutter"
          :class="{ live: line.was.length, shown: isOpen(line, 'was') }"
          :title="line.was.length ? 'Click to see what this line said before' : ''"
          @click="line.was.length && show(line, 'was')"
        >
          <span
            v-if="line.removed.length"
            class="gone"
            :class="{ shown: isOpen(line, 'gone') }"
            :title="`${line.removed.length} ${
              line.removed.length === 1 ? 'line' : 'lines'
            } deleted here — click to read them`"
            @click.stop="show(line, 'gone')"
          />

          <!-- The deleted lines sit above the one that took their place, which
               is where they were. -->
          <span v-if="isOpen(line, 'gone')" class="before gone-at">
            <span class="before-head">
              {{ line.removed.length }} deleted
              {{ line.removed.length === 1 ? 'line' : 'lines' }}
            </span>
            <span class="before-body">
              <span
                v-for="(html, at) in paintOld(line.removed)"
                :key="at"
                class="before-line"
                v-html="html || ' '"
              />
            </span>
          </span>

          <span v-if="isOpen(line, 'was')" class="before was-at">
            <span class="before-head">
              {{ line.was.length === 1 ? 'Was' : `Was, ${line.was.length} lines` }}
            </span>
            <span class="before-body">
              <span
                v-for="(html, at) in paintOld(line.was)"
                :key="at"
                class="before-line"
                v-html="html || ' '"
              />
            </span>
          </span>
        </span>
        <span class="text" v-html="paint(line.number)" />
      </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.file {
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.5;
}

.note {
  font-family: var(--font);
  padding: 12px;
}

.line {
  display: flex;
  align-items: flex-start;
  white-space: pre;
  height: 18px;
}

/* Wide enough for the longest line in the file and tall enough for none of it,
   so the view keeps one width whichever rows are drawn in it. */
.gauge {
  width: max-content;
  height: 0;
  overflow: hidden;
  visibility: hidden;
  pointer-events: none;
}

/* The blame column: wide enough for a name and a date, and no wider — the code
   is what the eye should land on. It sits outside the numbers, so turning it on
   pushes nothing about the file around but its left edge. */
.chip {
  display: flex;
  align-items: center;
  gap: 5px;
  flex: none;
  width: 178px;
  height: 18px;
  padding: 0 8px 0 6px;
  font-family: var(--font);
  font-size: 11px;
  line-height: 18px;
  color: var(--text-dim);
  text-align: left;
  border-right: 1px solid var(--line-soft);
  /* The rule that ties a run together, drawn where the chip's own left edge
     would be. */
  box-shadow: inset 2px 0 0 var(--line-soft);
}

button.chip {
  box-shadow: inset 2px 0 0 var(--line);
}

button.chip:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
  box-shadow: inset 2px 0 0 var(--accent);
}

button.chip:disabled {
  cursor: default;
  box-shadow: inset 2px 0 0 var(--accent);
}

.chip.rule {
  cursor: default;
}

/* While the history is still being walked there are no chips yet, only the
   rules between them; faded, so the column reads as an answer on its way
   rather than as an answer of nothing. */
.file.waiting .chip {
  opacity: 0.4;
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

/* Only when the blame column stands to its left do the numbers need air on
   that side; without it they keep the width they always had. */
.chip + .no {
  padding-left: 9px;
}

/* Between the numbers and the code, where an editor puts it: the mark belongs
   to the line it sits against, not to the edge of the window. */
.gutter {
  position: relative;
  flex: none;
  width: 3px;
  align-self: stretch;
  margin-right: 8px;
}

/* Three pixels is a mark, not a target. The bar keeps its width and takes its
   clicks from a few pixels either side of it, which costs no layout. */
.gutter.live {
  cursor: pointer;
}

.gutter.live::before {
  content: '';
  position: absolute;
  inset: 0 -4px;
}

.gutter.live:hover,
.gutter.shown {
  filter: brightness(1.35);
}

.line.added .gutter {
  background: var(--green);
}

/* A changed line is not a new one, and colouring both the same makes a rewrite
   look like a fresh file. */
.line.changed .gutter {
  background: var(--accent);
}

/* Sits on the seam between two lines rather than beside one of them. */
.gone {
  position: absolute;
  left: 0;
  top: -2px;
  width: 100%;
  height: 4px;
  background: var(--text-faint);
  border-radius: 1px;
  cursor: pointer;
  /* Above the change bar's own hit area, so the seam keeps its clicks. */
  z-index: 2;
}

.gone::after {
  /* The clickable part, wider than the wedge and invisible. */
  content: '';
  position: absolute;
  inset: -3px -4px;
}

.gone:hover,
.gone.shown {
  background: var(--text-dim);
}

/* What used to be there. Anchored to the mark, drawn over the code to its
   right, and never taller than a third of the window — anything longer scrolls
   inside itself rather than pushing the file around. */
.before {
  position: absolute;
  left: 11px;
  z-index: 6;
  display: block;
  min-width: 260px;
  max-width: 62vw;
  max-height: 34vh;
  overflow: auto;
  border: 1px solid var(--line);
  border-left: 3px solid var(--red-soft);
  border-radius: 4px;
  background: var(--bg-raised);
  box-shadow: 0 6px 20px var(--shadow);
  cursor: auto;
}

/* The old text lands where it belongs against the new: what a line used to say
   sits directly under the line that says it now, and lines that were taken out
   sit above the line they used to sit above. Neither covers the code it is
   there to be compared with. */
.was-at {
  top: calc(100% - 2px);
}

.gone-at {
  bottom: calc(100% - 2px);
}

.before-head {
  display: block;
  position: sticky;
  top: 0;
  padding: 2px 8px;
  font-family: var(--font);
  font-size: 10px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
  background: var(--bg-raised);
  border-bottom: 1px solid var(--line);
}

.before-body {
  display: block;
  padding: 3px 0;
}

.before-line {
  display: block;
  padding: 0 10px;
  white-space: pre;
}

.no {
  flex: none;
  width: 46px;
  padding-right: 9px;
  text-align: right;
  color: var(--text-faint);
  user-select: none;
  cursor: context-menu;
}

/* Never wrapped. Every row is exactly one line tall, because that is what the
   list scrolling this counts in — so a line long enough to wrap drew its
   second half on top of the row below. Long lines run off to the right
   instead, where the hidden gauge above has already made room for them. */
.text {
  flex: 1;
  padding-right: 12px;
  white-space: pre;
}
</style>
