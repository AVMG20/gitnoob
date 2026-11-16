<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Copy, FileDiff, GitCommitHorizontal } from 'lucide-vue-next'
import Avatar from '~/components/Avatar.vue'
import {
  blameAgo,
  blamePopup,
  closeBlamePopup,
  copyBlamedHash,
  showBlamedChange,
  showBlamedInGraph
} from '~/composables/blame'
import { diffMode } from '~/composables/useDiffMode'
import { fullTime, useGit } from '~/composables/useGit'

/**
 * The commit behind a line of the blame column, hung from the entry that was
 * clicked: who, when, the hash and the whole message, and the ways on from
 * there. The file stays on screen. Esc, a click outside, a scroll or another
 * file closes it.
 *
 * No backdrop, so the editor under it still scrolls (which closes it). It is
 * an `.overlay`, so the app's own keys (Esc back to the graph, Tab between the
 * views) stand aside while it is open.
 */

const WIDTH = 400
const GAP = 4

const panel = ref<HTMLElement | null>(null)
const height = ref(220)
const git = useGit()

const run = computed(() => blamePopup.run)

const place = computed(() => {
  const { left, top, bottom } = blamePopup.anchor
  const x = Math.max(8, Math.min(left, window.innerWidth - WIDTH - 8))
  const below = bottom + GAP
  const y =
    below + height.value > window.innerHeight - 8 ? Math.max(8, top - height.value - GAP) : below
  return { left: `${x}px`, top: `${y}px`, width: `${WIDTH}px` }
})

const inside = (target: EventTarget | null) =>
  target instanceof Node && !!panel.value?.contains(target)

function onDown(event: MouseEvent) {
  // Focus goes wherever the click went, so it is not handed back.
  if (!inside(event.target)) closeBlamePopup()
}

function onScroll(event: Event) {
  if (!inside(event.target)) closeBlamePopup(true)
}

function onKey(event: KeyboardEvent) {
  if (event.key !== 'Escape') return
  event.preventDefault()
  event.stopPropagation()
  closeBlamePopup(true)
}

const onResize = () => closeBlamePopup(true)

// Another file, another tab, the column turned off or another project: the
// line it was about is not on screen any more.
watch(
  () => [git.store.viewer?.path, git.store.viewer?.commit, git.store.repo?.path, diffMode.blame, diffMode.mode],
  () => closeBlamePopup()
)

onMounted(async () => {
  document.addEventListener('mousedown', onDown, true)
  document.addEventListener('scroll', onScroll, true)
  document.addEventListener('keydown', onKey, true)
  window.addEventListener('resize', onResize)
  await nextTick()
  if (panel.value) {
    height.value = panel.value.offsetHeight || height.value
    panel.value.focus({ preventScroll: true })
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('mousedown', onDown, true)
  document.removeEventListener('scroll', onScroll, true)
  document.removeEventListener('keydown', onKey, true)
  window.removeEventListener('resize', onResize)
})
</script>

<template>
  <div
    v-if="run"
    ref="panel"
    class="overlay blame-pop"
    role="dialog"
    :aria-label="`Commit ${run.short} by ${run.author}`"
    tabindex="-1"
    :style="place"
    data-testid="blame-popover"
  >
    <header class="who">
      <Avatar :name="run.author" :email="run.email" :size="28" />
      <div class="names">
        <div class="author">{{ run.author }}</div>
        <div class="email faint">{{ run.email }}</div>
        <div class="when faint" :title="fullTime(run.time)">
          {{ fullTime(run.time) }} · {{ blameAgo(run.time) }}
        </div>
      </div>
    </header>

    <div class="body">
      <button
        type="button"
        class="hash"
        title="Copy the full hash"
        data-testid="blame-hash"
        @click="copyBlamedHash()"
      >
        {{ run.short }} <Copy :size="11" />
      </button>
      <pre class="message" data-testid="blame-message">{{ blamePopup.message }}</pre>
    </div>

    <footer class="actions">
      <button type="button" class="act" data-testid="blame-change" @click="showBlamedChange">
        <FileDiff :size="13" /> Show this file's change
      </button>
      <button type="button" class="act" data-testid="blame-graph" @click="showBlamedInGraph">
        <GitCommitHorizontal :size="13" /> Show in graph
      </button>
      <button type="button" class="act" data-testid="blame-copy" @click="copyBlamedHash()">
        <Copy :size="13" /> Copy hash
      </button>
    </footer>
  </div>
</template>

<style scoped>
.blame-pop {
  position: fixed;
  z-index: 60;
  display: flex;
  flex-direction: column;
  max-height: min(420px, calc(100vh - 16px));
  background: var(--canvas);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: var(--shadow-pop);
  outline: none;
  overflow: hidden;
  font-size: 12px;
}

.who {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px 8px;
  border-bottom: 1px solid var(--line);
}

.names {
  min-width: 0;
  flex: 1;
}

.author {
  font-weight: 600;
  color: var(--text);
}

.email,
.when {
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.body {
  padding: 8px 12px;
  overflow-y: auto;
  min-height: 0;
}

.hash {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 7px;
  margin-bottom: 6px;
  border: none;
  border-radius: 8px;
  background: var(--hover);
  color: var(--text-dim);
  font-family: var(--mono);
  font-size: 11px;
  cursor: pointer;
}

.hash:hover {
  color: var(--text);
}

.message {
  margin: 0;
  font-family: var(--font);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--text);
}

.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  padding: 6px 12px 8px;
  border-top: 1px solid var(--line);
}

.act {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: none;
  background: none;
  padding: 0;
  color: var(--primary);
  font: inherit;
  font-size: 11px;
  cursor: pointer;
}

.act:hover {
  text-decoration: underline;
}
</style>
