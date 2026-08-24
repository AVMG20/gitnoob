<script setup lang="ts">
import { computed } from 'vue'
import { ChevronDown, ChevronRight, Folder } from 'lucide-vue-next'
import { buildRows, useFileView, type FileEntry } from '~/composables/useFileView'

const props = defineProps<{
  files: FileEntry[]
  selected?: string | null
  /** Shown when the list is empty. */
  empty?: string
  /** Draggable rows, for moving files between staged and unstaged. */
  draggable?: boolean
}>()
const emit = defineEmits<{
  select: [string]
  menu: [MouseEvent, FileEntry]
  dragstart: [DragEvent, FileEntry]
  dragend: []
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

function mark(kind: string) {
  return (
    {
      added: 'A',
      modified: 'M',
      deleted: 'D',
      renamed: 'R',
      copied: 'C',
      untracked: '?',
      typechange: 'T'
    }[kind] ?? '·'
  )
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
      >
        <component :is="row.collapsed ? ChevronRight : ChevronDown" :size="12" class="chev" />
        <Folder :size="12" class="folder" />
        <span class="name truncate">{{ row.name }}</span>
        <span class="count faint">{{ row.count }}</span>
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
        <span class="mark" :class="row.entry?.kind">{{ mark(row.entry?.kind ?? '') }}</span>
        <span class="name truncate">{{ row.name }}</span>
        <span v-if="row.entry?.additions" class="plus">+{{ row.entry.additions }}</span>
        <span v-if="row.entry?.deletions" class="minus">−{{ row.entry.deletions }}</span>
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

.dir .name {
  color: var(--text-dim);
}

.name {
  flex: 1;
  min-width: 0;
}

.count {
  font-size: 10.5px;
}

.mark {
  flex: none;
  width: 12px;
  text-align: center;
  font-family: var(--mono);
  font-size: 11px;
  font-weight: 700;
  color: var(--text-faint);
}

.mark.added,
.mark.untracked {
  color: var(--green);
}

.mark.deleted {
  color: var(--red);
}

.mark.modified {
  color: var(--accent);
}

.mark.renamed {
  color: var(--purple);
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
