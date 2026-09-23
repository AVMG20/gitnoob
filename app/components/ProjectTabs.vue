<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { Download, FolderOpen, FolderPlus, GitBranch, House, Plus, X } from 'lucide-vue-next'
import { useConfig } from '~/composables/useConfig'
import { useGit } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useShortcuts } from '~/composables/useShortcuts'

const emit = defineEmits<{ open: [string]; clone: []; init: []; home: [] }>()

const props = defineProps<{ home?: boolean }>()

const config = useConfig()
const git = useGit()
const menu = useContextMenu()

const dragging = ref<string | null>(null)

/** Whether to leave the left of the strip clear: on macOS the window controls
    sit on top of it, until full screen takes them away. */
const isMac = typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac OS')
const lights = ref(isMac)
let unlistenResize: UnlistenFn | undefined

onMounted(async () => {
  // Only the Tauri host can say whether the window is full screen, and
  // `npm run dev` in a browser is not it. The check is a try as well as a
  // guard: a fixture page can put the internals object there without the
  // window API behind it, and this throwing takes the strip down with it.
  if (!isMac || !('__TAURI_INTERNALS__' in window)) return
  const win = tryWindow()
  if (!win) return
  const sync = async () => {
    lights.value = !(await win.isFullscreen().catch(() => false))
  }
  await sync()
  // Going full screen is a resize, and has no event of its own to listen to.
  unlistenResize = await win.onResized(sync).catch(() => undefined)
})

onUnmounted(() => unlistenResize?.())

/** The Tauri window, when there really is one behind the internals object. */
function tryWindow() {
  try {
    return getCurrentWindow()
  } catch {
    return null
  }
}

async function pick() {
  const path = await open({ directory: true, multiple: false, title: 'Open a repository' })
  if (typeof path === 'string') emit('open', path)
}

/**
 * The `+` is every way to end up with another repository open: pick a folder,
 * clone one by address, or start a new one. A menu rather than three buttons,
 * because the strip runs out of width before a fourth icon wants to live there.
 */
function addMenu(event: MouseEvent) {
  menu.show(event, [
    {
      label: 'Open a repository…',
      icon: FolderOpen,
      hint: 'a folder that already is one',
      action: () => {
        void pick()
      }
    },
    { label: 'Clone a repository…', icon: Download, action: () => emit('clone') },
    { label: 'New repository…', icon: FolderPlus, action: () => emit('init') }
  ])
}

async function close(path: string) {
  await config.closeProject(path)
  const next = config.activeProject.value
  if (next) emit('open', next)
  else git.forget()
}

/** Moves along the strip, wrapping, so the keys never dead-end. */
function step(by: number) {
  const paths = config.projects.value.map((p) => p.path)
  if (paths.length < 2) return
  // From the tab on screen rather than the one the config has caught up to, so
  // holding the key steps one tab per press instead of bouncing off whichever
  // open is still in flight.
  const at = paths.indexOf(config.selectedProject.value ?? '')
  const next = paths[(at + by + paths.length) % paths.length]
  if (next) emit('open', next)
}

useShortcuts({
  'project.open': () => void pick(),
  'project.close': () => {
    const path = config.activeProject.value
    if (path) void close(path)
  },
  'project.next': () => step(1),
  'project.previous': () => step(-1),
  'project.nth': (index: number) => {
    const path = config.projects.value[index]?.path
    // While home is up its own tab is the one on screen, so even the project
    // the strip has marked is somewhere to go.
    if (path && (props.home || path !== config.selectedProject.value)) emit('open', path)
  }
})

function onDrop(target: string) {
  const from = dragging.value
  dragging.value = null
  if (!from || from === target) return
  const order = config.projects.value.map((p) => p.path)
  const without = order.filter((p) => p !== from)
  const at = without.indexOf(target)
  without.splice(at, 0, from)
  config.reorderProjects(without)
}
</script>

<template>
  <!-- Dragging the strip moves the window; the tabs and buttons keep their clicks. -->
  <nav class="strip" :class="{ lights }" data-tauri-drag-region>
    <!-- Home stands where a browser puts it: left of the tabs, always there,
         with no close on it. It is the one page that is about all of them. -->
    <button
      class="icon"
      :class="{ on: props.home }"
      title="Home — every project, and the year"
      @click="emit('home')"
    >
      <House :size="15" />
    </button>
    <button class="icon" title="Open a repository" @click="pick">
      <FolderOpen :size="15" />
    </button>

    <div class="tabs">
      <button
        v-for="project in config.projects.value"
        :key="project.path"
        class="tab"
        :class="{
          // Home is the page on screen while it is up, so no tab is: two lit
          // tabs at once say the window is in both places.
          on: !props.home && project.path === config.selectedProject.value,
          drag: dragging === project.path
        }"
        :title="project.path"
        draggable="true"
        @click="emit('open', project.path)"
        @dragstart="dragging = project.path"
        @dragend="dragging = null"
        @dragover.prevent
        @drop.prevent="onDrop(project.path)"
      >
        <GitBranch :size="13" class="tab-icon" />
        <span class="tab-name">{{ project.name }}</span>
        <span class="close" title="Close" @click.stop="close(project.path)">
          <X :size="12" />
        </span>
      </button>
    </div>

    <button class="icon" title="Open another repository" @click="addMenu">
      <Plus :size="15" />
    </button>
  </nav>
</template>

<style scoped>
/* The strip is the deepest tone in the window. The open tab is the toolbar's
   tone and runs into it through the strip's bottom line, so the tab and the
   toolbar under it read as one piece: this is the repository the bar is for. */
.strip {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  padding: 5px 8px 0 6px;
  min-height: 38px;
  background: var(--deep);
  border-bottom: 1px solid var(--line);
}

/* Clear of the window controls, which the config parks at x: 13. */
.strip.lights {
  padding-left: 78px;
}

.icon {
  display: grid;
  place-items: center;
  flex: none;
  align-self: center;
  width: 28px;
  height: 28px;
  margin-top: -5px;
  color: var(--text-faint);
  border-radius: var(--radius);
}

.icon:hover {
  color: var(--text);
  background: var(--bg-hover);
}

/* Marked the way an open tab is, because that is what it is while it is up. */
.icon.on {
  color: var(--accent);
  background: var(--primary-bg);
}

.tabs {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  min-width: 0;
  margin: 0 2px 0 4px;
  overflow-x: auto;
  overflow-y: hidden;
}

/* The pseudo-element alone. `scrollbar-width: none` hides it too, but it also
   opts this scroller into the platform's own rendering, which on GTK is an
   overlay bar painted above whatever is on top of it. */
.tabs::-webkit-scrollbar {
  display: none;
}

.tab {
  position: relative;
  display: flex;
  align-items: center;
  gap: 7px;
  height: 32px;
  padding: 0 6px 0 12px;
  max-width: 200px;
  flex: none;
  color: var(--text-dim);
  border: 1px solid transparent;
  border-bottom: none;
  border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  white-space: nowrap;
}

.tab:hover {
  background: color-mix(in srgb, var(--canvas) 55%, var(--deep));
  color: var(--text);
}

.tab.on {
  z-index: 1;
  margin-bottom: -1px;
  height: 33px;
  background: var(--canvas);
  border-color: var(--line);
  color: var(--text);
  font-weight: 600;
}

.tab.drag {
  opacity: 0.4;
}

.tab-icon {
  flex: none;
  opacity: 0.6;
}

.tab.on .tab-icon {
  color: var(--accent);
  opacity: 1;
}

.tab-name {
  overflow: hidden;
  text-overflow: ellipsis;
}

.close {
  display: grid;
  place-items: center;
  width: 18px;
  height: 18px;
  border-radius: var(--radius-sm);
  opacity: 0;
  flex: none;
}

.tab:hover .close,
.tab.on .close {
  opacity: 0.55;
}

.close:hover {
  opacity: 1;
  background: var(--bg-hover);
}
</style>
