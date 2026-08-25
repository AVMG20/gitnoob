<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Check, ChevronDown, Search } from 'lucide-vue-next'

/** One row of the list. `note` is drawn on the right, dimmed. */
export interface Choice {
  value: string
  label: string
  note?: string
  /** Searched as well as shown, so a group name finds its branches. */
  hint?: string
}

const props = withDefaults(
  defineProps<{
    modelValue: string | null
    options: Choice[]
    placeholder?: string
    /** Shown when the search matches nothing. */
    empty?: string
    disabled?: boolean
    /** Branch names read better in the same face they are typed in. */
    mono?: boolean
  }>(),
  { placeholder: 'Choose…', empty: 'Nothing matches that.', disabled: false, mono: false }
)

const emit = defineEmits<{ 'update:modelValue': [string] }>()

const open = ref(false)
const query = ref('')
const active = ref(0)
const trigger = ref<HTMLElement | null>(null)
const field = ref<HTMLInputElement | null>(null)
const list = ref<HTMLElement | null>(null)
const box = ref<{ left: number; top: number; width: number; drop: boolean }>({
  left: 0,
  top: 0,
  width: 220,
  drop: true
})

const chosen = computed(() => props.options.find((option) => option.value === props.modelValue) ?? null)

/** Every word has to appear somewhere in the row, in any order. */
const results = computed(() => {
  const needle = query.value.trim().toLowerCase()
  if (!needle) return props.options
  const words = needle.split(/\s+/)
  return props.options.filter((option) => {
    const haystack = `${option.label} ${option.note ?? ''} ${option.hint ?? ''}`.toLowerCase()
    return words.every((word) => haystack.includes(word))
  })
})

const style = computed(() => ({
  left: `${box.value.left}px`,
  width: `${box.value.width}px`,
  ...(box.value.drop ? { top: `${box.value.top}px` } : { bottom: `${box.value.top}px` })
}))

/**
 * Anchors the list to the button, above it when there is no room below.
 *
 * Positioned against the window rather than the button's own corner: the
 * dialog scrolls its content, and a list that scrolls away from what it
 * belongs to is worse than one that covers it.
 */
async function show() {
  if (props.disabled) return
  const rect = trigger.value?.getBoundingClientRect()
  if (!rect) return
  const below = window.innerHeight - rect.bottom
  const drop = below > 240 || below > rect.top
  box.value = {
    left: rect.left,
    width: Math.max(rect.width, 200),
    top: drop ? rect.bottom + 4 : window.innerHeight - rect.top + 4,
    drop
  }
  query.value = ''
  active.value = Math.max(0, props.options.findIndex((option) => option.value === props.modelValue))
  open.value = true
  await nextTick()
  field.value?.focus()
  scrollToActive()
}

function pick(choice: Choice) {
  emit('update:modelValue', choice.value)
  open.value = false
}

function move(step: number) {
  if (!results.value.length) return
  const count = results.value.length
  active.value = (active.value + step + count) % count
  scrollToActive()
}

function scrollToActive() {
  nextTick(() => {
    list.value?.querySelector('.row.active')?.scrollIntoView({ block: 'nearest' })
  })
}

function enter() {
  const choice = results.value[active.value]
  if (choice) pick(choice)
}

// A filtered list is a different list: the highlight belongs on its first row,
// not on whatever index the last one happened to leave behind.
watch(query, () => (active.value = 0))
</script>

<template>
  <div class="select">
    <button
      ref="trigger"
      type="button"
      class="face"
      :class="{ empty: !chosen, off: props.disabled }"
      :disabled="props.disabled"
      @click="open ? (open = false) : show()"
    >
      <span class="text truncate" :class="{ mono: props.mono && !!chosen }">
        {{ chosen?.label ?? props.placeholder }}
      </span>
      <span v-if="chosen?.note" class="note">{{ chosen.note }}</span>
      <ChevronDown :size="13" class="chev" />
    </button>

    <Teleport to="body">
      <div v-if="open" class="scrim" @click="open = false" @contextmenu.prevent="open = false">
        <div class="panel" :style="style" @click.stop>
          <span class="search">
            <Search :size="13" class="faint" />
            <input
              ref="field"
              v-model="query"
              type="search"
              :placeholder="props.placeholder"
              @keydown.down.prevent="move(1)"
              @keydown.up.prevent="move(-1)"
              @keydown.enter.prevent="enter"
              @keydown.esc.prevent="open = false"
              @keydown.tab="open = false"
            />
          </span>
          <div ref="list" class="select-list">
            <button
              v-for="(option, index) in results"
              :key="option.value"
              type="button"
              class="row"
              :class="{ active: index === active, on: option.value === props.modelValue }"
              @click="pick(option)"
              @mousemove="active = index"
            >
              <Check v-if="option.value === props.modelValue" :size="13" class="tick" />
              <span v-else class="tick-space" />
              <span class="label truncate" :class="{ mono: props.mono }">{{ option.label }}</span>
              <span v-if="option.note" class="row-note">{{ option.note }}</span>
            </button>
            <p v-if="!results.length" class="none faint">{{ props.empty }}</p>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.select {
  min-width: 0;
}

.face {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 5px 8px;
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: 5px;
  color: var(--text);
  font-size: 12.5px;
  text-align: left;
}

.face:hover:not(.off) {
  border-color: var(--accent);
}

.face.off {
  opacity: 0.55;
}

.face.empty .text {
  color: var(--text-faint);
}

.text {
  flex: 1;
  min-width: 0;
}

.note,
.row-note {
  flex: none;
  font-size: 10.5px;
  color: var(--text-faint);
}

.chev {
  flex: none;
  color: var(--text-faint);
}

/* Above the dialog it belongs to: the list is opened from inside one. */
.scrim {
  position: fixed;
  inset: 0;
  z-index: 60;
}

.panel {
  position: fixed;
  display: flex;
  flex-direction: column;
  max-height: 300px;
  min-width: 200px;
  background: var(--bg-panel);
  border: 1px solid var(--line);
  border-radius: 7px;
  box-shadow: 0 12px 30px rgba(0, 0, 0, 0.45);
  overflow: hidden;
}

.search {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 8px;
  border-bottom: 1px solid var(--line);
  background: var(--bg-raised);
}

.search input {
  flex: 1;
  min-width: 0;
  border: none;
  background: none;
  padding: 6px 0;
  font-size: 12.5px;
}

.search input:focus {
  outline: none;
}

.select-list {
  overflow-y: auto;
}

.row {
  display: flex;
  align-items: center;
  gap: 7px;
  width: 100%;
  padding: 5px 9px;
  text-align: left;
  font-size: 12.5px;
  color: var(--text);
}

.row.active {
  background: var(--bg-hover);
}

.row.on {
  color: var(--text);
}

.tick {
  flex: none;
  color: var(--green);
}

.tick-space {
  width: 13px;
  flex: none;
}

.label {
  flex: 1;
  min-width: 0;
}

.none {
  margin: 0;
  padding: 12px;
  font-size: 12px;
}
</style>
