<script setup lang="ts">
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { FolderOpen } from 'lucide-vue-next'
import { useGit, type NewRepo } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'

const emit = defineEmits<{ close: []; done: [string] }>()

const git = useGit()
const config = useConfig()

const url = ref('')
const parent = ref('')
const error = ref('')
const busy = ref(false)

/** Mirrors the backend's naming, so the folder is named before it is fetched. */
function folderName(value: string): string {
  const trimmed = value.trim().replace(/\/+$/, '')
  const withoutScheme = trimmed.split('://').pop() ?? trimmed
  return (withoutScheme.split(/[/:]/).filter(Boolean).pop() ?? '').replace(/\.git$/, '')
}

const name = computed(() => folderName(url.value))
const ready = computed(() => url.value.trim() !== '' && parent.value !== '' && name.value !== '')

async function pick() {
  const path = await open({ directory: true, multiple: false, title: 'Clone into' })
  if (typeof path === 'string') parent.value = path
}

async function submit() {
  if (!ready.value || busy.value) return
  busy.value = true
  error.value = ''
  try {
    const made = await invoke<NewRepo>('clone_repo', { url: url.value, parent: parent.value })
    if (made.note) git.note(made.note)
    emit('done', made.path)
  } catch (e) {
    error.value = String(e)
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <AppModal title="Clone a repository" :width="480" @close="emit('close')">
    <label class="field">
      <span class="label">Repository address</span>
      <input
        v-model="url"
        type="text"
        placeholder="git@github.com:acme/widget.git"
        autofocus
        spellcheck="false"
        @keyup.enter="submit"
      />
      <span class="hint faint">ssh or https, pasted from the forge's Clone button.</span>
    </label>

    <div class="field">
      <span class="label">Clone into</span>
      <div class="dest">
        <input v-model="parent" type="text" spellcheck="false" placeholder="Choose a folder" />
        <button class="btn" title="Choose a folder" @click="pick">
          <FolderOpen :size="15" />
        </button>
      </div>
      <span v-if="parent && name" class="hint faint">
        Clones into <span class="mono">{{ parent }}/{{ name }}</span>
      </span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <p v-else-if="config.profile.value?.ssh_key" class="hint faint">
      Over ssh this profile's key is offered, and no other.
    </p>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-primary" :disabled="!ready || busy" @click="submit">
        {{ busy ? 'Cloning…' : 'Clone' }}
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

.dest {
  display: flex;
  gap: 6px;
}

.hint {
  display: block;
  margin-top: 4px;
  font-size: 11px;
}

.hint.faint {
  color: var(--text-faint);
}

.error {
  margin: 0;
  font-size: 12px;
  color: var(--red);
  white-space: pre-wrap;
}
</style>
