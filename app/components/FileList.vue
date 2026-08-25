<script setup lang="ts">
import { computed } from 'vue'
import {
  ArrowRight,
  ChevronDown,
  ChevronRight,
  Folder,
  Minus,
  Pencil,
  Plus
} from 'lucide-vue-next'
import {
  buildRows,
  change,
  useFileView,
  type Change,
  type FileEntry,
  type Tally
} from '~/composables/useFileView'

const props = defineProps<{
  files: FileEntry[]
  selected?: string | null
  /** Shown when the list is empty. */
  empty?: string
  /** Draggable rows, for moving files between staged and unstaged. */
  draggable?: boolean
  /**
   * What the button on a hovered row does — "Stage file", say. Unset, and no
   * button appears.
   */
  action?: string
}>()
const emit = defineEmits<{
  select: [string]
  menu: [MouseEvent, FileEntry]
  /** Right-click on a folder row, with the folder's path. */
  dirmenu: [MouseEvent, string]
  dragstart: [DragEvent, FileEntry]
  dragend: []
  act: [FileEntry]
}>()

const view = useFileView()
const rows = computed(() => buildRows(props.files, view.state.mode, view.state.collapsed))

/**
 * Where a row's content starts.
 *
 * A directory row spends 18px on its chevron before the folder icon; a file
 * row has no chevron. Indenting both by the same step therefore left a file's
 * name to the *left* of its parent folder's name, which is why a file looked
 * like it sat beside the folder rather than inside it. Files get that 18px
 * back, less one step, and then a little more so the nesting reads.
 */
function indent(depth: number, file = false) {
  return 12 + depth * STEP + (file ? 8 : 0)
}

const STEP = 16

/**
 * What happened to a file, drawn rather than lettered.
 *
 * Git's own letters have to be learned — and `?` for a new file reads as
 * something being wrong with it rather than as something being new. A pencil
 * and a plus do not need a legend.
 */
const MARKS: Record<Change, { icon: typeof Pencil; label: string }> = {
  added: { icon: Plus, label: 'New file' },
  modified: { icon: Pencil, label: 'Edited' },
  deleted: { icon: Minus, label: 'Deleted' },
  renamed: { icon: ArrowRight, label: 'Renamed' }
}

function mark(kind: string) {
  return MARKS[change(kind)]
}

/** A folder's counts, in a fixed order and with the empty ones left out. */
function counted(tally: Tally) {
  return (Object.keys(MARKS) as Change[])
    .filter((key) => tally[key] > 0)
    .map((key) => ({ key, count: tally[key], icon: MARKS[key].icon, label: MARKS[key].label }))
}
</script>

<template>
  <div class="list">
    <template v-for="row in rows" :key="row.key">
      <button
        v-if="row.kind === 'dir'"
        class="row dir"
        :style="{ paddingLeft: `${indent(row.depth)}px` }"
        @click="view.toggleDir(row.path)"
        @contextmenu="emit('dirmenu', $event, row.path)"
      >
        <component :is="row.collapsed ? ChevronRight : ChevronDown" :size="12" class="chev" />
        <Folder :size="12" class="folder" />
        <span class="name truncate">{{ row.name }}</span>
        <!-- What is inside, summed, and only while it is folded away: with the
             folder open the files below say it themselves, in more detail. -->
        <template v-if="row.collapsed">
          <span
            v-for="part in counted(row.tally ?? { added: 0, modified: 0, deleted: 0, renamed: 0 })"
            :key="part.key"
            class="tally"
            :class="part.key"
            :title="`${part.count} ${part.label.toLowerCase()}`"
          >
            <component :is="part.icon" :size="10" :stroke-width="2.25" />{{ part.count }}
          </span>
        </template>
      </button>

      <div
        v-else
        class="row file"
        :class="{ on: props.selected === row.path }"
        :style="{ paddingLeft: `${indent(row.depth, true)}px` }"
        :title="row.path"
        :draggable="props.draggable"
        @click="emit('select', row.path)"
        @contextmenu="row.entry && emit('menu', $event, row.entry)"
        @dragstart="row.entry && emit('dragstart', $event, row.entry)"
        @dragend="emit('dragend')"
      >
        <component
          :is="mark(row.entry?.kind ?? '').icon"
          :size="12"
          :stroke-width="2"
          class="mark"
          :class="change(row.entry?.kind ?? '')"
        >
          <title>{{ mark(row.entry?.kind ?? '').label }}</title>
        </component>
        <span class="name truncate">{{ row.name }}</span>
        <span v-if="row.entry?.additions" class="plus">+{{ row.entry.additions }}</span>
        <span v-if="row.entry?.deletions" class="minus">−{{ row.entry.deletions }}</span>
        <!-- The whole file in one click, without going through the menu. It
             appears on hover rather than sitting on every row: a list of forty
             files with forty buttons on it is a list nobody can read. -->
        <button
          v-if="props.action && row.entry"
          class="act"
          @click.stop="row.entry && emit('act', row.entry)"
        >
          {{ props.action }}
        </button>
      </div>
    </template>

    <p v-if="!props.files.length" class="none faint">{{ props.empty ?? 'Nothing here.' }}</p>
  </div>
</template>

<style scoped>
.list {
  overflow-y: auto;
  flex: 1;
  min-height: 0;
}

.row {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 3px 12px 3px 12px;
  font-size: 12px;
  text-align: left;
  cursor: default;
}

.row:hover {
  background: var(--bg-hover);
}

.row.on {
  background: var(--bg-active);
}

.chev,
.folder {
  flex: none;
  color: var(--text-faint);
}

/* A folder's name gives up the rest of the row, so its counts sit against the
   name rather than out at the far edge where they belong to nothing. */
.dir .name {
  flex: 0 1 auto;
  color: var(--text-dim);
}

.name {
  flex: 1;
  min-width: 0;
}

.mark {
  flex: none;
}

/* The icon carries the colour and the number carries the count. Colouring both
   made a folder's summary louder than the folder. */
.tally {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  flex: none;
  color: var(--text-dim);
  font-size: 11px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

/* A plus is two thin strokes around a lot of empty space, so at the green the
   other marks are drawn in it reads as muted. It gets a brighter one. */
.mark.added,
.tally.added svg {
  color: #86f5b4;
}

.mark.deleted,
.tally.deleted svg {
  color: var(--red);
}

.mark.modified,
.tally.modified svg {
  color: var(--amber);
}

.mark.renamed,
.tally.renamed svg {
  color: var(--purple);
}

/* The same ghost button the panel heads use, so a row's button belongs to this
   window rather than to the one it was borrowed from. */
.act {
  display: none;
  flex: none;
  padding: 1px 7px;
  margin: -2px 0;
  border: 1px solid var(--line);
  border-radius: 4px;
  background: var(--bg-raised);
  color: var(--text-dim);
  font-size: 11px;
  white-space: nowrap;
}

.row:hover .act {
  display: block;
}

.act:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.plus {
  color: var(--green);
  font-size: 11px;
}

.minus {
  color: var(--red);
  font-size: 11px;
}

.none {
  padding: 5px 12px 7px;
  font-size: 11.5px;
  margin: 0;
}
</style>
