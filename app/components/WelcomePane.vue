<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { FolderOpen, GitBranch, Settings } from 'lucide-vue-next'
import { useConfig } from '~/composables/useConfig'

defineProps<{ ready: boolean }>()
const emit = defineEmits<{ open: [string] }>()

const config = useConfig()

async function pick() {
  const path = await open({ directory: true, multiple: false, title: 'Open a repository' })
  if (typeof path === 'string') emit('open', path)
}
</script>

<template>
  <div class="welcome">
    <div class="card">
      <h1>gitui</h1>
      <p class="dim sub">
        An open-source Git client<template v-if="config.profile.value">
          — {{ config.profile.value.name }}</template
        >.
      </p>

      <button class="btn btn-primary wide" @click="pick">
        <FolderOpen :size="15" /> Open a repository
      </button>

      <div v-if="config.projects.value.length" class="recents">
        <div class="section-title">In this profile</div>
        <button
          v-for="project in config.projects.value"
          :key="project.path"
          class="recent"
          @click="emit('open', project.path)"
        >
          <GitBranch :size="14" class="dim" />
          <span class="names">
            <strong>{{ project.name }}</strong>
            <span class="faint path">{{ project.path }}</span>
          </span>
        </button>
      </div>

      <button class="btn settings" @click="config.openSettings('profiles')">
        <Settings :size="14" /> Profiles and settings
      </button>
    </div>
  </div>
</template>

<style scoped>
.welcome {
  display: grid;
  place-items: center;
  min-height: 0;
}

.card {
  width: 440px;
}

h1 {
  margin: 0;
  font-size: 30px;
  letter-spacing: -0.02em;
}

.sub {
  margin: 4px 0 20px;
}

.wide {
  width: 100%;
  justify-content: center;
  padding: 9px;
}

.recents {
  margin-top: 24px;
  border-top: 1px solid var(--line);
  padding-top: 4px;
  max-height: 260px;
  overflow-y: auto;
}

.recent {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 7px 10px;
  border-radius: 6px;
  text-align: left;
}

.recent:hover {
  background: var(--bg-hover);
}

.names {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.path {
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.settings {
  margin-top: 18px;
  padding-left: 0;
}
</style>
