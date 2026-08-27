<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { Mark } from '~/composables/useCode'

/**
 * Where the changes are in the whole file, drawn beside the scrollbar.
 *
 * The gutter marks tell you what a line is once you have scrolled to it; this
 * says where to scroll.
 *
 * Handed the marks rather than reading them off the page. It used to find them
 * with a `querySelectorAll` over the rendered rows, which was neat while every
 * line of the file was drawn and became wrong the moment only the ones on
 * screen were: a strip that can only mark what you are already looking at has
 * nothing to say. They come from the same model the rows are drawn from, so the
 * two cannot disagree.
 */
const props = defineProps<{
  container: HTMLElement | null
  /** Every change in the file, in fractions of its height. */
  marks: Mark[]
  /**
   * The one mark being worked on, drawn brighter and wider than the rest.
   *
   * The resolver walks from conflict to conflict, and the strip is the only
   * thing on screen that can say where that is in the file as a whole.
   */
  active?: number | null
  /** What the strip is pointing at, for the tooltip. */
  hint?: string
}>()

const view = ref({ top: 0, height: 1 })
const strip = ref<HTMLElement | null>(null)

/**
 * The box's scroll extent, read when it changes rather than on every scroll.
 *
 * `scrollHeight` and `clientHeight` both force the layout the scroll just
 * invalidated to be computed again, and the rows are laid out at `max-content`
 * width, so computing it is not cheap. Neither can change without the box
 * resizing or its content being replaced, and both come back through here.
 */
let metrics = { height: 0, view: 0 }
let tracking = false
let observer: ResizeObserver | null = null

function remeasure() {
  const box = props.container
  if (!box) {
    metrics = { height: 0, view: 0 }
    return
  }
  metrics = { height: box.scrollHeight, view: box.clientHeight }
  track()
}

/** Which slice of the file is on screen, so the strip says where you are. */
function track() {
  const box = props.container
  if (!box || !metrics.height) return
  view.value = {
    top: box.scrollTop / metrics.height,
    height: Math.min(1, metrics.view / metrics.height)
  }
}

/** Scroll events outrun frames; the strip only has to be right once a frame. */
function onScroll() {
  if (tracking) return
  tracking = true
  requestAnimationFrame(() => {
    tracking = false
    track()
  })
}

/** A click anywhere on the strip goes to that part of the file. */
function jump(event: MouseEvent) {
  const box = props.container
  const rect = strip.value?.getBoundingClientRect()
  if (!box || !rect) return
  const at = (event.clientY - rect.top) / rect.height
  box.scrollTo({ top: at * box.scrollHeight - box.clientHeight / 2, behavior: 'smooth' })
}

const style = (mark: Mark) => ({
  top: `${mark.top * 100}%`,
  // Anything less than a couple of pixels is invisible, and a one-line change
  // is exactly the one worth finding.
  height: `max(2.5px, ${mark.height * 100}%)`
})

const anything = computed(() => props.marks.length > 0)

function watchBox(box: HTMLElement | null, old?: HTMLElement | null) {
  old?.removeEventListener('scroll', onScroll)
  observer?.disconnect()
  observer = null
  if (!box) return
  box.addEventListener('scroll', onScroll, { passive: true })
  observer = new ResizeObserver(remeasure)
  observer.observe(box)
  remeasure()
}

watch(() => props.container, (box, old) => watchBox(box, old))
// The rows are placed from the model, so the height follows the marks changing.
watch(() => props.marks, () => requestAnimationFrame(remeasure))

onMounted(() => watchBox(props.container))
onBeforeUnmount(() => {
  props.container?.removeEventListener('scroll', onScroll)
  observer?.disconnect()
})
</script>

<template>
  <div
    ref="strip"
    class="ruler"
    :class="{ bare: !anything }"
    :title="props.hint ?? 'Where the changes are — click to go there'"
    @click="jump"
  >
    <span class="view" :style="{ top: `${view.top * 100}%`, height: `${view.height * 100}%` }" />
    <span
      v-for="(mark, index) in props.marks"
      :key="index"
      class="mark"
      :class="[mark.kind, { now: index === props.active }]"
      :style="style(mark)"
    />
  </div>
</template>

<style scoped>
.ruler {
  position: relative;
  flex: none;
  width: 11px;
  cursor: pointer;
  background: var(--bg-panel);
  border-left: 1px solid var(--line-soft);
}

/* Nothing to point at: the strip stays as a plain edge rather than a control
   that does nothing. */
.ruler.bare {
  cursor: default;
}

.mark {
  position: absolute;
  left: 2px;
  right: 2px;
  border-radius: 1px;
}

.mark.added {
  background: var(--green);
}

.mark.changed {
  background: var(--accent);
}

.mark.removed {
  background: var(--red);
}

.mark.gone {
  background: var(--text-faint);
}

/* The resolver's three: still to look at, decided, and set to be dropped. */
.mark.open {
  background: var(--amber);
}

.mark.settled {
  background: var(--green);
}

.mark.dropped {
  background: var(--red);
}

/* The one being worked on: the full width of the strip, and outlined so it
   reads as the current place even where it sits among a run of its neighbours. */
.mark.now {
  left: 0;
  right: 0;
  box-shadow: 0 0 0 1px var(--text);
  border-radius: 2px;
}

/* Where you are now, drawn behind the marks so it never hides one. */
.view {
  position: absolute;
  left: 0;
  right: 0;
  min-height: 6px;
  background: var(--bg-hover);
}
</style>
