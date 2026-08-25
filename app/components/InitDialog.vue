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

const name = ref('')
const parent = ref('')
const error = ref('')
const busy = ref(false)

const invalid = computed(() => {
  const value = name.value.trim()
  return (
    value.startsWith('.') || value.includes('/') || value.includes('\\') || value.includes(':')
  )
})

const ready = computed(
  () => name.value.trim() !== '' && !invalid.value && parent.value !== ''
)

async function pick() {
  const path = await open({ directory: true, multiple: false, title: 'Create in' })
  if (typeof path === 'string') parent.value = path
}

async function submit() {
  if (!ready.value || busy.value) return
  busy.value = true
  error.value = ''
  try {
    const made = await invoke<NewRepo>('init_repo', {
      name: name.value.trim(),
      parent: parent.value
    })
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
  <AppModal title="New repository" :width="480" @close="emit('close')">
    <label class="field">
      <span class="label">Name</span>
      <input
        v-model="name"
        type="text"
        placeholder="widget"
        autofocus
        spellcheck="false"
        @keyup.enter="submit"
      />
      <span v-if="name && invalid" class="hint bad">
        That cannot be used as a folder name.
      </span>
      <span v-else class="hint faint">
        Starts on <span class="mono">main</span>, with a first commit ignoring the usual noise.
      </span>
    </label>

    <div class="field">
      <span class="label">Create in</span>
      <div class="dest">
        <input v-model="parent" type="text" spellcheck="false" placeholder="Choose a folder" />
        <button class="btn" title="Choose a folder" @click="pick">
          <FolderOpen :size="15" />
        </button>
      </div>
      <span v-if="parent && name.trim() && !invalid" class="hint faint">
        Creates <span class="mono">{{ parent }}/{{ name.trim() }}</span>
      </span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <p v-else-if="config.profile.value?.git_name" class="hint faint">
      Commits as {{ config.profile.value.git_name }}.
    </p>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-primary" :disabled="!ready || busy" @click="submit">
        {{ busy ? 'Creating…' : 'Create' }}
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

.bad {
  color: var(--red);
}

.error {
  margin: 0;
  font-size: 12px;
  color: var(--red);
  white-space: pre-wrap;
}
</style>
