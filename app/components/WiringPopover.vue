<script setup lang="ts">
import { nextTick, onMounted, ref } from 'vue'
import { X } from 'lucide-vue-next'
import Spinner from '~/components/Spinner.vue'
import { invoke } from '~/composables/useInvoke'
import { closeWiring, framework, wiring, type FrameworkTarget, type WiringRow } from '~/composables/useFramework'
import { openLocation } from '~/composables/useSymbols'
import { useToasts } from '~/composables/useToasts'

/**
 * How a class is wired, as the framework's own tool (Magento's magequery)
 * sees it: what it resolves to, its plugins with their sort order, the
 * observers, cron jobs and endpoints that name it, every config file that
 * mentions it. A row's name opens its class; the file and line beside it
 * open the config that says so. Esc or a click outside closes it.
 */

const panel = ref<HTMLElement | null>(null)

const short = (fqn: string) => fqn.slice(fqn.lastIndexOf('\\') + 1)
const base = (path: string) => path.slice(path.lastIndexOf('/') + 1)

async function openClass(fqn: string) {
  const found = await invoke<FrameworkTarget | null>('framework_class', { class: fqn }).catch(() => null)
  if (!found) {
    useToasts().info(`Could not find ${short(fqn)} in this repository`)
    return
  }
  closeWiring()
  openLocation(found)
}

function openSource(row: WiringRow) {
  if (!row.path) return
  closeWiring()
  openLocation({ path: row.path, line: row.line ?? 1, column: 1, kind: 'config', text: '' })
}

function openFile() {
  const path = wiring.result?.path
  if (!path) return
  closeWiring()
  openLocation({ path, line: 1, column: 1, kind: 'class', text: '' })
}

function onKey(event: KeyboardEvent) {
  if (event.key !== 'Escape') return
  event.preventDefault()
  event.stopPropagation()
  closeWiring()
}

onMounted(async () => {
  await nextTick()
  panel.value?.focus()
})
</script>

<template>
  <div class="overlay wiring-backdrop" @mousedown.self="closeWiring">
    <div
      ref="panel"
      class="wiring"
      role="dialog"
      aria-modal="true"
      :aria-label="`${framework.info?.name ?? 'Framework'} wiring of ${wiring.class}`"
      tabindex="-1"
      @keydown="onKey"
    >
      <header class="head">
        <div class="titles">
          <div class="title">{{ framework.info?.name ?? 'Framework' }} wiring</div>
          <div class="class mono" :title="wiring.class">{{ wiring.class }}</div>
        </div>
        <button class="close" title="Close" @click="closeWiring"><X :size="14" /></button>
      </header>

      <div v-if="wiring.loading" class="state faint"><Spinner :size="12" /> Asking magequery…</div>
      <div v-else-if="wiring.error" class="state error" data-testid="wiring-error">{{ wiring.error }}</div>
      <div v-else-if="wiring.result" class="body">
        <div class="facts">
          <span v-if="wiring.result.module" class="fact">{{ wiring.result.module }}</span>
          <span v-if="wiring.result.kind" class="fact">{{ wiring.result.kind }}</span>
          <button v-if="wiring.result.path" class="fact link" :title="wiring.result.path" @click="openFile">
            {{ base(wiring.result.path) }}
          </button>
        </div>
        <div v-if="wiring.result.preference" class="preference" data-testid="wiring-preference">
          Resolves to
          <button class="link mono" @click="openClass(wiring.result.preference)">
            {{ wiring.result.preference }}
          </button>
        </div>
        <p v-if="!wiring.result.sections.length" class="faint none">No plugins, observers or config reference it.</p>
        <section v-for="section in wiring.result.sections" :key="section.title" class="section">
          <h3>{{ section.title }} <span class="faint">{{ section.rows.length }}</span></h3>
          <div v-for="(row, at) in section.rows" :key="at" class="row">
            <button
              v-if="row.class"
              class="label link"
              :title="row.class"
              @click="openClass(row.class)"
            >
              {{ row.label }}
            </button>
            <span v-else class="label">{{ row.label }}</span>
            <span v-if="row.detail" class="detail faint">{{ row.detail }}</span>
            <button
              v-if="row.path"
              class="source link"
              :title="`${row.path}:${row.line ?? 1}`"
              @click="openSource(row)"
            >
              {{ base(row.path) }}:{{ row.line ?? 1 }}
            </button>
          </div>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.wiring-backdrop {
  position: fixed;
  inset: 0;
  z-index: 60;
}

.wiring {
  position: fixed;
  top: 64px;
  right: 16px;
  width: min(560px, calc(100vw - 32px));
  max-height: calc(100vh - 96px);
  display: flex;
  flex-direction: column;
  background: var(--canvas);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: var(--shadow-pop);
  outline: none;
  overflow: hidden;
  font-size: 12px;
}

.head {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 10px 12px 8px;
  border-bottom: 1px solid var(--line);
}

.titles {
  min-width: 0;
  flex: 1;
}

.title {
  font-weight: 600;
  color: var(--text);
}

.class {
  color: var(--text-dim);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mono {
  font-family: var(--mono);
}

.close {
  border: none;
  background: none;
  color: var(--text-dim);
  cursor: pointer;
  padding: 2px;
}

.state {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 14px 12px;
}

.state.error {
  color: var(--danger);
}

.body {
  overflow-y: auto;
  padding: 8px 12px 12px;
}

.facts {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 8px;
}

.fact {
  font-size: 10px;
  padding: 1px 7px;
  border-radius: 8px;
  background: var(--hover);
  color: var(--text-dim);
  border: none;
}

.preference {
  margin-bottom: 8px;
  color: var(--text-dim);
}

.none {
  margin: 8px 0 0;
}

.section h3 {
  margin: 10px 0 4px;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-dim);
}

.row {
  display: grid;
  grid-template-columns: minmax(0, auto) minmax(0, 1fr) auto;
  gap: 8px;
  align-items: baseline;
  padding: 3px 0;
}

.label {
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
}

.detail {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
}

.link {
  border: none;
  background: none;
  padding: 0;
  color: var(--primary);
  cursor: pointer;
  font: inherit;
}

.link:hover {
  text-decoration: underline;
}

.source {
  font-size: 11px;
  font-family: var(--mono);
}
</style>
