<script setup lang="ts">
import { computed, ref } from 'vue'
import { useGit } from '~/composables/useGit'

const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const name = ref('')
const start = ref('')

const taken = computed(() =>
  store.refs?.locals.some((b) => b.name === name.value.trim()) ?? false
)
const invalid = computed(() => {
  const value = name.value.trim()
  // The rules git itself enforces, checked up front so the error is inline.
  return (
    !value ||
    taken.value ||
    /[\s~^:?*\[\\]/.test(value) ||
    value.startsWith('-') ||
    value.startsWith('/') ||
    value.endsWith('/') ||
    value.endsWith('.') ||
    value.includes('..') ||
    value.includes('@{')
  )
})

async function submit() {
  if (invalid.value) return
  // Null is the only failure: a command that succeeds quietly still returns a
  // string, and an empty one would otherwise leave the dialog open saying the
  // branch it had just made already exists.
  const created = await git.createBranch(name.value.trim(), start.value.trim() || undefined)
  if (created !== null) emit('close')
}
</script>

<template>
  <AppModal title="New branch" :width="440" @close="emit('close')">
    <label class="field">
      <span class="label">Name</span>
      <input
        v-model="name"
        type="text"
        placeholder="feature/thing"
        autofocus
        @keyup.enter="submit"
      />
      <span v-if="taken" class="hint bad">A branch with that name already exists.</span>
      <span v-else-if="name && invalid" class="hint bad">Git will not accept that name.</span>
    </label>

    <label class="field">
      <span class="label">Starting point</span>
      <input v-model="start" type="text" :placeholder="store.repo?.head ?? 'HEAD'" />
      <span class="hint faint">A branch, tag or commit. Defaults to the current HEAD.</span>
    </label>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-primary" :disabled="store.busy || invalid" @click="submit">
        Create and check out
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.field {
  display: block;
  margin-bottom: 14px;
}

.label {
  display: block;
  margin-bottom: 4px;
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.field input {
  width: 100%;
}

.hint {
  display: block;
  margin-top: 4px;
  font-size: 11px;
}

.bad {
  color: var(--red);
}
</style>
