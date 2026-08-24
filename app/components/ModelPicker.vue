<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { Check, Eye, RefreshCw, Search } from 'lucide-vue-next'
import { contextLabel, priceLabel, useAi, type Model } from '~/composables/useAi'

const props = defineProps<{ selected: string | null }>()
const emit = defineEmits<{ pick: [string] }>()

const ai = useAi()
const query = ref('')
const sort = ref<'name' | 'cheapest' | 'context'>('name')

/** Filters on id, name and description so "claude", "free" and "vision" all work. */
const results = computed(() => {
  const needle = query.value.trim().toLowerCase()
  let list = ai.store.models
  if (needle) {
    const words = needle.split(/\s+/)
    list = list.filter((model) => {
      const haystack = `${model.id} ${model.name} ${model.description}`.toLowerCase()
      return words.every((word) => haystack.includes(word))
    })
  }
  const sorted = [...list]
  if (sort.value === 'cheapest') {
    sorted.sort((a, b) => a.prompt_price + a.completion_price - (b.prompt_price + b.completion_price))
  } else if (sort.value === 'context') {
    sorted.sort((a, b) => b.context_length - a.context_length)
  }
  return sorted.slice(0, 400)
})

const chosen = computed(() => ai.store.models.find((m) => m.id === props.selected) ?? null)

function pick(model: Model) {
  emit('pick', model.id)
}

onMounted(() => {
  if (!ai.store.models.length) ai.loadModels()
})
</script>

<template>
  <div class="picker">
    <div class="bar">
      <span class="search">
        <Search :size="13" class="faint" />
        <input v-model="query" type="search" placeholder="Search models — try claude, free, vision" />
      </span>
      <select v-model="sort" class="sort">
        <option value="name">By name</option>
        <option value="cheapest">Cheapest first</option>
        <option value="context">Largest context</option>
      </select>
      <button
        class="btn tiny"
        :disabled="ai.store.loadingModels"
        title="Refetch the catalogue from OpenRouter"
        @click="ai.loadModels(true)"
      >
        <RefreshCw :size="13" />
      </button>
    </div>

    <p v-if="ai.store.modelsError" class="error">{{ ai.store.modelsError }}</p>
    <p v-else-if="ai.store.loadingModels" class="dim pad">Loading the model catalogue…</p>

    <div v-else class="list">
      <button
        v-for="model in results"
        :key="model.id"
        class="row"
        :class="{ on: model.id === props.selected }"
        @click="pick(model)"
      >
        <Check v-if="model.id === props.selected" :size="14" class="tick" />
        <span v-else class="tick-space" />
        <span class="names">
          <span class="name truncate">{{ model.name || model.id }}</span>
          <span class="id mono truncate">{{ model.id }}</span>
        </span>
        <span class="meta">
          <Eye v-if="model.multimodal" :size="12" class="faint" title="Takes images too" />
          <span v-if="model.context_length" class="ctx">{{ contextLabel(model) }}</span>
          <span class="price" :class="{ free: model.prompt_price === 0 }">
            {{ priceLabel(model) }}
          </span>
        </span>
      </button>
      <p v-if="!results.length" class="dim pad">Nothing matches that.</p>
    </div>

    <p class="foot faint">
      Prices are US dollars per million tokens, as OpenRouter reports them.
      <template v-if="chosen"> Using <span class="mono">{{ chosen.id }}</span>.</template>
    </p>
  </div>
</template>

<style scoped>
.picker {
  border: 1px solid var(--line);
  border-radius: 8px;
  overflow: hidden;
}

.bar {
  display: flex;
  gap: 6px;
  padding: 7px;
  border-bottom: 1px solid var(--line);
  background: var(--bg-raised);
}

.search {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 8px;
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: 5px;
}

.search input {
  flex: 1;
  border: none;
  background: none;
  padding: 5px 0;
}

.search input:focus {
  outline: none;
}

.sort,
.tiny {
  font-size: 11.5px;
  padding: 4px 7px;
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: 5px;
  color: var(--text);
}

.list {
  max-height: 330px;
  overflow-y: auto;
}

.row {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 6px 10px;
  text-align: left;
  border-bottom: 1px solid var(--line-soft);
}

.row:hover {
  background: var(--bg-hover);
}

.row.on {
  background: var(--bg-active);
}

.tick {
  color: var(--green);
  flex: none;
}

.tick-space {
  width: 14px;
  flex: none;
}

.names {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.name {
  font-size: 12.5px;
}

.id {
  font-size: 10.5px;
  color: var(--text-faint);
}

.meta {
  display: flex;
  align-items: center;
  gap: 9px;
  flex: none;
  font-size: 11px;
}

.ctx {
  color: var(--text-dim);
}

.price {
  color: var(--amber);
  white-space: nowrap;
}

.price.free {
  color: var(--green);
}

.pad {
  padding: 14px;
  font-size: 12px;
  margin: 0;
}

.error {
  margin: 0;
  padding: 12px;
  font-size: 12px;
  color: var(--red);
}

.foot {
  margin: 0;
  padding: 6px 10px;
  font-size: 11px;
  border-top: 1px solid var(--line);
  background: var(--bg-raised);
}
</style>
