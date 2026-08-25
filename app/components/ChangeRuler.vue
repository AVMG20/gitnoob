<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

/**
 * Where the changes are in the whole file, drawn beside the scrollbar.
 *
 * The gutter marks tell you what a line is once you have scrolled to it; this
 * says where to scroll. It is measured from the rendered lines rather than
 * built from the diff, so one strip serves both views and neither has to hand
 * over a model of its own layout.
 */
const props = defineProps<{
  container: HTMLElement | null
  /** Changes whenever the content does, so the marks are measured again. */
  version: string
}>()

interface Mark {
  kind: 'added' | 'changed' | 'removed' | 'gone'
  /** Fractions of the scrollable height, 0 to 1. */
  top: number
  height: number
}

const marks = ref<Mark[]>([])
const view = ref({ top: 0, height: 1 })
const strip = ref<HTMLElement | null>(null)

const CHANGED = '.line.added, .line.changed, .line.add, .line.del, .gone'

function kindOf(element: Element): Mark['kind'] {
  const list = element.classList
  if (list.contains('gone')) return 'gone'
  if (list.contains('added') || list.contains('add')) return 'added'
  if (list.contains('changed')) return 'changed'
  return 'removed'
}

/** Reads the marked lines out of the rendered view and folds runs into bars. */
function measure() {
  const box = props.container
  if (!box) {
    marks.value = []
    return
  }
  const height = box.scrollHeight
  if (!height) return
  const origin = box.getBoundingClientRect().top - box.scrollTop
  const found: Mark[] = []

  for (const element of box.querySelectorAll(CHANGED)) {
    const rect = element.getBoundingClientRect()
    if (!rect.height) continue
    const kind = kindOf(element)
    const top = rect.top - origin
    const last = found[found.length - 1]
    // A run of changed lines is one thing to look at, not twenty.
    if (last && last.kind === kind && top <= last.top + last.height + 1.5) {
      last.height = top + rect.height - last.top
    } else {
      found.push({ kind, top, height: rect.height })
    }
  }

  marks.value = found.map((mark) => ({
    kind: mark.kind,
    top: mark.top / height,
    height: mark.height / height
  }))
  // Switching between the two views replaces the whole child, and it is the
  // child that grows as the lines are painted.
  if (observer && box.firstElementChild) observer.observe(box.firstElementChild)
  track()
}

/** Which slice of the file is on screen, so the strip says where you are. */
function track() {
  const box = props.container
  if (!box || !box.scrollHeight) return
  view.value = {
    top: box.scrollTop / box.scrollHeight,
    height: Math.min(1, box.clientHeight / box.scrollHeight)
  }
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

const anything = computed(() => marks.value.length > 0)

let observer: ResizeObserver | null = null
let frame = 0

function remeasure() {
  cancelAnimationFrame(frame)
  frame = requestAnimationFrame(measure)
}

function watchBox(box: HTMLElement | null, old?: HTMLElement | null) {
  old?.removeEventListener('scroll', track)
  observer?.disconnect()
  observer = null
  if (!box) return
  box.addEventListener('scroll', track, { passive: true })
  // The content is painted a frame or two after the data arrives, and a long
  // file keeps growing as highlighting lands.
  observer = new ResizeObserver(remeasure)
  observer.observe(box)
  if (box.firstElementChild) observer.observe(box.firstElementChild)
  remeasure()
}

watch(() => props.container, (box, old) => watchBox(box, old))
watch(() => props.version, remeasure)

onMounted(() => watchBox(props.container))
onBeforeUnmount(() => {
  cancelAnimationFrame(frame)
  props.container?.removeEventListener('scroll', track)
  observer?.disconnect()
})
</script>

<template>
  <div
    ref="strip"
    class="ruler"
    :class="{ bare: !anything }"
    title="Where the changes are — click to go there"
    @click="jump"
  >
    <span class="view" :style="{ top: `${view.top * 100}%`, height: `${view.height * 100}%` }" />
    <span v-for="(mark, index) in marks" :key="index" class="mark" :class="mark.kind" :style="style(mark)" />
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

/* Where you are now, drawn behind the marks so it never hides one. */
.view {
  position: absolute;
  left: 0;
  right: 0;
  min-height: 6px;
  background: var(--bg-hover);
}
</style>
