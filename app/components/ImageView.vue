<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Maximize, Minus, Plus } from 'lucide-vue-next'
import {
  bytesLabel,
  dropImage,
  readImage,
  type ImageInfo,
  type ImageSide,
  type LoadedImage
} from '~/composables/useImage'
import { aimedAt } from '~/composables/useInvoke'
import { keysHere, matchesShortcut } from '~/composables/useShortcuts'

/**
 * One image, on a checkerboard so transparency shows, fitted to the view.
 *
 * Fit, 100%, and zoom with the buttons, ⌘+ / ⌘− / ⌘0, the wheel or a pinch;
 * drag to move round a zoomed image. Drawn through `<img>` from a Blob URL
 * only, never as inline markup, so an SVG's scripts cannot run. A work-tree
 * image is read again when the watcher says the work tree changed.
 *
 * `compact` is the search window's preview: fitted, no controls, no keys.
 */

const props = defineProps<{
  path: string
  side: ImageSide
  compact?: boolean
}>()

const emit = defineEmits<{
  /** Its size and type once drawn, for the header; null when there is none. */
  info: [ImageInfo | null]
  /** The pointer text when the file is a Git LFS pointer. */
  lfs: [string]
}>()

const repo = aimedAt()
const image = ref<LoadedImage | null>(null)
const failed = ref<string | null>(null)
const natural = ref<{ width: number; height: number } | null>(null)
/** null is fit to the view; otherwise the scale, 1 being 100%. */
const scale = ref<number | null>(null)
const stage = ref<HTMLElement | null>(null)
let seq = 0

async function load() {
  const mine = ++seq
  try {
    const found = await readImage(props.path, props.side, repo)
    if (mine !== seq) {
      dropImage(found)
      return
    }
    dropImage(image.value)
    image.value = found
    failed.value = null
    if (found.lfs) emit('lfs', found.lfs)
    if (!found.url) emit('info', null)
  } catch (error) {
    if (mine === seq) failed.value = String(error)
  }
}

function onLoad(event: Event) {
  const img = event.target as HTMLImageElement
  natural.value = { width: img.naturalWidth, height: img.naturalHeight }
  emit('info', {
    width: img.naturalWidth,
    height: img.naturalHeight,
    size: image.value?.size ?? 0,
    mime: image.value?.mime ?? null
  })
}

/** What the image is drawn at now, fitted or not, for the label. */
const shownScale = computed(() => {
  if (scale.value !== null) return scale.value
  const box = stage.value
  if (!box || !natural.value?.width) return 1
  return Math.min(1, box.clientWidth / natural.value.width, box.clientHeight / natural.value.height)
})

const imgStyle = computed(() =>
  scale.value === null || !natural.value
    ? { maxWidth: '100%', maxHeight: '100%' }
    : {
        width: `${natural.value.width * scale.value}px`,
        height: `${natural.value.height * scale.value}px`,
        maxWidth: 'none',
        maxHeight: 'none'
      }
)

const STEPS = [0.1, 0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4, 6, 8, 12, 16]

function zoom(by: number) {
  const now = shownScale.value
  const next =
    by > 0 ? (STEPS.find((step) => step > now + 0.001) ?? 16) : ([...STEPS].reverse().find((step) => step < now - 0.001) ?? 0.1)
  scale.value = next
}

function fit() {
  scale.value = null
}

function actual() {
  scale.value = 1
}

/** The wheel and a trackpad pinch (a wheel with ctrl) both zoom. */
function onWheel(event: WheelEvent) {
  if (props.compact) return
  event.preventDefault()
  const factor = Math.exp(-event.deltaY * (event.ctrlKey ? 0.01 : 0.002))
  scale.value = Math.min(16, Math.max(0.05, shownScale.value * factor))
}

// Dragging a zoomed image moves it.
let drag: { x: number; y: number; left: number; top: number } | null = null

function onDown(event: PointerEvent) {
  const box = stage.value
  if (!box || props.compact || scale.value === null) return
  drag = { x: event.clientX, y: event.clientY, left: box.scrollLeft, top: box.scrollTop }
  box.setPointerCapture?.(event.pointerId)
}

function onMove(event: PointerEvent) {
  const box = stage.value
  if (!drag || !box) return
  box.scrollLeft = drag.left - (event.clientX - drag.x)
  box.scrollTop = drag.top - (event.clientY - drag.y)
}

function onUp() {
  drag = null
}

/** ⌘+ ⌘− ⌘0 zoom the image while it is on screen, not the whole window. */
function onKey(event: KeyboardEvent) {
  if (props.compact || document.querySelector('.scrim, .overlay')) return
  // The window's own zoom keys, wherever they have been moved to.
  let handled = true
  if (matchesShortcut(event, 'zoom.in') || (event.key === '+' && (event.metaKey || event.ctrlKey))) zoom(1)
  else if (matchesShortcut(event, 'zoom.out')) zoom(-1)
  else if (matchesShortcut(event, 'zoom.reset')) fit()
  else handled = false
  if (handled) {
    event.preventDefault()
    event.stopImmediatePropagation()
  }
}

let unlisten: UnlistenFn | undefined
let reloadTimer: ReturnType<typeof setTimeout> | undefined
let alive = true

onMounted(async () => {
  void load()
  if (!props.compact) window.addEventListener('keydown', onKey, true)
  if (props.side.source !== 'work') return
  const off = await listen<{ work_tree: boolean }>('repo-changed', ({ payload }) => {
    if (!payload.work_tree) return
    clearTimeout(reloadTimer)
    reloadTimer = setTimeout(() => void load(), 150)
  }).catch(() => undefined)
  if (!alive) off?.()
  else unlisten = off
})

onBeforeUnmount(() => {
  alive = false
  seq++
  clearTimeout(reloadTimer)
  unlisten?.()
  window.removeEventListener('keydown', onKey, true)
  dropImage(image.value)
  image.value = null
})

watch(
  () => [props.path, props.side.source, props.side.commit],
  () => {
    scale.value = null
    natural.value = null
    void load()
  }
)

defineExpose({ zoom, fit, actual, reload: load })
</script>

<template>
  <div class="image-view" :class="{ compact: props.compact }">
    <div v-if="!props.compact && image?.url" class="tools">
      <button class="tool" :class="{ on: scale === null }" :title="keysHere('Fit to the view (⌘0)')" @click="fit">
        <Maximize :size="13" /> Fit
      </button>
      <button class="tool" :class="{ on: scale === 1 }" title="Actual size" @click="actual">100%</button>
      <button class="tool" :title="keysHere('Zoom out (⌘−)')" @click="zoom(-1)"><Minus :size="13" /></button>
      <span class="pct">{{ Math.round(shownScale * 100) }}%</span>
      <button class="tool" :title="keysHere('Zoom in (⌘+)')" @click="zoom(1)"><Plus :size="13" /></button>
    </div>
    <div
      ref="stage"
      class="stage"
      :class="{ zoomed: scale !== null, fitted: scale === null }"
      @wheel="onWheel"
      @pointerdown="onDown"
      @pointermove="onMove"
      @pointerup="onUp"
      @pointercancel="onUp"
    >
      <img
        v-if="image?.url"
        class="picture"
        :src="image.url"
        :alt="props.path"
        :style="imgStyle"
        draggable="false"
        @load="onLoad"
      />
      <p v-else-if="failed" class="note bad">{{ failed }}</p>
      <p v-else-if="image?.too_large" class="note">
        Too large to preview: {{ bytesLabel(image.size) }}, over the 30 MB the viewer reads.
      </p>
      <p v-else-if="image?.lfs" class="note">Stored with Git LFS: this is the pointer, not the image.</p>
      <p v-else-if="image?.missing" class="note">Not there.</p>
      <p v-else-if="image" class="note">Not an image this viewer can show.</p>
    </div>
  </div>
</template>

<style scoped>
.image-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.tools {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 4px 8px;
  border-bottom: 1px solid var(--line-soft);
  font-size: 11.5px;
}

.tool {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 22px;
  padding: 0 7px;
  color: var(--text-dim);
  border-radius: var(--radius-sm);
}

.tool:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.tool.on {
  color: var(--text);
  background: var(--bg-active);
}

.pct {
  min-width: 42px;
  text-align: center;
  font-variant-numeric: tabular-nums;
  color: var(--text-faint);
}

/* A checkerboard in the theme's own two tones, so a transparent pixel shows
   as one on a light theme and on a dark one. */
.stage {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: auto;
  background-color: var(--bg);
  background-image:
    linear-gradient(45deg, var(--bg-raised) 25%, transparent 25%),
    linear-gradient(-45deg, var(--bg-raised) 25%, transparent 25%),
    linear-gradient(45deg, transparent 75%, var(--bg-raised) 75%),
    linear-gradient(-45deg, transparent 75%, var(--bg-raised) 75%);
  background-size: 16px 16px;
  background-position:
    0 0,
    0 8px,
    8px -8px,
    -8px 0;
}

.stage.zoomed {
  cursor: grab;
}

.stage.zoomed:active {
  cursor: grabbing;
}

.picture {
  margin: auto;
  object-fit: contain;
  image-rendering: auto;
  user-select: none;
}

.stage.fitted .picture {
  padding: 12px;
  box-sizing: border-box;
}

.compact .stage.fitted .picture {
  padding: 8px;
}

.note {
  margin: auto;
  padding: 16px;
  font-size: 12px;
  color: var(--text-faint);
  text-align: center;
}

.note.bad {
  color: var(--danger);
}
</style>
