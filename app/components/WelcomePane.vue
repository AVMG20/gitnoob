<script setup lang="ts">
import { computed } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { ArrowRight, Download, FolderOpen, FolderPlus, GitBranch, Settings } from 'lucide-vue-next'
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
    <div class="page">
      <!-- The front of the shop: what this is, and the one thing to do next,
           filled in ink. Clone and New sit beside it as the alternatives. -->
      <header class="hero">
        <span class="mark"><GitBranch :size="22" :stroke-width="2.25" /></span>
        <h1>{{ recents.length ? 'Pick up where you left off' : 'Welcome to gitnoob' }}</h1>
        <p class="sub">
          An open-source Git client<template v-if="config.profile.value">
            · {{ config.profile.value.name }}</template
          >
        </p>

        <div class="cta">
          <button class="btn btn-primary big" @click="pick">
            <FolderOpen :size="15" /> Open a repository
          </button>
          <button class="btn btn-ghost big" @click="emit('clone')">
            <Download :size="15" /> Clone
          </button>
          <button class="btn btn-ghost big" @click="emit('init')">
            <FolderPlus :size="15" /> New
          </button>
        </div>

        <button
          v-if="updateOffered"
          class="btn update"
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
      </header>

      <section v-if="recents.length" class="recents">
        <div class="head">
          <h2>In this profile</h2>
          <span class="count">{{ recents.length }}</span>
        </div>
        <div class="grid">
          <button
            v-for="project in recents"
            :key="project.path"
            class="recent"
            :title="project.path"
            @click="emit('open', project.path)"
          >
            <span class="tile"><GitBranch :size="15" /></span>
            <span class="names">
              <strong>{{ project.name }}</strong>
              <span class="path">{{ project.path }}</span>
            </span>
            <ArrowRight :size="14" class="go" />
          </button>
        </div>
      </section>

      <button class="btn settings" @click="config.openSettings('profiles')">
        <Settings :size="14" /> Profiles and settings
      </button>
    </div>
  </div>
</template>

<style scoped>
/* Straight on the canvas: the hero is type on the page, the recent projects
   are cards resting on it. Scrolls as a page when the window is short. */
.welcome {
  min-height: 0;
  overflow-y: auto;
}

.page {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: min(760px, 100% - 48px);
  margin: 0 auto;
  padding: clamp(32px, 11vh, 110px) 0 40px;
}

.hero {
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
}

.mark {
  display: grid;
  place-items: center;
  width: 48px;
  height: 48px;
  margin-bottom: 20px;
  border-radius: 14px;
  background: var(--primary);
  color: var(--primary-fg);
  box-shadow: var(--shadow-pop);
}

h1 {
  margin: 0;
  font-size: 30px;
  font-weight: 650;
  line-height: 1.15;
  letter-spacing: -0.03em;
}

.sub {
  margin: 8px 0 26px;
  font-size: 14px;
  color: var(--text-dim);
}

.cta {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 8px;
}

.big {
  min-height: 40px;
  padding: 8px 20px;
  font-size: 13.5px;
}

.btn-ghost.big {
  background: var(--bg);
}

/* News, not an alarm: a small pill under the buttons, the dot doing the
   talking. */
.update {
  margin-top: 18px;
  padding: 5px 12px;
  border-radius: var(--radius-pill);
  background: var(--bg);
  box-shadow: var(--shadow-card);
  color: var(--text);
  font-size: 12px;
}

.update:hover:not(:disabled) {
  background: var(--bg);
  box-shadow: var(--shadow-pop);
}

.dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--success);
}

.recents {
  width: 100%;
  margin-top: 56px;
}

.head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0 2px 12px;
}

h2 {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  letter-spacing: -0.01em;
}

.count {
  padding: 0 7px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--text) 7%, transparent);
  color: var(--text-dim);
  font-size: 11px;
  font-weight: 600;
  line-height: 18px;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 12px;
}

.recent {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
  padding: 14px;
  border-radius: var(--radius-lg);
  background: var(--bg);
  box-shadow: var(--shadow-card);
  text-align: left;
  transition:
    box-shadow 0.15s,
    transform 0.15s;
}

.recent:hover {
  box-shadow: var(--shadow-pop);
  transform: translateY(-1px);
}

.tile {
  display: grid;
  place-items: center;
  flex: none;
  width: 34px;
  height: 34px;
  border-radius: var(--radius);
  background: var(--bg-raised);
  color: var(--text-dim);
}

.names {
  display: flex;
  flex-direction: column;
  gap: 1px;
  flex: 1;
  min-width: 0;
}

.names strong {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}

.path {
  font-size: 11.5px;
  color: var(--text-faint);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
}

.go {
  flex: none;
  color: var(--text-faint);
  opacity: 0;
  transform: translateX(-4px);
  transition:
    opacity 0.15s,
    transform 0.15s;
}

.recent:hover .go {
  opacity: 1;
  transform: none;
}

.settings {
  margin-top: 32px;
  border-radius: var(--radius-pill);
}

.settings:hover:not(:disabled) {
  background: color-mix(in srgb, var(--text) 7%, transparent);
}
</style>
