<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import { GitBranch, Search, X } from 'lucide-vue-next'
import { useConfig } from '~/composables/useConfig'
import MidTruncate from '~/components/MidTruncate.vue'

/**
 * Every repository this profile has opened, one keystroke away.
 *
 * The tab strip only holds what is open now, and closing a tab used to mean
 * finding the folder again. This searches the open tabs and everything opened
 * before them, so the way back is always the same three keys.
 *
 * Its own Escape and arrow handling rather than `useShortcuts`, because a
 * scrim stops that firing at all — which is what keeps the rest of the window's
 * keys from going off underneath this.
 */
const emit = defineEmits<{ open: [path: string]; close: [] }>()

const config = useConfig()

const query = ref('')
const active = ref(0)
const box = ref<HTMLInputElement | null>(null)

interface Row {
  path: string
  name: string
  /** Whether it is already a tab, which decides what picking it does. */
  open: boolean
}

/** Open tabs first, then everything else, each newest-first within its group. */
const rows = computed<Row[]>(() => {
  const open = config.projects.value.map((one) => ({ ...one, open: true }))
  const seen = new Set(open.map((one) => one.path))
  const past = config.recents.value
    .filter((one) => !seen.has(one.path))
    .map((one) => ({ ...one, open: false }))
  return [...open, ...past]
})

const results = computed(() => {
  const words = query.value.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return rows.value
  return rows.value.filter((row) => {
    const haystack = `${row.name} ${row.path}`.toLowerCase()
    return words.every((word) => haystack.includes(word))
  })
})

function move(step: number) {
  const count = results.value.length
  if (!count) return
  active.value = (active.value + step + count) % count
  void nextTick(() => {
    document.querySelector('.switch .row.active')?.scrollIntoView({ block: 'nearest' })
  })
}

function pick(row: Row | undefined) {
  if (!row) return
  emit('open', row.path)
  emit('close')
}

/** Takes one out of the list without opening it. */
async function forget(row: Row, event: Event) {
  event.stopPropagation()
  await config.forgetProject(row.path)
  if (active.value >= results.value.length) active.value = Math.max(0, results.value.length - 1)
}

onMounted(async () => {
  await nextTick()
  box.value?.focus()
})
</script>

<template>
  <Teleport to="body">
    <div class="scrim" @click.self="emit('close')">
      <div class="switch">
        <div class="search">
          <Search :size="14" class="faint" />
          <input
            ref="box"
            v-model="query"
            type="text"
            placeholder="Go to a repository"
            spellcheck="false"
            @input="active = 0"
            @keydown.down.prevent="move(1)"
            @keydown.up.prevent="move(-1)"
            @keydown.enter.prevent="pick(results[active])"
            @keydown.esc.prevent="emit('close')"
          />
        </div>

        <div class="list">
          <button
            v-for="(row, at) in results"
            :key="row.path"
            class="row"
            :class="{ active: at === active, on: row.open }"
            @click="pick(row)"
            @mousemove="active = at"
          >
            <GitBranch :size="13" class="glyph" />
            <span class="names">
              <span class="name truncate">{{ row.name }}</span>
              <MidTruncate class="faint path" :text="row.path" />
            </span>
            <span v-if="row.open" class="badge">open</span>
            <span
              v-else
              class="drop"
              title="Forget this one"
              @click="forget(row, $event)"
            >
              <X :size="12" />
            </span>
          </button>
          <p v-if="!results.length" class="none faint">
            {{ rows.length ? 'Nothing matches.' : 'Nothing opened under this profile yet.' }}
          </p>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: grid;
  /* Near the top rather than centred: the list grows downwards, and a box that
     grows from the middle of the window moves its own first row as you type. */
  align-content: start;
  justify-items: center;
  padding-top: 12vh;
  background: var(--overlay);
}

.switch {
  width: min(560px, 92vw);
  background: var(--bg-panel);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  box-shadow: 0 18px 50px var(--shadow-strong);
  overflow: hidden;
}

.search {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 12px;
  border-bottom: 1px solid var(--line-soft);
}

.search input {
  flex: 1;
  min-width: 0;
  padding: 0;
  background: none;
  border: none;
  outline: none;
  font-size: 14px;
}

.list {
  max-height: 340px;
  overflow-y: auto;
  padding: 4px;
}

.row {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 5px 8px;
  border-radius: var(--radius-sm);
  text-align: left;
}

.row.active {
  background: var(--bg-hover);
}

.glyph {
  flex: none;
  color: var(--text-faint);
}

.row.on .glyph {
  color: var(--accent);
}

.names {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.path {
  font-size: 10px;
}

.badge {
  flex: none;
  padding: 1px 6px;
  border-radius: 999px;
  font-size: 10px;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 16%, transparent);
}

/* Only on the row under the pointer: a column of crosses beside every past
   repository reads as a list of things to delete. */
.drop {
  display: flex;
  flex: none;
  padding: 2px;
  border-radius: 4px;
  color: var(--text-faint);
  opacity: 0;
}

.row:hover .drop,
.row.active .drop {
  opacity: 1;
}

.drop:hover {
  background: var(--bg-active);
  color: var(--text);
}

.none {
  padding: 10px 12px;
  font-size: 12px;
}
</style>
