<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { FolderOpen } from 'lucide-vue-next'
import { useGit, type NewRepo } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'
import { useForge } from '~/composables/useForge'
import SearchSelect, { type Choice } from '~/components/SearchSelect.vue'

const emit = defineEmits<{ close: []; done: [string] }>()

const git = useGit()
const config = useConfig()
const forge = useForge()

const url = ref('')
const parent = ref('')
const error = ref('')
const busy = ref(false)
const picked = ref<string | null>(null)

/** Mirrors the backend's naming, so the folder is named before it is fetched. */
function folderName(value: string): string {
  const trimmed = value.trim().replace(/\/+$/, '')
  const withoutScheme = trimmed.split('://').pop() ?? trimmed
  return (withoutScheme.split(/[/:]/).filter(Boolean).pop() ?? '').replace(/\.git$/, '')
}

const name = computed(() => folderName(url.value))
const ready = computed(() => url.value.trim() !== '' && parent.value !== '' && name.value !== '')

/** The repositories the profile's token can see, as picker rows. */
const choices = computed<Choice[]>(() =>
  forge.store.repos.map((repo) => ({
    value: repo.ssh_url || repo.https_url,
    label: repo.name,
    note: repo.owner,
    hint: repo.full_name
  }))
)

const forgeLabel = computed(() =>
  forge.store.status?.kind === 'gitlab' ? 'GitLab' : 'GitHub'
)

function pickRepo(value: string) {
  picked.value = value
  url.value = value
}

onMounted(async () => {
  // The dialog is reached from the welcome pane, where no repository is open
  // and the status has not been read this session. `forge_status` asks only
  // the config, so it works with nothing open.
  if (!forge.store.status) await forge.refreshStatus().catch(() => null)
  if (forge.store.status?.has_token && forge.store.status.kind !== 'none') {
    await forge.loadRepos().catch(() => null)
  }
})

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
    <div v-if="choices.length" class="field">
      <span class="label">Your {{ forgeLabel }} repositories</span>
      <SearchSelect
        :model-value="picked"
        :options="choices"
        placeholder="Pick one…"
        empty="Nothing of yours matches that."
        @update:model-value="pickRepo"
      />
      <span class="hint faint">Picking fills the address below; edit it if you would rather.</span>
    </div>
    <p v-else-if="forge.store.reposError" class="hint error-line">
      Could not list your repositories: {{ forge.store.reposError }}
    </p>
    <p v-else-if="forge.store.loadingRepos" class="hint faint">Listing your repositories…</p>

    <label class="field">
      <span class="label">Repository address</span>
      <input
        v-model="url"
        type="text"
        placeholder="git@github.com:acme/widget.git"
        :autofocus="!choices.length"
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

.error-line {
  color: var(--red);
}

.error {
  margin: 0;
  font-size: 12px;
  color: var(--red);
  white-space: pre-wrap;
}
</style>
