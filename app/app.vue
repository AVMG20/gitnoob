<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useGit } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'
import { useForge } from '~/composables/useForge'
import { useAi } from '~/composables/useAi'
import { usePanes } from '~/composables/usePanes'

const git = useGit()
const store = git.store
const config = useConfig()
const forge = useForge()
const ai = useAi()
const { layout } = usePanes()

const ready = ref(false)
let fetchTimer: number | undefined
let unlisten: UnlistenFn | undefined

const settings = computed(() => config.settings.value)

/** Opens a project and does the on-open housekeeping GitKraken does. */
async function openProject(path: string) {
  if (!(await git.openRepo(path))) return
  await config.reload()
  await Promise.all([forge.refreshStatus(), ai.refreshStatus()])
  forge.loadReviews()
  // Fetch straight away so the ahead/behind counts on screen are true rather
  // than whatever they were when the app last ran.
  if (settings.value?.auto_fetch_on_open) git.fetch()
  scheduleFetch()
}

function scheduleFetch() {
  if (fetchTimer) window.clearInterval(fetchTimer)
  const minutes = settings.value?.auto_fetch_minutes ?? 0
  if (minutes > 0) {
    fetchTimer = window.setInterval(() => {
      if (!store.busy) git.fetch()
    }, minutes * 60_000)
  }
}

// Switching profile swaps the tab strip, so follow it to whatever is open there.
watch(
  () => config.profile.value?.id,
  async () => {
    const path = config.activeProject.value
    if (path) await openProject(path)
    else {
      store.repo = null
      store.rows = []
      store.refs = null
      store.status = null
    }
  }
)

watch(() => settings.value?.auto_fetch_minutes, scheduleFetch)

onMounted(async () => {
  // A failure here must not take the whole window down with it; without the
  // config the welcome pane is still usable.
  try {
    await config.load()
    await ai.refreshStatus()
  } catch (error) {
    git.note(`Could not read settings: ${String(error)}`, 'error')
  }

  // `gitnoob /path/to/repo` wins; otherwise reopen whatever was open last time.
  const fromArgv = await invoke<string | null>('startup_repo').catch(() => null)
  const target = fromArgv ?? config.activeProject.value
  if (target) await openProject(target)
  ready.value = true

  // The backend watches the open repository and says what kind of change it
  // saw. A write under `.git` moved refs or HEAD, so everything is rebuilt; a
  // write in the work tree only changed the status, which is far cheaper.
  unlisten = await listen<{ git_dir: boolean; work_tree: boolean }>(
    'repo-changed',
    ({ payload }) => {
      if (store.busy) return
      if (payload.git_dir) git.refresh()
      else if (payload.work_tree) git.refreshStatus()
    }
  ).catch(() => undefined)

  // Belt and braces for anything the watcher cannot see — a network share, a
  // platform without file notifications — and for the commonest case of all:
  // you left, changed something elsewhere, and came back.
  window.addEventListener('focus', onFocus)
})

function onFocus() {
  if (!store.busy && store.repo) git.refresh()
}

onUnmounted(() => {
  if (fetchTimer) window.clearInterval(fetchTimer)
  unlisten?.()
  window.removeEventListener('focus', onFocus)
})
</script>

<template>
  <div class="shell">
    <ProjectTabs @open="openProject" />

    <template v-if="store.repo">
      <TitleBar />
      <BusyBar />
      <div
        class="body"
        :style="{
          gridTemplateColumns: store.viewer
            ? `minmax(0, 1fr) 5px ${layout.panel}px`
            : `${layout.sidebar}px 5px minmax(0, 1fr) 5px ${layout.panel}px`
        }"
      >
        <!-- Opening a file takes over the graph area, as GitKraken does. -->
        <template v-if="store.viewer">
          <DiffViewer />
        </template>
        <template v-else>
          <SideBar />
          <ResizeHandle side="sidebar" />
          <GraphList />
        </template>
        <ResizeHandle side="panel" />
        <RightPanel />
      </div>
    </template>

    <WelcomePane v-else :ready="ready" @open="openProject" />

    <ActivityLog />

    <SettingsModal v-if="config.store.settingsOpen" />
    <ConflictOverlay v-if="store.resolving !== null" />
    <ContextMenu />
  </div>
</template>

<style scoped>
.shell {
  /* tabs, toolbar, progress bar, body, activity log */
  display: grid;
  grid-template-rows: auto auto auto minmax(0, 1fr) auto;
  height: 100%;
}

/* Without a repository open there is no toolbar, so the welcome pane takes the
   row the body would have had. */
.shell:has(.welcome) {
  grid-template-rows: auto minmax(0, 1fr) auto;
}

.body {
  display: grid;
  min-height: 0;
}

.body > :deep(*) {
  min-width: 0;
  min-height: 0;
}
</style>
