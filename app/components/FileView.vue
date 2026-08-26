<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import type { FileDiff } from '~/composables/useGit'
import { highlightWhole, languageFor } from '~/composables/useHighlight'
import { CODE_ROW, markedLines, windowOf, type Line } from '~/composables/useCode'

const props = defineProps<{
  diff: FileDiff | null
  /** The whole file as it stands on the side being shown. */
  text: string | null
  loading?: boolean
  error?: string | null
  /** Where the box that scrolls this is scrolled to, and how tall it is. */
  top?: number
  view?: number
}>()

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

const ROW = CODE_ROW
</script>

<template>
  <div class="file">
    <p v-if="props.loading" class="note dim">Loading file…</p>
    <p v-else-if="props.error" class="note dim">{{ props.error }}</p>
    <p v-else-if="props.diff?.binary" class="note dim">Binary file — nothing to read.</p>
    <p v-else-if="props.text === null" class="note dim">Select a file.</p>

    <template v-else>
      <p v-if="!counts.marked && !counts.gaps" class="note dim">
        Nothing changed in this file — it is shown as it stands.
      </p>
      <!-- Not painted, and no height of its own; it is here to be measured. -->
      <div class="line gauge" aria-hidden="true">
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
        <span class="no">{{ line.number }}</span>
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
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
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
}

.text {
  flex: 1;
  min-width: 0;
  padding-right: 12px;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>
