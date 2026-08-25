<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Check, Plus, Search, X } from 'lucide-vue-next'
import { initials, tint } from '~/composables/useAvatars'
import type { Member } from '~/composables/useForge'

const props = withDefaults(
  defineProps<{
    /** The people chosen so far; this component owns none of them. */
    modelValue: Member[]
    people: Member[]
    loading?: boolean
    error?: string | null
    placeholder?: string
  }>(),
  { loading: false, error: null, placeholder: 'Add someone' }
)

const emit = defineEmits<{ 'update:modelValue': [Member[]] }>()

const open = ref(false)
const query = ref('')
const active = ref(0)
const trigger = ref<HTMLElement | null>(null)
const field = ref<HTMLInputElement | null>(null)
const list = ref<HTMLElement | null>(null)
const box = ref({ left: 0, top: 0, width: 240, drop: true })

const results = computed(() => {
  const needle = query.value.trim().toLowerCase()
  if (!needle) return props.people
  const words = needle.split(/\s+/)
  return props.people.filter((person) => {
    const haystack = `${person.login} ${person.name}`.toLowerCase()
    return words.every((word) => haystack.includes(word))
  })
})

const style = computed(() => ({
  left: `${box.value.left}px`,
  width: `${box.value.width}px`,
  ...(box.value.drop ? { top: `${box.value.top}px` } : { bottom: `${box.value.top}px` })
}))

function has(person: Member) {
  return props.modelValue.some((chosen) => chosen.login === person.login)
}

/** Clicking someone already on the list takes them off it again. */
function toggle(person: Member) {
  emit(
    'update:modelValue',
    has(person)
      ? props.modelValue.filter((chosen) => chosen.login !== person.login)
      : [...props.modelValue, person]
  )
}

function remove(person: Member) {
  emit('update:modelValue', props.modelValue.filter((chosen) => chosen.login !== person.login))
}

async function show() {
  const rect = trigger.value?.getBoundingClientRect()
  if (!rect) return
  const below = window.innerHeight - rect.bottom
  const drop = below > 260 || below > rect.top
  box.value = {
    left: rect.left,
    width: Math.max(rect.width, 240),
    top: drop ? rect.bottom + 4 : window.innerHeight - rect.top + 4,
    drop
  }
  query.value = ''
  active.value = 0
  open.value = true
  await nextTick()
  field.value?.focus()
}

function move(step: number) {
  if (!results.value.length) return
  const count = results.value.length
  active.value = (active.value + step + count) % count
  nextTick(() => list.value?.querySelector('.row.active')?.scrollIntoView({ block: 'nearest' }))
}

function enter() {
  const person = results.value[active.value]
  if (person) toggle(person)
}

watch(query, () => (active.value = 0))
</script>

<template>
  <div class="people">
    <span v-for="person in props.modelValue" :key="person.login" class="chip">
      <span class="face" :style="{ background: tint(person.login) }">
        {{ initials(person.name, person.login) }}
      </span>
      <span class="who truncate">{{ person.login }}</span>
      <button type="button" class="drop" :title="`Remove ${person.login}`" @click="remove(person)">
        <X :size="11" />
      </button>
    </span>

    <button ref="trigger" type="button" class="add" @click="open ? (open = false) : show()">
      <Plus :size="12" />
      {{ props.modelValue.length ? 'Add' : props.placeholder }}
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
              placeholder="Search people"
              @keydown.down.prevent="move(1)"
              @keydown.up.prevent="move(-1)"
              @keydown.enter.prevent="enter"
              @keydown.esc.prevent="open = false"
            />
          </span>

          <p v-if="props.error" class="none bad">{{ props.error }}</p>
          <p v-else-if="props.loading" class="none faint">Asking the forge who is on this project…</p>

          <div v-else ref="list" class="select-list">
            <button
              v-for="(person, index) in results"
              :key="person.login"
              type="button"
              class="row"
              :class="{ active: index === active }"
              @click="toggle(person)"
              @mousemove="active = index"
            >
              <Check v-if="has(person)" :size="13" class="tick" />
              <span v-else class="tick-space" />
              <span class="face small" :style="{ background: tint(person.login) }">
                {{ initials(person.name, person.login) }}
              </span>
              <span class="names truncate">
                <span class="login">{{ person.login }}</span>
                <span v-if="person.name !== person.login" class="real faint">{{ person.name }}</span>
              </span>
            </button>
            <p v-if="!results.length" class="none faint">Nobody matches that.</p>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.people {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 5px;
}

.chip {
  display: flex;
  align-items: center;
  gap: 5px;
  max-width: 180px;
  padding: 2px 3px 2px 3px;
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: 11px;
  font-size: 11.5px;
}

.face {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 17px;
  height: 17px;
  border-radius: 50%;
  font-size: 8px;
  font-weight: 600;
  color: #0d1116;
}

.face.small {
  width: 18px;
  height: 18px;
  font-size: 8.5px;
}

.who {
  min-width: 0;
}

.drop {
  display: flex;
  padding: 1px;
  border-radius: 50%;
  color: var(--text-faint);
}

.drop:hover {
  color: var(--red);
  background: var(--bg-hover);
}

.add {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border: 1px dashed var(--line);
  border-radius: 11px;
  color: var(--text-dim);
  font-size: 11.5px;
}

.add:hover {
  border-color: var(--accent);
  color: var(--text);
}

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
  padding: 4px 9px;
  text-align: left;
  font-size: 12.5px;
  color: var(--text);
}

.row.active {
  background: var(--bg-hover);
}

.tick {
  flex: none;
  color: var(--green);
}

.tick-space {
  width: 13px;
  flex: none;
}

.names {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}

.real {
  font-size: 11px;
}

.none {
  margin: 0;
  padding: 12px;
  font-size: 12px;
}

.bad {
  color: var(--red);
}
</style>
