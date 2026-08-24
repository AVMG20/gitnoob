<script setup lang="ts">
import { ref } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { FolderOpen, GitBranch, Plus, X } from 'lucide-vue-next'
import { useConfig } from '~/composables/useConfig'
import { useGit } from '~/composables/useGit'

const emit = defineEmits<{ open: [string] }>()

const config = useConfig()
const git = useGit()

const dragging = ref<string | null>(null)

async function pick() {
  const path = await open({ directory: true, multiple: false, title: 'Open a repository' })
  if (typeof path === 'string') emit('open', path)
}

async function close(path: string) {
  await config.closeProject(path)
  const next = config.activeProject.value
  if (next) emit('open', next)
  else {
    git.store.repo = null
    git.store.rows = []
  }
}

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
  <nav class="strip">
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

    <button class="icon" title="Open another repository" @click="pick">
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
  background: #10141a;
  border-bottom: 1px solid var(--line);
  min-height: 36px;
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
  scrollbar-width: none;
}

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
