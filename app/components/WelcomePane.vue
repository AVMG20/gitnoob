<script setup lang="ts">
import { computed } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { Download, FolderOpen, FolderPlus, GitBranch, Settings } from 'lucide-vue-next'
import { useConfig } from '~/composables/useConfig'
import { useUpdates } from '~/composables/useUpdates'

defineProps<{ ready: boolean }>()
const emit = defineEmits<{ open: [string]; clone: []; init: [] }>()

const config = useConfig()
const updates = useUpdates()

/**
 * The toolbar that normally carries the Update button is not on screen here,
 * so this pane says it instead — otherwise the one place you land with no
 * repository open is the one place a waiting release is invisible.
 */
const updateOffered = computed(() =>
  ['available', 'downloading', 'ready'].includes(updates.store.stage)
)

/** The tabs open now, then everything opened before them. */
const recents = computed(() => {
  const open = config.projects.value
  const seen = new Set(open.map((one) => one.path))
  return [...open, ...config.recents.value.filter((one) => !seen.has(one.path))].slice(0, 12)
})

async function pick() {
  const path = await open({ directory: true, multiple: false, title: 'Open a repository' })
  if (typeof path === 'string') emit('open', path)
}
</script>

<template>
  <div class="welcome">
    <div class="card">
      <h1>gitnoob</h1>
      <p class="dim sub">
        An open-source Git client<template v-if="config.profile.value">
          — {{ config.profile.value.name }}</template
        >.
      </p>

      <button class="btn btn-primary wide" @click="pick">
        <FolderOpen :size="15" /> Open a repository
      </button>

      <div class="pair">
        <button class="btn" @click="emit('clone')">
          <Download :size="15" /> Clone
        </button>
        <button class="btn" @click="emit('init')">
          <FolderPlus :size="15" /> New
        </button>
      </div>

      <div v-if="recents.length" class="recents">
        <div class="section-title">In this profile</div>
        <button
          v-for="project in recents"
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

      <button
        v-if="updateOffered"
        class="btn update"
        @click="config.openSettings('updates')"
      >
        <Download :size="14" />
        {{
          updates.store.stage === 'available'
            ? `Version ${updates.store.version} is ready to install`
            : 'Installing the update…'
        }}
      </button>

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

.pair {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}

.pair .btn {
  flex: 1;
  justify-content: center;
  padding: 8px;
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

/* Same tint as the toolbar's Update button, so it reads as the same thing in
   the one place that toolbar is missing. */
.update {
  width: 100%;
  justify-content: center;
  margin-top: 16px;
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
  font-weight: 600;
}

.update:hover {
  background: color-mix(in srgb, var(--accent) 26%, transparent);
  color: var(--accent);
}
</style>