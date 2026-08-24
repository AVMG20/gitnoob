<script setup lang="ts">
import { computed, ref } from 'vue'
import { useGit, type GraphRow } from '~/composables/useGit'

const props = defineProps<{ row: GraphRow }>()
const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const name = ref('')
const message = ref('')
const push = ref(false)

const taken = computed(() => (store.refs?.tags ?? []).some((t) => t.name === name.value.trim()))
const invalid = computed(() => {
  const value = name.value.trim()
  return !value || taken.value || /[\s~^:?*\[\\]/.test(value) || value.includes('..')
})

async function submit() {
  if (invalid.value) return
  const tag = name.value.trim()
  if ((await git.createTag(tag, props.row.oid, message.value.trim() || undefined)) === null) return
  if (push.value) await git.pushTag('origin', tag)
  emit('close')
}
</script>

<template>
  <AppModal title="Tag this commit" :width="440" @close="emit('close')">
    <p class="dim intro">
      <span class="mono">{{ props.row.short }}</span>
      {{ props.row.summary }}
    </p>

    <label class="field">
      <span class="label">Tag name</span>
      <input v-model="name" type="text" placeholder="v1.2.0" autofocus @keyup.enter="submit" />
      <span v-if="taken" class="hint bad">That tag already exists.</span>
    </label>

    <label class="field">
      <span class="label">Message</span>
      <input v-model="message" type="text" placeholder="Optional — makes it an annotated tag" />
    </label>

    <label class="check">
      <input v-model="push" type="checkbox" />
      Push the tag to origin as well
    </label>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-primary" :disabled="store.busy || invalid" @click="submit">
        Create tag
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.intro {
  margin: 0 0 14px;
  font-size: 12px;
  display: flex;
  gap: 8px;
}

.field {
  display: block;
  margin-bottom: 14px;
}

.label {
  display: block;
  margin-bottom: 5px;
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

input[type='text'] {
  width: 100%;
}

.hint {
  display: block;
  margin-top: 5px;
  font-size: 11px;
}

.bad {
  color: var(--red);
}

.check {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  cursor: pointer;
}
</style>
