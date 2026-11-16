<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { ChevronLeft, ChevronRight, Copy, FolderTree, GitBranch, X } from 'lucide-vue-next'
import { copyText, useGit } from '~/composables/useGit'
import { change } from '~/composables/useFileView'
import FileIcon from '~/components/FileIcon.vue'
import { useContextMenu, type MenuItem } from '~/composables/useContextMenu'
import { keyLabel, keysFor, useShortcuts } from '~/composables/useShortcuts'
import { useTabs, type Tab } from '~/composables/useTabs'
import { revealInFiles } from '~/composables/useFileTree'
import { closeTabsSafely } from '~/composables/useFileEdit'
import { isDependencyPath } from '~/composables/fileHeader'

/**
 * The strip over the middle of the window: back and forward, the graph, and
 * every file that is open.
 *
 * It is only there while a file is: with nothing open the window is the graph
 * and nothing else, exactly as before there were tabs.
 */

const git = useGit()
const store = git.store
const tabs = useTabs()
const menu = useContextMenu()

const shown = computed(() => tabs.state.tabs.length > 0)
const strip = ref<HTMLElement | null>(null)

const LETTERS = { added: 'A', modified: 'M', deleted: 'D', renamed: 'R' } as const

/** The letter the right panel would give the file, and its colour. */
function status(tab: Tab): { letter: string; kind: string } | null {
  if (tab.commit) {
    if (store.detail?.oid !== tab.commit) return null
    const file = store.detail.files.find((one) => one.path === tab.path)
    if (!file) return null
    const kind = change(file.status)
    return { letter: LETTERS[kind], kind }
  }
  if (store.status?.conflicted.includes(tab.path)) return { letter: '!', kind: 'clash' }
  const lists =
    tab.side === 'staged'
      ? [store.status?.staged, store.status?.unstaged]
      : [store.status?.unstaged, store.status?.staged]
  for (const list of lists) {
    const entry = (list ?? []).find((one) => one.path === tab.path)
    if (entry) {
      const kind = change(entry.kind)
      return { letter: LETTERS[kind], kind }
    }
  }
  return null
}

function base(path: string) {
  return path.slice(path.lastIndexOf('/') + 1)
}

function title(tab: Tab) {
  const where = (tab.commit ? ` at ${tab.commit.slice(0, 7)}` : '') + (isDependencyPath(tab.path) ? ' (dependency, read-only)' : '')
  const how = tab.preview ? ' — preview, double-click to keep it' : ''
  return `${tab.path}${where}${how}\nMiddle-click or ⇧-click to close`
}

/** A click: ⇧ closes the tab, as a middle click does, without going to it first. */
function onTabClick(event: MouseEvent, tab: Tab) {
  if (event.shiftKey) {
    void closeTabsSafely([tab.id], () => tabs.closeTab(tab.id))
    return
  }
  tabs.selectTab(tab.id)
}

function menuFor(event: MouseEvent, tab: Tab) {
  const items: MenuItem[] = [
    { label: 'Close', icon: X, hint: keyLabel(keysFor('project.close')), action: () => void closeTabsSafely([tab.id], () => tabs.closeTab(tab.id)) },
    {
      label: 'Close others',
      disabled: tabs.state.tabs.length < 2,
      action: () =>
        void closeTabsSafely(
          tabs.state.tabs.filter((one) => one.id !== tab.id).map((one) => one.id),
          () => tabs.closeOthers(tab.id)
        )
    },
    {
      label: 'Close all',
      action: () =>
        void closeTabsSafely(
          tabs.state.tabs.map((one) => one.id),
          () => tabs.closeAll()
        )
    },
    { separator: true, label: '' },
    { label: 'Copy path', icon: Copy, action: () => copyText(tab.path, 'Path') },
    { label: 'Reveal in Files', icon: FolderTree, action: () => revealInFiles(tab.path) }
  ]
  if (tab.preview) {
    items.splice(3, 0, { label: 'Keep open', action: () => tabs.pinTab(tab.id) })
  }
  menu.show(event, items, tab.path)
}

/** A wheel turned over the strip scrolls it sideways, the way the tabs run. */
function onWheel(event: WheelEvent) {
  const box = strip.value
  if (!box || Math.abs(event.deltaX) > Math.abs(event.deltaY)) return
  box.scrollLeft += event.deltaY
}

// The tab on screen is always in view, however many there are.
watch(
  () => tabs.state.active,
  async () => {
    await nextTick()
    const on = strip.value?.querySelector<HTMLElement>('.ftab.on')
    on?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' })
  }
)

useShortcuts({
  'nav.back': () => tabs.goBack(),
  'nav.forward': () => tabs.goForward(),
  'tab.next': () => tabs.stepTab(1),
  'tab.previous': () => tabs.stepTab(-1)
})

const backTitle = computed(() => `Back (${keyLabel(keysFor('nav.back'))})`)
const forwardTitle = computed(() => `Forward (${keyLabel(keysFor('nav.forward'))})`)
</script>

<template>
  <nav v-if="shown" class="file-tabs" aria-label="Open files">
    <span class="nav">
      <button
        class="arrow"
        data-testid="nav-back"
        :disabled="!tabs.state.back.length"
        :title="backTitle"
        @click="tabs.goBack()"
      >
        <ChevronLeft :size="15" />
      </button>
      <button
        class="arrow"
        data-testid="nav-forward"
        :disabled="!tabs.state.forward.length"
        :title="forwardTitle"
        @click="tabs.goForward()"
      >
        <ChevronRight :size="15" />
      </button>
    </span>

    <div ref="strip" class="strip" role="tablist" @wheel.passive="onWheel">
      <button
        class="ftab graph"
        role="tab"
        :class="{ on: tabs.state.active === null }"
        :aria-selected="tabs.state.active === null"
        title="The commit graph (Esc)"
        @click="!$event.shiftKey && tabs.showGraph()"
      >
        <GitBranch :size="13" />
        <span class="name">Graph</span>
      </button>

      <div
        v-for="tab in tabs.state.tabs"
        :key="tab.id"
        class="ftab"
        role="tab"
        :class="{
          on: tabs.state.active === tab.id,
          preview: tab.preview,
          unsaved: tab.dirty,
          dependency: isDependencyPath(tab.path)
        }"
        :aria-selected="tabs.state.active === tab.id"
        :data-path="tab.path"
        :title="title(tab)"
        tabindex="0"
        @mousedown.shift.prevent
        @click="onTabClick($event, tab)"
        @keydown.enter.self.prevent="tabs.selectTab(tab.id)"
        @keydown.space.self.prevent="tabs.selectTab(tab.id)"
        @dblclick="tabs.pinTab(tab.id)"
        @mousedown.middle.prevent
        @mouseup.middle="closeTabsSafely([tab.id], () => tabs.closeTab(tab.id))"
        @contextmenu="menuFor($event, tab)"
      >
        <span v-if="status(tab)" class="letter" :class="status(tab)!.kind">{{
          status(tab)!.letter
        }}</span>
        <FileIcon :path="tab.path" :size="13" />
        <span class="name">{{ base(tab.path) }}</span>
        <span v-if="tab.commit" class="hash">@ {{ tab.commit.slice(0, 6) }}</span>
        <span v-if="tab.dirty" class="dirty" title="Not saved yet" />
        <button
          class="close"
          :title="`Close (${keyLabel(keysFor('project.close'))})`"
          @click.stop="closeTabsSafely([tab.id], () => tabs.closeTab(tab.id))"
        >
          <X :size="12" />
        </button>
      </div>
    </div>
  </nav>
</template>

<style scoped>
/* Chrome, like the bar over a file: the canvas tone, with the tab on screen
   running down into the page below it. */
.file-tabs {
  display: flex;
  align-items: stretch;
  flex: none;
  height: 32px;
  background: var(--canvas);
  border-bottom: 1px solid var(--line);
}

.nav {
  display: flex;
  align-items: center;
  gap: 1px;
  flex: none;
  padding: 0 4px;
  border-right: 1px solid var(--line);
}

.arrow {
  display: grid;
  place-items: center;
  width: 22px;
  height: 22px;
  color: var(--text-dim);
  border-radius: var(--radius-sm);
}

.arrow:hover:not(:disabled) {
  background: var(--hover);
  color: var(--text);
}

.arrow:disabled {
  color: var(--text-faint);
  opacity: 0.5;
}

.strip {
  display: flex;
  align-items: stretch;
  min-width: 0;
  flex: 1;
  overflow-x: auto;
  overflow-y: hidden;
}

/* The pseudo-element alone, as the project tabs do: `scrollbar-width: none`
   makes GTK paint an overlay bar over whatever is on top of the strip. */
.strip::-webkit-scrollbar {
  display: none;
}

.ftab {
  position: relative;
  display: flex;
  align-items: center;
  gap: 6px;
  flex: none;
  max-width: 240px;
  padding: 0 6px 0 10px;
  font-size: 12px;
  color: var(--text-dim);
  border-right: 1px solid var(--line);
  white-space: nowrap;
  cursor: default;
  user-select: none;
}

.ftab.graph {
  padding-right: 12px;
}

.ftab:hover {
  color: var(--text);
}

/* The tab on screen is the page's own tone, joined to the page under it. */
.ftab.on {
  background: var(--bg);
  color: var(--text);
  margin-bottom: -1px;
  box-shadow: inset 0 2px 0 var(--accent);
}

/* A dependency's tab: amber name on an amber wash, as the bar over it. */
.ftab.dependency {
  background: color-mix(in srgb, var(--amber) 8%, var(--canvas));
}

.ftab.dependency.on {
  background: color-mix(in srgb, var(--amber) 12%, var(--bg));
  box-shadow: inset 0 2px 0 var(--amber);
}

.ftab.dependency .name {
  color: var(--amber);
}

.ftab.preview .name {
  font-style: italic;
}

.name {
  overflow: hidden;
  text-overflow: ellipsis;
}

.hash {
  font-family: var(--mono);
  font-size: 11px;
  color: var(--text-faint);
}

.letter {
  flex: none;
  font-family: var(--mono);
  font-size: 11px;
  font-weight: 700;
  color: var(--amber);
}

.letter.added {
  color: var(--green-soft);
}

.letter.deleted {
  color: var(--red);
}

.letter.renamed {
  color: var(--purple);
}

.letter.clash {
  color: var(--amber);
}

.close {
  display: grid;
  place-items: center;
  width: 18px;
  height: 18px;
  color: var(--text-faint);
  border-radius: var(--radius-sm);
  opacity: 0;
}

.ftab:hover .close,
.ftab.on .close,
.close:focus-visible {
  opacity: 1;
}

.ftab:focus-visible {
  outline: none;
  box-shadow: inset var(--focus);
}

/* Unsaved: the dot stands where the close button is, and gives way to it
   under the pointer or the keyboard, so the tab can still be closed. */
.ftab.unsaved .close {
  display: none;
}

.ftab.unsaved:hover .close,
.ftab.unsaved:focus-within .close {
  display: grid;
}

.ftab.unsaved:hover .dirty,
.ftab.unsaved:focus-within .dirty {
  display: none;
}

.close:hover {
  background: var(--hover);
  color: var(--text);
}

.dirty {
  width: 8px;
  height: 8px;
  margin: 0 5px;
  border-radius: 50%;
  background: var(--text-dim);
}
</style>
