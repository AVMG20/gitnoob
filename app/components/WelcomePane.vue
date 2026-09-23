<script setup lang="ts">
import { computed } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { ArrowRight, Download, FolderOpen, FolderPlus, GitBranch, Settings } from 'lucide-vue-next'
import { useConfig } from '~/composables/useConfig'
import { useUpdates } from '~/composables/useUpdates'
import { keyLabel } from '~/composables/useShortcuts'

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
    <div class="page">
      <!-- A start page, the way an editor has one: what this is in a line, the
           ways in as a short list, and the repositories you were last in. -->
      <header class="head">
        <span class="mark"><GitBranch :size="18" :stroke-width="2.25" /></span>
        <div>
          <h1>gitnoob</h1>
          <p class="sub">
            An open-source Git client<template v-if="config.profile.value">
              · {{ config.profile.value.name }}</template
            >
          </p>
        </div>
      </header>

      <button
        v-if="updateOffered"
        class="update"
        @click="config.openSettings('updates')"
      >
        <span class="dot" />
        {{
          updates.store.stage === 'available'
            ? `Version ${updates.store.version} is ready to install`
            : 'Installing the update…'
        }}
        <ArrowRight :size="13" />
      </button>

      <div class="columns">
        <section class="start">
          <h2>Start</h2>
          <button class="action" @click="pick">
            <FolderOpen :size="15" class="glyph" />
            <span class="label">Open a repository…</span>
            <span class="kbd">{{ keyLabel('mod+o') }}</span>
          </button>
          <button class="action" @click="emit('clone')">
            <Download :size="15" class="glyph" />
            <span class="label">Clone a repository…</span>
          </button>
          <button class="action" @click="emit('init')">
            <FolderPlus :size="15" class="glyph" />
            <span class="label">Start a new repository…</span>
          </button>
          <button class="action" @click="config.openSettings('profiles')">
            <Settings :size="15" class="glyph" />
            <span class="label">Profiles and settings</span>
          </button>
        </section>

        <section v-if="recents.length" class="recents">
          <h2>Recent <span class="pill">{{ recents.length }}</span></h2>
          <button
            v-for="project in recents"
            :key="project.path"
            class="recent"
            :title="project.path"
            @click="emit('open', project.path)"
          >
            <span class="name">{{ project.name }}</span>
            <span class="path">{{ project.path }}</span>
          </button>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* The page, not the canvas: a start page is where the work would be. The block
   sits a little above the middle and scrolls as a page when the window is
   short. */
.welcome {
  min-height: 0;
  overflow-y: auto;
  background: var(--bg);
}

.page {
  width: min(820px, 100% - 64px);
  margin: 0 auto;
  padding: clamp(32px, 14vh, 140px) 0 40px;
}

.head {
  display: flex;
  align-items: center;
  gap: 12px;
}

.mark {
  display: grid;
  place-items: center;
  flex: none;
  width: 36px;
  height: 36px;
  border-radius: var(--radius-lg);
  background: var(--primary);
  color: var(--primary-fg);
}

h1 {
  margin: 0;
  font-size: 20px;
  font-weight: 650;
  line-height: 1.2;
}

.sub {
  margin: 1px 0 0;
  color: var(--text-dim);
}

/* News, not an alarm: one line under the name, the dot doing the talking. */
.update {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  margin-top: 16px;
  padding: 4px 10px;
  border-radius: var(--radius);
  border: 1px solid var(--line);
  color: var(--text);
  font-size: 12px;
}

.update:hover {
  background: var(--bg-hover);
}

.dot {
  width: 7px;
  height: 7px;
  border-radius: var(--radius-pill);
  background: var(--success);
}

.columns {
  display: grid;
  grid-template-columns: minmax(0, 260px) minmax(0, 1fr);
  gap: 48px;
  margin-top: 40px;
}

@media (max-width: 720px) {
  .columns {
    grid-template-columns: minmax(0, 1fr);
    gap: 28px;
  }
}

h2 {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 0 6px;
  padding: 0 8px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-dim);
}

section {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.action,
.recent {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  height: 30px;
  padding: 0 8px;
  border-radius: var(--radius);
  text-align: left;
}

.action:hover,
.recent:hover {
  background: var(--bg-hover);
}

.glyph {
  flex: none;
  color: var(--accent);
}

.label {
  flex: 1;
  min-width: 0;
  color: var(--accent-soft);
  font-weight: 500;
}

.action:hover .label {
  text-decoration: underline;
  text-underline-offset: 2px;
}

.recent {
  gap: 12px;
}

.name {
  flex: none;
  max-width: 45%;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.path {
  flex: 1;
  min-width: 0;
  font-family: var(--mono);
  font-size: 11.5px;
  color: var(--text-faint);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
