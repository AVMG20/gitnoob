<script setup lang="ts">
import { computed } from 'vue'
import type { DiffLine, FileDiff } from '~/composables/useGit'
import { highlightLine, highlightWhole, languageFor } from '~/composables/useHighlight'

const props = defineProps<{
  diff: FileDiff | null
  /** The file as it stands on the side being shown, when it could be read. */
  text?: string | null
  loading?: boolean
  /** When set, each hunk gets its own stage/unstage and discard buttons. */
  side?: 'staged' | 'unstaged' | null
  busy?: boolean
}>()
const emit = defineEmits<{ hunk: [number, 'stage' | 'unstage' | 'discard'] }>()

const empty = computed(() => !!props.diff && props.diff.hunks.length === 0)
const language = computed(() => (props.diff ? languageFor(props.diff.path) : null))

function lineClass(origin: string) {
  if (origin === '+') return 'add'
  if (origin === '-') return 'del'
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
function paint(line: DiffLine) {
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
      <div v-for="(hunk, hi) in props.diff.hunks" :key="hi" class="hunk">
        <div class="hunk-head mono">
          <span class="truncate">{{ hunk.header }}</span>
          <!-- Discard sits away from the staging button, so the destructive
               one is never where the hand already is. -->
          <span v-if="props.side" class="hunk-actions">
            <button
              v-if="props.side === 'unstaged'"
              class="hunk-btn danger"
              :disabled="props.busy"
              title="Throw away just this hunk"
              @click="emit('hunk', hi, 'discard')"
            >
              Discard hunk
            </button>
            <button
              class="hunk-btn"
              :disabled="props.busy"
              @click="emit('hunk', hi, props.side === 'staged' ? 'unstage' : 'stage')"
            >
              {{ props.side === 'staged' ? 'Unstage hunk' : 'Stage hunk' }}
            </button>
          </span>
        </div>
        <div v-for="(line, li) in hunk.lines" :key="li" class="line" :class="lineClass(line.origin)">
          <span class="no">{{ line.old_lineno ?? '' }}</span>
          <span class="no">{{ line.new_lineno ?? '' }}</span>
          <span class="sign">{{ line.origin === ' ' ? '' : line.origin }}</span>
          <span class="text" v-html="paint(line)" />
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
.diff {
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.5;
}

.note {
  font-family: var(--font);
  padding: 12px;
}

.hunk-head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 2px 10px;
  color: var(--text-faint);
  background: var(--bg-raised);
  border-top: 1px solid var(--line-soft);
  border-bottom: 1px solid var(--line-soft);
  position: sticky;
  top: 0;
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
  background: rgba(224, 87, 109, 0.14);
}

.hunk-btn:disabled {
  opacity: 0.4;
}

.line {
  display: flex;
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
  background: rgba(87, 193, 132, 0.1);
}

.add .sign {
  color: var(--green-soft);
}

.del {
  background: rgba(224, 87, 109, 0.1);
}

.del .sign {
  color: var(--red-soft);
}

/* The row tint carries which side a line is on, so the syntax colours stay
   readable rather than being overridden by green and red. */
.add {
  background: rgba(87, 193, 132, 0.11);
}

.del {
  background: rgba(224, 87, 109, 0.11);
}
</style>
