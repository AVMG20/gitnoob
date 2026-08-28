<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useGit } from '~/composables/useGit'

/**
 * Adds a repository as a submodule of this one.
 *
 * Two fields, because git needs both and guessing the second one wrong puts a
 * clone somewhere you did not want it. The path is filled in from the address
 * as you type and stops following it the moment you touch it yourself.
 */
const emit = defineEmits<{ close: [] }>()

const git = useGit()

const url = ref('')
const path = ref('')
const busy = ref(false)
/** Set once the path has been typed in, so it stops tracking the address. */
const ownPath = ref(false)

/** The folder a plain `git clone` of this address would make. */
function folderFor(address: string) {
  const trimmed = address.trim().replace(/\/+$/, '')
  if (!trimmed) return ''
  const tail = trimmed.split(/[/:]/).pop() ?? ''
  return tail.replace(/\.git$/, '')
}

watch(url, (address) => {
  if (!ownPath.value) path.value = folderFor(address)
})

const taken = computed(() =>
  git.store.submodules.some((one) => one.path === path.value.trim().replace(/\/+$/, ''))
)

const badPath = computed(() => {
  const value = path.value.trim()
  return (
    !value ||
    taken.value ||
    value.startsWith('/') ||
    value.startsWith('-') ||
    value.startsWith('..') ||
    value.includes('\0')
  )
})

const ready = computed(() => url.value.trim() !== '' && !badPath.value && !busy.value)

async function submit() {
  if (!ready.value) return
  busy.value = true
  const said = await git.submoduleAdd(url.value.trim(), path.value.trim().replace(/\/+$/, ''))
  busy.value = false
  // A failure is already in the log and the corner; the dialog stays open so
  // the address that did not work is still in front of the reader.
  if (said !== null) emit('close')
}
</script>

<template>
  <AppModal title="Add a submodule" :width="480" @close="emit('close')">
    <label class="field">
      <span class="label">Address</span>
      <input
        v-model="url"
        type="text"
        placeholder="git@github.com:acme/shared.git"
        autofocus
        spellcheck="false"
        @keyup.enter="submit"
      />
      <span class="hint faint">ssh or https, pasted from the forge’s Clone button.</span>
    </label>

    <label class="field">
      <span class="label">Folder</span>
      <input
        v-model="path"
        type="text"
        placeholder="libs/shared"
        spellcheck="false"
        @input="ownPath = true"
        @keyup.enter="submit"
      />
      <span v-if="taken" class="hint bad">There is already a submodule there.</span>
      <span v-else-if="path && badPath" class="hint bad">
        It has to be a folder inside this repository.
      </span>
      <span v-else class="hint faint">
        Where inside this repository it is cloned to, and the name it is recorded under.
      </span>
    </label>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-primary" :disabled="!ready" @click="submit">
        <Spinner v-if="busy" :size="13" />
        Add and clone it
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.field {
  display: grid;
  gap: 5px;
  margin-bottom: 14px;
}

.label {
  font-size: 11px;
  color: var(--text-faint);
}

.hint {
  font-size: 11px;
}

.bad {
  color: var(--red);
}
</style>
