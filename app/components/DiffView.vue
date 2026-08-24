<script setup lang="ts">
import { computed } from 'vue'
import type { FileDiff } from '~/composables/useGit'
import { highlightLine, languageFor } from '~/composables/useHighlight'

const props = defineProps<{
  diff: FileDiff | null
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

const paint = (code: string) => highlightLine(code, language.value)
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
          <span v-if="props.side" class="hunk-actions">
            <button
              class="hunk-btn"
              :disabled="props.busy"
              @click="emit('hunk', hi, props.side === 'staged' ? 'unstage' : 'stage')"
            >
              {{ props.side === 'staged' ? 'Unstage hunk' : 'Stage hunk' }}
            </button>
            <button
              v-if="props.side === 'unstaged'"
              class="hunk-btn danger"
              :disabled="props.busy"
              title="Throw away just this hunk"
              @click="emit('hunk', hi, 'discard')"
            >
              Discard hunk
            </button>
          </span>
        </div>
        <div v-for="(line, li) in hunk.lines" :key="li" class="line" :class="lineClass(line.origin)">
          <span class="no">{{ line.old_lineno ?? '' }}</span>
          <span class="no">{{ line.new_lineno ?? '' }}</span>
          <span class="sign">{{ line.origin === ' ' ? '' : line.origin }}</span>
          <span class="text" v-html="paint(line.content)" />
        </div>
      </div>
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
  background: rgba(79, 156, 249, 0.14);
}

.hunk-btn.danger:hover:not(:disabled) {
  color: #ef8d9c;
  border-color: var(--red);
  background: rgba(224, 87, 109, 0.14);
}

.hunk-btn:disabled {
  opacity: 0.4;
}

.line {
  display: flex;
  white-space: pre;
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
  color: #8ad9ae;
}

.del {
  background: rgba(224, 87, 109, 0.1);
}

.del .sign {
  color: #ef8d9c;
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
