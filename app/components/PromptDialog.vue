<script setup lang="ts">
import { ref } from 'vue'

const props = defineProps<{
  title: string
  label: string
  initial?: string
  placeholder?: string
  hint?: string
  confirm?: string
  danger?: boolean
}>()
const emit = defineEmits<{ close: []; submit: [string] }>()

const value = ref(props.initial ?? '')
</script>

<template>
  <AppModal :title="props.title" :width="420" :tone="props.danger ? 'danger' : 'normal'" @close="emit('close')">
    <label class="field">
      <span class="label">{{ props.label }}</span>
      <input
        v-model="value"
        type="text"
        autofocus
        :placeholder="props.placeholder"
        @keyup.enter="value.trim() && emit('submit', value.trim())"
      />
      <span v-if="props.hint" class="hint faint">{{ props.hint }}</span>
    </label>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn"
        :class="props.danger ? 'btn-danger' : 'btn-primary'"
        :disabled="!value.trim()"
        @click="emit('submit', value.trim())"
      >
        {{ props.confirm ?? 'Continue' }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.field {
  display: block;
}

.label {
  display: block;
  margin-bottom: 5px;
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

input {
  width: 100%;
}

.hint {
  display: block;
  margin-top: 6px;
  font-size: 11px;
  line-height: 1.5;
}
</style>
