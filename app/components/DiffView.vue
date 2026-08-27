<script setup lang="ts">
import { computed } from 'vue'
import type { DiffLine, FileDiff } from '~/composables/useGit'
import { highlightLine, highlightWhole, languageFor } from '~/composables/useHighlight'
import { diffRows, diffWindow } from '~/composables/useCode'

const props = defineProps<{
  diff: FileDiff | null
  /** The file as it stands on the side being shown, when it could be read. */
  text?: string | null
  loading?: boolean
  /** When set, each hunk gets its own stage/unstage and discard buttons. */
  side?: 'staged' | 'unstaged' | null
  busy?: boolean
  /** Where the box that scrolls this is scrolled to, and how tall it is. */
  top?: number
  view?: number
  /** The same sideways: how far it is scrolled, and how wide the box is. */
  left?: number
  width?: number
}>()
const emit = defineEmits<{ hunk: [number, 'stage' | 'unstage' | 'discard'] }>()

const empty = computed(() => !!props.diff && props.diff.hunks.length === 0)
const language = computed(() => (props.diff ? languageFor(props.diff.path) : null))

function lineClass(origin: string) {
  if (origin === '+') return 'add'
  if (origin === '-') return 'del'
  // Git's "\ No newline at end of file": a remark about the lines around it
  // rather than a line of either file, and drawn as one.
  if (origin === '\\') return 'eof'
  return 'ctx'
}

/**
 * A file long enough that painting all of it to colour a few changed lines
 * costs more than the colour is worth. Generated files are what reach this.
 */
const WHOLE_LIMIT = 20_000

/**
 * The file, coloured whole, beside the plain lines it was made from.
 *
 * This is what lets the patch be coloured with everything a line cannot know
 * about itself: the body of a block comment, a string that runs on, and the
 * script inside a `.vue` file, which is only JavaScript because of a `<script>`
 * tag some lines above the hunk. The plain lines are kept so a line can be
 * checked against the one it claims to be before its colour is used.
 */
const whole = computed(() => {
  const source = props.text
  const language_ = language.value
  if (!language_ || source === null || source === undefined) return null
  const plain = source.split('\n')
  if (plain.length > WHOLE_LIMIT) return null
  return { plain, html: highlightWhole(source, language_) }
})

/**
 * Highlighted lines, cached by content.
 *
 * `paint` used to be called straight from the template, so highlight.js ran
 * again for every line on every re-render — and a diff is thousands of lines
 * that have not changed. The cache is dropped whenever the language does, which
 * is whenever a different file is opened.
 */
const perLine = computed(() => {
  const cache = new Map<string, string>()
  const language_ = language.value
  return (code: string) => {
    const hit = cache.get(code)
    if (hit !== undefined) return hit
    const html = highlightLine(code, language_)
    cache.set(code, html)
    return html
  }
})

/**
 * A line's colour, taken from the whole file where the line is in it.
 *
 * Only where the file agrees that this is the line it says it is: the diff and
 * the file are read separately, and a write landing between the two would
 * otherwise paint each line in the colours of whatever now sits at its number.
 * A deleted line is not in the new file at all and is coloured on its own,
 * which is the best that can be done without reading the old one too.
 */
// --- only what is on screen
const laid = computed(() => diffRows(props.diff?.hunks ?? []))
const shown = computed(() => diffWindow(laid.value.rows, props.top ?? 0, props.view ?? 0))
const visible = computed(() => laid.value.rows.slice(shown.value.first, shown.value.last))

/**
 * The heading of the hunk the top of the view is in.
 *
 * It used to be `position: sticky`, which needs the heading to be in the page
 * to stick — and with only the rows on screen drawn, the heading of a hunk
 * taller than the box is not. So it is drawn again where the sticky one would
 * have come to rest. Which hunk that is comes from the first row on screen, not
 * the first row drawn: the window reaches above the top edge.
 */
const pinned = computed(() => {
  const rows = laid.value.rows
  const at = shown.value.first
  if (!rows.length || at >= rows.length) return null
  const top = props.top ?? 0
  let hunk = rows[at]!.hunk
  for (let i = at; i < rows.length && rows[i]!.top <= top; i++) hunk = rows[i]!.hunk
  return hunk
})

/**
 * Where a hunk heading is drawn, and how wide.
 *
 * As wide as the box rather than as wide as the file, and carried along by the
 * horizontal scroll, so the heading and the buttons on it are in the same place
 * whatever the longest line in the file did to the width. Without a box to
 * measure — nothing has told it yet — it falls back to the old full width,
 * which is right until the first scroll event.
 */
function headStyle(at: number) {
  return {
    top: `${at}px`,
    width: props.width ? `${props.width}px` : '100%',
    transform: props.left ? `translateX(${props.left}px)` : undefined
  }
}

/** The longest line in the patch, which is what holds the view open sideways. */
const longest = computed(() => {
  let found = ''
  for (const hunk of props.diff?.hunks ?? []) {
    for (const line of hunk.lines) if (line.content.length > found.length) found = line.content
  }
  return found
})

function paint(line: DiffLine) {
  // The no-newline remark is git talking, not the file: highlighting it as
  // whatever language this is would colour a sentence as code.
  if (line.origin === '\\') return highlightLine(line.content, null)
  const file = whole.value
  if (file && line.new_lineno !== null) {
    const at = line.new_lineno - 1
    if (file.plain[at] === line.content) return file.html[at] ?? ''
  }
  return perLine.value(line.content)
}
</script>

<template>
  <div class="diff">
    <p v-if="props.loading" class="note dim">Loading diff…</p>
    <p v-else-if="!props.diff" class="note dim">Select a file.</p>
    <p v-else-if="props.diff.binary" class="note dim">Binary file — no text diff to show.</p>
    <p v-else-if="empty" class="note dim">No changes in this file.</p>

    <template v-else>
      <!-- Not painted, and no height of its own; it is here to be measured, so
           the view keeps one width whichever rows are drawn in it. -->
      <div class="line gauge" aria-hidden="true">
        <span class="no" />
        <span class="no" />
        <span class="sign" />
        <span class="text">{{ longest }}</span>
      </div>

      <div class="rows" :style="{ height: `${laid.height}px` }">
        <template v-for="row in visible" :key="`${row.kind}${row.top}`">
          <div
            v-if="row.kind === 'head'"
            class="hunk-head mono"
            :style="headStyle(row.top)"
          >
            <span class="truncate">{{ props.diff.hunks[row.hunk]?.header }}</span>
            <!-- Discard sits away from the staging button, so the destructive
                 one is never where the hand already is. -->
            <span v-if="props.side" class="hunk-actions">
              <button
                v-if="props.side === 'unstaged'"
                class="hunk-btn danger"
                :disabled="props.busy"
                title="Throw away just this hunk"
                @click="emit('hunk', row.hunk, 'discard')"
              >
                Discard hunk
              </button>
              <button
                class="hunk-btn"
                :disabled="props.busy"
                @click="emit('hunk', row.hunk, props.side === 'staged' ? 'unstage' : 'stage')"
              >
                {{ props.side === 'staged' ? 'Unstage hunk' : 'Stage hunk' }}
              </button>
            </span>
          </div>
          <div
            v-else-if="row.line"
            class="line"
            :class="lineClass(row.line.origin)"
            :style="{ top: `${row.top}px` }"
          >
            <span class="no">{{ row.line.old_lineno ?? '' }}</span>
            <span class="no">{{ row.line.new_lineno ?? '' }}</span>
            <span class="sign">{{ row.line.origin === ' ' ? '' : row.line.origin }}</span>
            <span class="text" v-html="paint(row.line)" />
          </div>
        </template>

        <!-- Where the sticky heading would have come to rest. -->
        <div
          v-if="pinned !== null"
          class="hunk-head mono pin"
          :style="headStyle(props.top ?? 0)"
        >
          <span class="truncate">{{ props.diff.hunks[pinned]?.header }}</span>
          <span v-if="props.side" class="hunk-actions">
            <button
              v-if="props.side === 'unstaged'"
              class="hunk-btn danger"
              :disabled="props.busy"
              title="Throw away just this hunk"
              @click="emit('hunk', pinned, 'discard')"
            >
              Discard hunk
            </button>
            <button
              class="hunk-btn"
              :disabled="props.busy"
              @click="emit('hunk', pinned, props.side === 'staged' ? 'unstage' : 'stage')"
            >
              {{ props.side === 'staged' ? 'Unstage hunk' : 'Stage hunk' }}
            </button>
          </span>
        </div>
      </div>

      <p v-if="props.diff.truncated" class="note dim">
        {{ props.diff.truncated.toLocaleString() }} more changed lines are not shown. A file this
        large is generated rather than written; open it in an editor if you need the rest.
      </p>
    </template>
  </div>
</template>

<style scoped>
/* As wide as the widest line rather than as wide as the view, which the hidden
   gauge below settles.
   
   The rows are positioned against a box inside this one, so a row's
   `min-width: 100%` is only worth what this is worth: left at the width of the
   view, the tint on an added line stopped where the view did and the rest of
   the row was bare as soon as the patch was scrolled sideways. */
.diff {
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.5;
  width: max-content;
  min-width: 100%;
}

.note {
  font-family: var(--font);
  padding: 12px;
}

/* Rows are placed rather than stacked: only the ones on screen are drawn, and
   each has to sit where it would have if they all were. */
.rows {
  position: relative;
}

.rows > * {
  position: absolute;
  left: 0;
}

.hunk-head {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 24px;
  width: 100%;
  padding: 2px 10px;
  color: var(--text-faint);
  background: var(--bg-raised);
  border-top: 1px solid var(--line-soft);
  border-bottom: 1px solid var(--line-soft);
  box-sizing: border-box;
}

/* The one drawn at the top edge, over the rows it is the heading for. */
.hunk-head.pin {
  z-index: 3;
}

.hunk-actions {
  margin-left: auto;
  display: flex;
  gap: 5px;
  opacity: 0;
  transition: opacity 0.1s;
}

.hunk-head:hover .hunk-actions {
  opacity: 1;
}

.hunk-btn {
  font-family: var(--font);
  font-size: 10.5px;
  padding: 1px 7px;
  border: 1px solid var(--line);
  border-radius: 9px;
  color: var(--text-dim);
  white-space: nowrap;
}

.hunk-btn:hover:not(:disabled) {
  color: var(--text);
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 14%, transparent);
}

.hunk-btn.danger:hover:not(:disabled) {
  color: var(--red-soft);
  border-color: var(--red);
  background: var(--danger-bg);
}

.hunk-btn:disabled {
  opacity: 0.4;
}

.line {
  display: flex;
  height: 18px;
  white-space: pre;
  /* As wide as the widest line, and never narrower than the view. Without the
     first, the tint on an added or deleted row stops at the edge of the window
     and the rest of the row is bare once you scroll sideways; without the
     second, short rows do not reach the right-hand edge. */
  width: max-content;
  min-width: 100%;
}

.no {
  flex: none;
  width: 42px;
  padding-right: 8px;
  text-align: right;
  color: var(--text-faint);
  user-select: none;
}

.sign {
  flex: none;
  width: 14px;
  text-align: center;
  user-select: none;
}

.text {
  flex: 1;
  padding-right: 10px;
  tab-size: 4;
}

.add {
  background: var(--success-bg);
}

.add .sign {
  color: var(--green-soft);
}

.del {
  background: var(--danger-bg);
}

.del .sign {
  color: var(--red-soft);
}

.eof,
.eof .sign {
  color: var(--text-faint);
  font-style: italic;
}

/* The row tint carries which side a line is on, so the syntax colours stay
   readable rather than being overridden by green and red. */
.add {
  background: var(--success-bg);
}

.del {
  background: var(--danger-bg);
}

.gauge {
  position: static;
  width: max-content;
  height: 0;
  overflow: hidden;
  visibility: hidden;
  pointer-events: none;
}
</style>
