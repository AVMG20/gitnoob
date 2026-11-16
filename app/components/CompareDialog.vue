<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ConflictBlock } from '~/composables/useGit'
import {
  buildGrid,
  freshPick,
  resolutionOf,
  sideLines,
  stanceOf,
  type Pick
} from '~/composables/useConflictGrid'
import { compareBlocks } from '~/composables/useFileEdit'

/**
 * Your edits beside what is on disk now, one difference at a time.
 *
 * Built on the conflict resolver's own model: each place the two differ is a
 * region, laid out by its grid so the sides line up, and answered with its
 * picks — yours, theirs, or both. What comes out is written over the file as
 * it now stands, so neither side is lost unless it was chosen to be.
 */

const props = defineProps<{ path: string; mine: string; theirs: string }>()
const emit = defineEmits<{ done: [string]; close: [] }>()

type Region = Extract<ConflictBlock, { kind: 'conflict' }>

const blocks = compareBlocks(props.mine, props.theirs)
const regions = blocks.filter((block): block is Region => block.kind === 'conflict')

const escape = (text: string) =>
  text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

const grid = buildGrid(blocks, {
  ours: sideLines(blocks, 'ours').map(escape),
  theirs: sideLines(blocks, 'theirs').map(escape),
  base: []
})

const picks = ref<Pick[]>(regions.map((region) => freshPick(region)))

function choose(index: number, how: 'ours' | 'theirs' | 'both') {
  const region = regions[index]
  if (!region) return
  picks.value[index] = {
    ours: region.ours.map(() => how !== 'theirs'),
    theirs: region.theirs.map(() => how !== 'ours'),
    ours_first: true,
    custom: null,
    touched: true
  }
}

const stances = computed(() => picks.value.map((pick) => stanceOf(pick)))

/**
 * Differences nobody has answered yet. Saving waits for all of them: a
 * default of "yours" would quietly drop whatever the other side wrote.
 */
const undecided = computed(() => picks.value.filter((pick) => !pick.touched).length)

/** The file as the picks make it. */
function result(): string {
  const out: string[] = []
  for (const block of blocks) {
    if (block.kind === 'context') {
      out.push(...block.lines)
      continue
    }
    const pick = picks.value[block.index]!
    const choice = resolutionOf(pick, block)
    if (choice.custom) out.push(...choice.custom)
    else {
      const ours = choice.take_ours ? block.ours : []
      const theirs = choice.take_theirs ? block.theirs : []
      out.push(...(choice.ours_first ? [...ours, ...theirs] : [...theirs, ...ours]))
    }
  }
  return out.join('\n')
}

const LABEL: Record<string, string> = {
  ours: 'yours',
  theirs: 'on disk',
  both: 'both',
  dropped: 'neither',
  mixed: 'mixed',
  edited: 'edited'
}
</script>

<template>
  <AppModal :title="`Compare ${props.path}`" :width="980" keep @close="emit('close')">
    <p class="intro dim">
      {{ regions.length }} {{ regions.length === 1 ? 'place differs' : 'places differ' }}. Pick
      what each one keeps; the result is saved over the file on disk.
    </p>
    <div class="grid">
      <div class="side-head">Your edits</div>
      <div class="side-head">On disk now</div>
      <template v-for="(row, at) in grid.rows" :key="at">
        <div v-if="row.kind === 'head'" class="head" :class="stances[row.conflict]">
          <span class="which">
            Difference {{ row.conflict + 1 }} ·
            {{ picks[row.conflict]?.touched ? `keeps ${LABEL[stances[row.conflict] ?? 'ours']}` : 'not decided yet' }}
          </span>
          <button
            class="btn tiny"
            data-pick="ours"
            :class="{ picked: picks[row.conflict]?.touched && stances[row.conflict] === 'ours' }"
            :aria-pressed="!!picks[row.conflict]?.touched && stances[row.conflict] === 'ours'"
            @click="choose(row.conflict, 'ours')"
          >
            Yours
          </button>
          <button
            class="btn tiny"
            data-pick="theirs"
            :class="{ picked: picks[row.conflict]?.touched && stances[row.conflict] === 'theirs' }"
            :aria-pressed="!!picks[row.conflict]?.touched && stances[row.conflict] === 'theirs'"
            @click="choose(row.conflict, 'theirs')"
          >
            On disk
          </button>
          <button
            class="btn tiny"
            data-pick="both"
            :class="{ picked: picks[row.conflict]?.touched && stances[row.conflict] === 'both' }"
            :aria-pressed="!!picks[row.conflict]?.touched && stances[row.conflict] === 'both'"
            @click="choose(row.conflict, 'both')"
          >
            Both
          </button>
        </div>
        <template v-else>
          <div class="cell" :class="{ code: row.kind === 'code', empty: !row.ours }">
            <span class="num">{{ row.ours?.num ?? '' }}</span>
            <span class="text" v-html="row.ours?.html ?? ''" />
          </div>
          <div class="cell" :class="{ code: row.kind === 'code', empty: !row.theirs, theirs: true }">
            <span class="num">{{ row.theirs?.num ?? '' }}</span>
            <span class="text" v-html="row.theirs?.html ?? ''" />
          </div>
        </template>
      </template>
    </div>
    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <span v-if="undecided" class="undecided faint">
        {{ undecided }} {{ undecided === 1 ? 'difference' : 'differences' }} left to decide
      </span>
      <button
        class="btn btn-primary"
        data-testid="compare-save"
        :disabled="undecided > 0"
        @click="emit('done', result())"
      >
        Save the result
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.undecided {
  margin-right: auto;
  font-size: 12px;
}

.intro {
  margin: 0 0 8px;
  font-size: 12.5px;
}

.grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  max-height: 60vh;
  overflow: auto;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  font-family: var(--mono);
  font-size: var(--code-size, 12px);
}

.side-head {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: 4px 8px;
  background: var(--canvas);
  border-bottom: 1px solid var(--line);
  font-family: var(--font);
  font-weight: 600;
}

.head {
  grid-column: 1 / span 2;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  background: var(--canvas);
  border-top: 1px solid var(--line);
  font-family: var(--font);
  font-size: 12px;
}

.head .which {
  flex: 1;
}

.cell {
  display: flex;
  min-height: 18px;
  line-height: 18px;
  white-space: pre;
  overflow: hidden;
}

/* A tint of each side's colour with an edge in the colour itself, as the
   conflict resolver draws its sides: the tokens are solid accents, and code
   on a solid accent cannot be read in any theme. */
.cell.code {
  background: color-mix(in srgb, var(--diff-ours) 12%, transparent);
  box-shadow: inset 2px 0 0 var(--diff-ours);
}

.cell.code.theirs {
  background: color-mix(in srgb, var(--diff-theirs) 12%, transparent);
  box-shadow: inset 2px 0 0 var(--diff-theirs);
}

/* A line longer than its half says so rather than stopping mid-word. */
.cell .text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.head .btn.picked {
  background: var(--bg-active);
  color: var(--text);
  font-weight: 600;
}

.cell.empty {
  background: var(--bg-hover);
}

.num {
  flex: none;
  width: 40px;
  padding-right: 8px;
  text-align: right;
  color: var(--text-faint);
}
</style>
