<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
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
  /** What this line said before it was changed, when it replaced something. */
  was: string[]
  /** Lines deleted immediately above this one, with nothing put in their place. */
  removed: string[]
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
  // The text of what went, not just how much of it: a gutter mark that can be
  // asked what it replaced is worth more than one that can only say something
  // happened here.
  const before = new Map<number, string[]>()
  const gaps = new Map<number, string[]>()

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
      const deleted: string[] = []
      const added: number[] = []
      while (end < hunk.lines.length && hunk.lines[end]!.origin !== ' ') {
        const line = hunk.lines[end]!
        if (line.origin === '-') deleted.push(line.content)
        else if (line.new_lineno) added.push(line.new_lineno)
        end++
      }
      const deletions = deleted.length

      if (added.length) {
        // As many added lines as were deleted are the replacements; anything
        // beyond that is genuinely new.
        for (const [at, number] of added.entries()) {
          marks.set(number, at < deletions ? 'changed' : 'added')
          if (at >= deletions) continue
          // Where more went than came back, the surplus has no line of its own
          // to hang from, so it joins the last of the replacements: the run
          // still reads as one change, and none of it goes unaccounted for.
          const replaced =
            at === added.length - 1 ? deleted.slice(at) : deleted.slice(at, at + 1)
          before.set(number, replaced)
        }
      } else if (deletions) {
        // Nothing replaced them, so the mark belongs to the seam: the line the
        // deleted ones used to sit above.
        const next = hunk.lines[end]?.new_lineno ?? source.length + 1
        gaps.set(next, [...(gaps.get(next) ?? []), ...deleted])
      }
      index = end
    }
  }

  return source.map((_, at) => ({
    number: at + 1,
    mark: marks.get(at + 1) ?? null,
    was: before.get(at + 1) ?? [],
    removed: gaps.get(at + 1) ?? []
  }))
})

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
