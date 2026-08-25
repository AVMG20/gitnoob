<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useGit } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'
import { useForge } from '~/composables/useForge'
import { useAi } from '~/composables/useAi'
import { usePanes } from '~/composables/usePanes'
import { useTheme } from '~/composables/useTheme'

const git = useGit()
const store = git.store
const config = useConfig()
const forge = useForge()
const ai = useAi()
const { layout } = usePanes()
// Applied as a side effect of loading the composable, before the first paint.
useTheme()

const ready = ref(false)
let fetchTimer: number | undefined
let unlisten: UnlistenFn | undefined
let unlistenCommand: UnlistenFn | undefined

/** The clone and new-repository dialogs, reachable from the welcome pane. */
const cloneOpen = ref(false)
const initOpen = ref(false)

/** A repository that has just been cloned or created opens like any other. */
async function made(path: string) {
  cloneOpen.value = false
  initOpen.value = false
  await openProject(path)
}

/**
 * Changes seen while the app was busy.
 *
 * A watcher event that arrives mid-operation used to be dropped, and the next
 * one might be minutes away — long enough to sit looking at a stale window. So
 * it is remembered instead, and acted on when the work finishes.
 */
const pending = { gitDir: false, workTree: false }

function drain() {
  if (store.busy || !store.repo) return
  const { gitDir, workTree } = pending
  pending.gitDir = false
  pending.workTree = false
  // A write under `.git` moved refs or HEAD, so everything is rebuilt; a write
  // in the work tree only changed the status, which is far cheaper.
  if (gitDir) git.refresh()
  else if (workTree) git.refreshStatus()
}

// Whatever the watcher saw while an operation was running is applied the moment
// the last one finishes.
watch(
  () => store.busy,
  (busy) => {
    if (!busy) drain()
  }
)

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
      if (payload.git_dir) pending.gitDir = true
      if (payload.work_tree) pending.workTree = true
      drain()
    }
  ).catch(() => undefined)

  // Every git command the backend runs, so the log doubles as a lesson in what
  // the buttons do.
  unlistenCommand = await listen<{ line: string; ok: boolean }>(
    'git-command',
    ({ payload }) => git.note(payload.line, payload.ok ? 'command' : 'error')
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
  unlistenCommand?.()
  window.removeEventListener('focus', onFocus)
})
</script>

<template>
  <div class="shell">
    <ProjectTabs @open="openProject" @clone="cloneOpen = true" @init="initOpen = true" />

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

    <WelcomePane
      v-else
      :ready="ready"
      @open="openProject"
      @clone="cloneOpen = true"
      @init="initOpen = true"
    />

    <ActivityLog />

    <SettingsModal v-if="config.store.settingsOpen" />
    <CloneDialog v-if="cloneOpen" @close="cloneOpen = false" @done="made" />
    <InitDialog v-if="initOpen" @close="initOpen = false" @done="made" />
    <ConflictOverlay v-if="store.resolving !== null" />
    <ContextMenu />
  </div>
</template>

<style scoped>
.shell {
  /* tabs, toolbar, progress bar, body, activity log */
  display: grid;
  grid-template-columns: minmax(0, 1fr);
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
