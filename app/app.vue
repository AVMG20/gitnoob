<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '~/composables/useInvoke'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useGit } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'
import { useForge } from '~/composables/useForge'
import { useAi } from '~/composables/useAi'
import { usePanes } from '~/composables/usePanes'
import { useReview } from '~/composables/useReview'
import { useTheme } from '~/composables/useTheme'
import { useUpdates } from '~/composables/useUpdates'
import { useShortcuts } from '~/composables/useShortcuts'
import { useZoom } from '~/composables/useZoom'

const git = useGit()
const store = git.store
const config = useConfig()
const forge = useForge()
const review = useReview()
const ai = useAi()
const { layout } = usePanes()
const updates = useUpdates()
// Both applied as a side effect of loading the composable, before the first
// paint, so neither the theme nor the size is seen changing.
useTheme()
const zoom = useZoom()

useShortcuts({
  'zoom.in': zoom.zoomIn,
  'zoom.out': zoom.zoomOut,
  'zoom.reset': zoom.reset
})

const ready = ref(false)

/**
 * The review page on fixture data, at `?lab=review` on the dev server.
 *
 * A Tauri window is the only place the real app can run, which makes looking
 * at a page in a browser — at another width, with the devtools open, on a
 * review nobody has to have open — impossible. This is the way in, and
 * `import.meta.dev` keeps it out of anything built for release.
 */
const labKind = import.meta.dev
  ? new URLSearchParams(window.location.search).get('lab')
  : null
const lab = labKind === 'review' || labKind === 'conflict'

// Loaded only where it can be reached, so the fixture review is not bundled
// into anything shipped: with `import.meta.dev` false the import goes with it.
const DevReviewLab = import.meta.dev
  ? defineAsyncComponent(() => import('~/components/DevReviewLab.vue'))
  : null
/** The conflict resolver on a fixture merge, at `?lab=conflict`. */
const DevConflictLab = import.meta.dev
  ? defineAsyncComponent(() => import('~/components/DevConflictLab.vue'))
  : null
const labPage = computed(() => (labKind === 'conflict' ? DevConflictLab : DevReviewLab))

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

/**
 * The review's files stay in the panel wherever in the review you are.
 *
 * The list is how a review is walked, not only how a diff is chosen: a file
 * clicked from the conversation or the checks page opens the files page on
 * it, and the panel never appears and disappears under the pointer.
 */
const reviewOpen = computed(() => !!review.store.current)

/**
 * The columns of the window, and what gives when there is not enough of it.
 *
 * Every fixed track is `minmax(0, …)` so it shrinks rather than pushing the
 * grid wider than the window: at a stated width they kept it, the middle column
 * hit zero, and the panel carried on off the right-hand edge.
 */
const columns = computed(() => {
  const panel = `minmax(0, ${layout.panel}px)`
  if (store.viewer || reviewOpen.value) return `minmax(0, 1fr) 5px ${panel}`
  return `minmax(0, ${layout.sidebar}px) 5px minmax(0, 1fr) 5px ${panel}`
})

/** Opens a project and does the on-open housekeeping GitKraken does. */
async function openProject(path: string) {
  // Before the first await, so the tab strip moves in the same frame as the
  // click rather than two round trips later. The work below is quick; it was
  // waiting for it to finish that made switching feel slow.
  config.beginOpen(path)
  try {
    if (!(await git.openRepo(path))) return
    await config.reload()
  } finally {
    config.endOpen()
  }
  // A review page belongs to the repository it was opened on.
  review.close()
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
      git.forget()
      review.close()
    }
  }
)

watch(() => settings.value?.auto_fetch_minutes, scheduleFetch)

/** Follows the setting: switching it off stops the schedule, not only the
    next launch. */
function watchUpdates() {
  if (settings.value?.check_updates !== false) updates.watchForUpdates()
  else updates.stopWatching()
}

watch(() => settings.value?.check_updates, watchUpdates)

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
    ({ payload }) => git.note(payload.line, payload.ok ? 'command' : 'failed')
  ).catch(() => undefined)

  // Whether a newer release exists, asked quietly and kept asking: a machine
  // that is offline should not be told so every time the window opens. What
  // it finds shows up as a button in the toolbar and a dot next to Updates in
  // settings, not as a dialog over the repository you came here to look at.
  watchUpdates()

  // Belt and braces for anything the watcher cannot see — a network share, a
  // platform without file notifications — and for the commonest case of all:
  // you left, changed something elsewhere, and came back.
  window.addEventListener('focus', onFocus)
})

/** When the window was last read on being focused. */
let lastFocusRead = 0

/**
 * Cmd-tabbing through windows, and a dialog closing, both fire focus — several
 * times a second in the first case. Reading the whole repository each time is
 * work nobody asked for, and one read per visit answers the question just as
 * well.
 */
function onFocus() {
  if (store.busy || !store.repo) return
  const now = Date.now()
  if (now - lastFocusRead < 2000) return
  lastFocusRead = now
  git.refresh()
}

onUnmounted(() => {
  if (fetchTimer) window.clearInterval(fetchTimer)
  unlisten?.()
  unlistenCommand?.()
  updates.stopWatching()
  window.removeEventListener('focus', onFocus)
})
</script>

<template>
  <component :is="labPage" v-if="lab && labPage" />
  <div v-else class="shell">
    <ProjectTabs @open="openProject" @clone="cloneOpen = true" @init="initOpen = true" />

    <template v-if="store.repo">
      <!-- Fetch, pull, push and stash are about the working tree, and a review
           page is not the working tree: the toolbar stands down while one is
           open, and Back brings it straight back. Hidden rather than unmounted,
           because the repository's keyboard shortcuts live in it and those still
           work from a review page. -->
      <TitleBar v-show="!review.store.current" />
      <BusyBar />
      <div class="body" :style="{ gridTemplateColumns: columns }">
        <!-- Opening a file takes over the graph area, as GitKraken does; so
             does opening a review, which is read here rather than in the
             forge's browser tab. -->
        <template v-if="store.viewer">
          <DiffViewer />
        </template>
        <template v-else-if="review.store.current">
          <ReviewPane />
        </template>
        <template v-else>
          <SideBar @open="openProject" />
          <ResizeHandle side="sidebar" />
          <GraphList />
        </template>
        <ResizeHandle side="panel" />
        <!-- While a review is being read, the panel holds its files: the
             working tree has nothing to say about somebody else's branch,
             and the list is how the review itself is walked. -->
        <ReviewFilesPanel v-if="reviewOpen" />
        <RightPanel v-else />
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
    <Toasts />

    <SettingsModal v-if="config.store.settingsOpen" />
    <CloneDialog v-if="cloneOpen" @close="cloneOpen = false" @done="made" />
    <InitDialog v-if="initOpen" @close="initOpen = false" @done="made" />
    <ConflictOverlay v-if="store.resolving !== null" />
    <ContextMenu />
  </div>
</template>

<style scoped>
/*
 * Tabs, toolbar, progress bar, body, activity log — stacked, with the body
 * taking whatever is left.
 *
 * A column of rows named by position broke the moment one of them could be
 * absent: with the toolbar hidden on a review page, everything below it moved
 * up a row and the activity log inherited the row that stretches. Flex says
 * "the body is the one that grows" without counting anybody.
 */
.shell {
  display: flex;
  flex-direction: column;
  height: 100%;
}

.body,
.shell > :deep(.welcome) {
  flex: 1;
  min-height: 0;
}

.body {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
}

.body > :deep(*) {
  min-width: 0;
  min-height: 0;
}
</style>
