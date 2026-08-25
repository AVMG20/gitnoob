<script setup lang="ts">
import { computed, ref } from 'vue'
import { useGit } from '~/composables/useGit'

/**
 * Adds a remote, or edits the address of one that exists. The name is fixed
 * once the remote does — renaming moves the tracking branches, so it is a
 * menu action of its own rather than a quiet side effect of this form.
 */
const props = defineProps<{
  /** The remote being edited; empty means adding one. */
  name?: string
}>()
const emit = defineEmits<{ close: [] }>()

const git = useGit()

const name = ref(props.name ?? '')
const url = ref('')
const busy = ref(false)

if (props.name) {
  git.remoteUrl(props.name).then((found) => {
    if (found) url.value = found
  })
}

const adding = computed(() => !props.name)

const taken = computed(() =>
  adding.value
    ? (git.store.refs?.remotes ?? []).some((b) => b.remote === name.value.trim())
    : false
)

const invalidName = computed(() => {
  const value = name.value.trim()
  return (
    !value ||
    taken.value ||
    value.startsWith('-') ||
    value.startsWith('/') ||
    value.endsWith('/') ||
    value.includes('..') ||
    /[\s/~^:?*\[\\]/.test(value)
  )
})

const ready = computed(
  () => !invalidName.value && url.value.trim() !== '' && !busy.value
)

async function submit() {
  if (!ready.value) return
  busy.value = true
  const result = adding.value
    ? await git.remoteAdd(name.value.trim(), url.value.trim())
    : await git.remoteSetUrl(name.value.trim(), url.value.trim())
  busy.value = false
  // A failure is reported to the log by the store; the dialog stays open so
  // the address that did not work is still in front of the reader.
  if (result !== null) emit('close')
}
</script>

<template>
  <AppModal :title="adding ? 'Add a remote' : `Remote ${name}`" :width="460" @close="emit('close')">
    <label v-if="adding" class="field">
      <span class="label">Name</span>
      <input
        v-model="name"
        type="text"
        placeholder="upstream"
        autofocus
        spellcheck="false"
        @keyup.enter="submit"
      />
      <span v-if="taken" class="hint bad">A remote with that name already exists.</span>
      <span v-else-if="name && invalidName" class="hint bad">Git will not accept that name.</span>
    </label>

    <label class="field">
      <span class="label">Address</span>
      <input
        v-model="url"
        type="text"
        :autofocus="!adding"
        placeholder="git@github.com:acme/widget.git"
        spellcheck="false"
        @keyup.enter="submit"
      />
      <span class="hint faint">
        {{ adding ? 'ssh or https, pasted from the forge’s Clone button.' : 'Where this remote fetches from.' }}
      </span>
    </label>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-primary" :disabled="!ready" @click="submit">
        {{ adding ? 'Add remote' : 'Save address' }}
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

.hint.faint {
  color: var(--text-faint);
}

.bad {
  color: var(--red);
}
</style>
