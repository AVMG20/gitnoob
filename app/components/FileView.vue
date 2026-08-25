<script setup lang="ts">
import { computed } from 'vue'
import type { FileDiff } from '~/composables/useGit'
import { highlightWhole, languageFor } from '~/composables/useHighlight'

const props = defineProps<{
  diff: FileDiff | null
  /** The whole file as it stands on the side being shown. */
  text: string | null
  loading?: boolean
  error?: string | null
}>()

/** What happened to a line of the file as it now stands. */
type Mark = 'added' | 'changed' | null

interface Line {
  number: number
  mark: Mark
  /** Lines deleted immediately above this one, with nothing put in their place. */
  removed: number
}

const language = computed(() => (props.diff ? languageFor(props.diff.path) : null))

/**
 * The file, line by line, with what changed marked against it.
 *
 * An editor's gutter distinguishes three things, and so does this: a line that
 * is new, a line that replaced one, and a place where lines were taken out and
 * nothing put back. Git's diff does not name the middle one — it is a deletion
 * and an insertion sitting together — so a run of the two is read as a change
 * to the lines that survived it, which is what someone reading the file sees.
 */
const lines = computed<Line[]>(() => {
  const text = props.text
  if (text === null) return []
  const source = text.split('\n')
  // A file that ends in a newline splits into a last empty piece that is not a
  // line of the file.
  if (source.length && source[source.length - 1] === '') source.pop()

  const marks = new Map<number, Mark>()
  const gaps = new Map<number, number>()

  for (const hunk of props.diff?.hunks ?? []) {
    // Walk each run of touched lines together: what a run is made of decides
    // whether it reads as an addition or as a change.
    let index = 0
    while (index < hunk.lines.length) {
      if (hunk.lines[index]!.origin === ' ') {
        index++
        continue
      }
      let end = index
      let deletions = 0
      const added: number[] = []
      while (end < hunk.lines.length && hunk.lines[end]!.origin !== ' ') {
        const line = hunk.lines[end]!
        if (line.origin === '-') deletions++
        else if (line.new_lineno) added.push(line.new_lineno)
        end++
      }

      if (added.length) {
        // As many added lines as were deleted are the replacements; anything
        // beyond that is genuinely new.
        for (const [at, number] of added.entries()) {
          marks.set(number, at < deletions ? 'changed' : 'added')
        }
      } else if (deletions) {
        // Nothing replaced them, so the mark belongs to the seam: the line the
        // deleted ones used to sit above.
        const next = hunk.lines[end]?.new_lineno ?? source.length + 1
        gaps.set(next, (gaps.get(next) ?? 0) + deletions)
      }
      index = end
    }
  }

  return source.map((_, at) => ({
    number: at + 1,
    mark: marks.get(at + 1) ?? null,
    removed: gaps.get(at + 1) ?? 0
  }))
})

const counts = computed(() => ({
  marked: lines.value.filter((line) => line.mark).length,
  gaps: lines.value.filter((line) => line.removed).length
}))

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
      <div v-for="line in lines" :key="line.number" class="line" :class="line.mark ?? ''">
        <span class="no">{{ line.number }}</span>
        <!-- The bar an editor draws between the numbers and the code: solid
             where a line is new or changed, and a wedge where lines were taken
             out and nothing put in their place, since a removal has no line of
             its own to colour. -->
        <span class="gutter">
          <span
            v-if="line.removed"
            class="gone"
            :title="`${line.removed} ${line.removed === 1 ? 'line' : 'lines'} deleted here`"
          />
        </span>
        <span class="text" v-html="paint(line.number)" />
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
