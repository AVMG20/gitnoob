<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { Download, FolderOpen, FolderPlus, GitBranch, Plus, X } from 'lucide-vue-next'
import { useConfig } from '~/composables/useConfig'
import { useGit } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useShortcuts } from '~/composables/useShortcuts'

const emit = defineEmits<{ open: [string]; clone: []; init: [] }>()

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
  // `npm run dev` in a browser is not it.
  if (!isMac || !('__TAURI_INTERNALS__' in window)) return
  const win = getCurrentWindow()
  const sync = async () => {
    lights.value = !(await win.isFullscreen().catch(() => false))
  }
  await sync()
  // Going full screen is a resize, and has no event of its own to listen to.
  unlistenResize = await win.onResized(sync).catch(() => undefined)
})

onUnmounted(() => unlistenResize?.())

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
  const at = paths.indexOf(config.activeProject.value ?? '')
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
    if (path && path !== config.activeProject.value) emit('open', path)
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
    <button class="icon" title="Open a repository" @click="pick">
      <FolderOpen :size="15" />
    </button>

    <div class="tabs">
      <button
        v-for="project in config.projects.value"
        :key="project.path"
        class="tab"
        :class="{ on: project.path === config.activeProject.value, drag: dragging === project.path }"
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
.strip {
  display: flex;
  align-items: stretch;
  gap: 2px;
  padding: 0 6px 0 4px;
  background: var(--bg-deep);
  border-bottom: 1px solid var(--line);
  min-height: 38px;
}

/* Clear of the window controls, which the config parks at x: 13. */
.strip.lights {
  padding-left: 78px;
}

.icon {
  display: grid;
  place-items: center;
  width: 30px;
  color: var(--text-faint);
  border-radius: 5px;
  margin: 4px 0;
}

.icon:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.tabs {
  display: flex;
  align-items: stretch;
  gap: 2px;
  overflow-x: auto;
}

/* The pseudo-element alone. `scrollbar-width: none` hides it too, but it also
   opts this scroller into the platform's own rendering, which on GTK is an
   overlay bar painted above whatever is on top of it. */
.tabs::-webkit-scrollbar {
  display: none;
}

.tab {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 0 8px 0 11px;
  max-width: 190px;
  color: var(--text-dim);
  border-top: 2px solid transparent;
  white-space: nowrap;
}

.tab:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.tab.on {
  background: var(--bg-panel);
  color: var(--text);
  border-top-color: var(--accent);
  font-weight: 600;
}

.tab.drag {
  opacity: 0.4;
}

.tab-icon {
  flex: none;
  opacity: 0.65;
}

.tab-name {
  overflow: hidden;
  text-overflow: ellipsis;
}

.close {
  display: grid;
  place-items: center;
  width: 17px;
  height: 17px;
  border-radius: 4px;
  opacity: 0;
  flex: none;
}

.tab:hover .close,
.tab.on .close {
  opacity: 0.55;
}

.close:hover {
  opacity: 1;
  background: var(--bg-active);
}
</style>
