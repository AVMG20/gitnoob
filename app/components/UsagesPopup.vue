<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { keyLabel, keysFor } from '~/composables/useShortcuts'
import FileIcon from '~/components/FileIcon.vue'
import { closeUsages, openLocation, usagesPopup, type PopupItem } from '~/composables/useSymbols'
import { framework, openWiring } from '~/composables/useFramework'
import { pathKind } from '~/composables/pathKind'

/**
 * The list ⌘-click shows when a name has several usages, or several
 * definitions. Placed where the click was, kept on screen. ↑ and ↓ move, ↵
 * opens, Esc or a click outside closes. Focus goes back where it was.
 *
 * Definite usages come first. The possible ones (same name, receiver
 * unknown) sit under a "Possible (n)" row that ↵ or a click unfolds.
 *
 * Tests and dependencies sort after the rest of their group (`marked` in
 * useSymbols.ts) and are drawn in a colour of their own, with a tag, since
 * they are seldom where anyone means to go.
 *
 * The backdrop is an `.overlay`, so the app's shortcuts stand aside while it
 * is open.
 */

const panel = ref<HTMLElement | null>(null)
let before: Element | null = null

const WIDTH = 460

type Row =
  | { type: 'item'; item: PopupItem; possible: boolean; config: boolean; heading: boolean }
  | { type: 'group' }

const rows = computed<Row[]>(() => {
  const out: Row[] = usagesPopup.items.map((item) => ({
    type: 'item',
    item,
    possible: false,
    config: false,
    heading: false
  }))
  // Framework config that names it (Magento's XML), under its own heading.
  usagesPopup.config.forEach((item, at) =>
    out.push({ type: 'item', item, possible: false, config: true, heading: at === 0 })
  )
  if (usagesPopup.possible.length) {
    out.push({ type: 'group' })
    if (usagesPopup.showPossible) {
      for (const item of usagesPopup.possible) {
        out.push({ type: 'item', item, possible: true, config: false, heading: false })
      }
    }
  }
  return out
})

/** The footer's link to the framework's account of the class, when it has one. */
const wiringClass = computed(() => (framework.info?.wiring ? usagesPopup.wiring : null))

function showWiring() {
  const fqn = wiringClass.value
  if (!fqn) return
  closeUsages()
  void openWiring(fqn)
}

const place = computed(() => {
  const x = Math.max(8, Math.min(usagesPopup.x, window.innerWidth - WIDTH - 8))
  const below = usagesPopup.y + 18
  const height = Math.min(
    360,
    64 + (usagesPopup.note ? 18 : 0) + rows.value.length * 26 + (usagesPopup.config.length ? 24 : 0)
  )
  const y = below + height > window.innerHeight - 8 ? Math.max(8, usagesPopup.y - height - 6) : below
  return { left: `${x}px`, top: `${y}px`, width: `${WIDTH}px` }
})

const base = (path: string) => path.slice(path.lastIndexOf('/') + 1)
const folder = (path: string) => (path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '')

const LABEL: Record<PopupItem['state'], string> = { changed: 'changed', new: 'new', unchanged: '' }

function open(item: PopupItem | undefined) {
  if (!item) return
  closeUsages()
  openLocation(item)
}

function togglePossible() {
  usagesPopup.showPossible = !usagesPopup.showPossible
}

function pick(row: Row | undefined) {
  if (!row) return
  if (row.type === 'group') togglePossible()
  else open(row.item)
}

async function move(by: number) {
  const count = rows.value.length
  if (!count) return
  usagesPopup.cursor = (usagesPopup.cursor + by + count) % count
  await nextTick()
  panel.value?.querySelector('.row.on')?.scrollIntoView?.({ block: 'nearest' })
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.preventDefault()
    closeUsages()
  } else if (event.key === 'ArrowDown') {
    event.preventDefault()
    void move(1)
  } else if (event.key === 'ArrowUp') {
    event.preventDefault()
    void move(-1)
  } else if (event.key === 'Enter') {
    event.preventDefault()
    pick(rows.value[usagesPopup.cursor])
  } else if (event.key === 'Tab') {
    // Nothing else in the popup takes focus, so Tab stays where it is rather
    // than walking into the page behind the backdrop.
    event.preventDefault()
  } else return
  event.stopPropagation()
}

const backKey = computed(() => keyLabel(keysFor('nav.back')))

onMounted(async () => {
  before = document.activeElement
  await nextTick()
  panel.value?.focus()
})

onBeforeUnmount(() => {
  if (before instanceof HTMLElement && before.isConnected && !usagesPopup.open) before.focus()
})
</script>

<template>
  <div class="overlay usages-backdrop" @mousedown.self="closeUsages">
    <div
      ref="panel"
      class="usages"
      role="dialog"
      aria-modal="true"
      :aria-label="usagesPopup.title"
      :aria-activedescendant="rows.length ? `usage-${usagesPopup.cursor}` : undefined"
      tabindex="-1"
      :style="place"
      @keydown="onKey"
    >
      <div class="title">{{ usagesPopup.title }}</div>
      <div v-if="usagesPopup.note" class="note faint">{{ usagesPopup.note }}</div>
      <div class="rows" role="listbox">
        <template v-for="(row, at) in rows" :key="row.type === 'group' ? 'possible' : `${row.possible ? 'p' : row.config ? 'c' : 'd'}:${row.item.path}:${row.item.line}:${row.item.column}`">
          <div v-if="row.type === 'item' && row.heading" class="heading" data-testid="config-heading">
            {{ usagesPopup.configLabel }} ({{ usagesPopup.config.length }})
          </div>
          <div
            v-if="row.type === 'group'"
            :id="`usage-${at}`"
            class="group"
            role="option"
            :class="{ on: at === usagesPopup.cursor }"
            :aria-selected="at === usagesPopup.cursor"
            :aria-expanded="usagesPopup.showPossible"
            title="The same name, called on something whose class could not be worked out"
            @mousemove="usagesPopup.cursor = at"
            @click="togglePossible"
          >
            <span class="chevron" :class="{ open: usagesPopup.showPossible }">›</span>
            Possible ({{ usagesPopup.possible.length }})
          </div>
          <div
            v-else
            :id="`usage-${at}`"
            class="row"
            role="option"
            :class="[`kind-${pathKind(row.item.path)}`, { on: at === usagesPopup.cursor, possible: row.possible }]"
            :aria-selected="at === usagesPopup.cursor"
            :title="`${row.item.path}:${row.item.line}`"
            @mousemove="usagesPopup.cursor = at"
            @click="open(row.item)"
          >
            <span class="where">
              <FileIcon :path="row.item.path" :size="13" class="icon" />
              <span class="file">{{ base(row.item.path) }}</span><span class="num">:{{ row.item.line }}</span>
              <span class="dir">{{ folder(row.item.path) }}</span>
            </span>
            <span class="code">{{ row.item.text }}</span>
            <span v-if="pathKind(row.item.path) !== 'project'" class="kind-tag" :class="pathKind(row.item.path)">{{
              pathKind(row.item.path)
            }}</span>
            <span v-if="row.item.label" class="tag" :class="{ override: row.item.label.endsWith('override') }">{{ row.item.label }}</span>
            <span v-else-if="LABEL[row.item.state]" class="state" :class="row.item.state">{{ LABEL[row.item.state] }}</span>
          </div>
        </template>
      </div>
      <div class="foot faint">
        ↵ open · {{ backKey }} or the mouse's back button goes back to where you were
        <button v-if="wiringClass" type="button" class="wiring-link" data-testid="wiring-link" @click="showWiring">
          {{ framework.info?.name }} wiring
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.usages-backdrop {
  position: fixed;
  inset: 0;
  z-index: 60;
}

.usages {
  position: fixed;
  display: flex;
  flex-direction: column;
  max-height: 360px;
  background: var(--canvas);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: var(--shadow-pop);
  outline: none;
  overflow: hidden;
  font-size: 12px;
}

.title {
  padding: 8px 12px 6px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex-shrink: 0;
}

.rows {
  overflow-y: auto;
  min-height: 0;
}

.row {
  display: grid;
  grid-template-columns: minmax(0, 15em) minmax(0, 1fr) auto auto;
  gap: 10px;
  align-items: baseline;
  padding: 4px 12px;
  cursor: pointer;
}

.row.on,
.group.on {
  background: var(--active);
}

.row.possible .file,
.row.possible .code {
  color: var(--text-faint);
}

/* A test or a dependency: in a colour of its own, quieter than the rest. */
.row.kind-test .file,
.row.kind-test .num {
  color: color-mix(in srgb, var(--success) 60%, var(--text-faint));
}

.row.kind-vendor .file,
.row.kind-vendor .num {
  color: color-mix(in srgb, var(--warning) 45%, var(--text-faint));
}

.row.kind-test .code,
.row.kind-vendor .code {
  color: var(--text-faint);
}

.kind-tag {
  font-size: 10px;
  padding: 0 6px;
  border-radius: 8px;
  white-space: nowrap;
}

.kind-tag.test {
  color: color-mix(in srgb, var(--success) 70%, var(--text-faint));
  background: color-mix(in srgb, var(--success) 12%, transparent);
}

.kind-tag.vendor {
  color: color-mix(in srgb, var(--warning) 60%, var(--text-faint));
  background: color-mix(in srgb, var(--warning) 12%, transparent);
}

.note {
  padding: 0 12px 6px;
  font-size: 11px;
  flex-shrink: 0;
}

.group {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 12px;
  margin-top: 2px;
  border-top: 1px solid var(--line);
  color: var(--text-dim);
  font-size: 11px;
  cursor: pointer;
  user-select: none;
}

.chevron {
  display: inline-block;
  width: 10px;
  transition: transform 0.12s;
}

.chevron.open {
  transform: rotate(90deg);
}

.where .icon {
  margin-right: 4px;
}

.where {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.file {
  color: var(--text);
}

.num {
  color: var(--text-faint);
}

.dir {
  margin-left: 6px;
  color: var(--text-faint);
  font-size: 11px;
}

.code {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-family: var(--mono);
  font-size: 11px;
  color: var(--text-dim);
}

.state {
  font-size: 10px;
  padding: 0 6px;
  border-radius: 8px;
  border: 1px solid currentColor;
}

.state.changed {
  color: var(--info);
}

.state.new {
  color: var(--success);
}

.foot {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px 8px;
  border-top: 1px solid var(--line);
  font-size: 11px;
}

.heading {
  padding: 6px 12px 2px;
  margin-top: 2px;
  border-top: 1px solid var(--line);
  color: var(--text-dim);
  font-size: 11px;
  font-weight: 600;
}

.tag {
  font-size: 10px;
  padding: 0 6px;
  border-radius: 8px;
  color: var(--text-dim);
  background: var(--hover);
  white-space: nowrap;
}

.tag.override {
  color: var(--info);
}

.wiring-link {
  margin-left: auto;
  border: none;
  background: none;
  padding: 0;
  color: var(--primary);
  font-size: 11px;
  cursor: pointer;
}

.wiring-link:hover {
  text-decoration: underline;
}
</style>
