<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import {
  bytesLabel,
  dropImage,
  formatOf,
  readImage,
  type ImageSide,
  type LoadedImage
} from '~/composables/useImage'
import { aimedAt } from '~/composables/useInvoke'

/**
 * A changed image, before and after: side by side with each one's size, or
 * one over the other with a slider between them (onion skin). A new image
 * has only an after, a deleted one only a before.
 */

const props = defineProps<{
  path: string
  before: ImageSide
  after: ImageSide
  /** Read again when this changes: the viewer's reload after a write. */
  version?: number
}>()

const repo = aimedAt()
const sides = ref<{ before: LoadedImage | null; after: LoadedImage | null }>({ before: null, after: null })
const sizes = ref<{ before: string; after: string }>({ before: '', after: '' })
const failed = ref<string | null>(null)
const mode = ref<'side' | 'onion'>('side')
const mix = ref(50)
let seq = 0

async function load() {
  const mine = ++seq
  try {
    const [before, after] = await Promise.all([
      readImage(props.path, props.before, repo),
      readImage(props.path, props.after, repo)
    ])
    if (mine !== seq) {
      dropImage(before)
      dropImage(after)
      return
    }
    drop()
    sides.value = { before, after }
    sizes.value = { before: '', after: '' }
    failed.value = null
  } catch (error) {
    if (mine === seq) failed.value = String(error)
  }
}

function drop() {
  dropImage(sides.value.before)
  dropImage(sides.value.after)
}

function measured(which: 'before' | 'after', event: Event) {
  const img = event.target as HTMLImageElement
  sizes.value = { ...sizes.value, [which]: `${img.naturalWidth} × ${img.naturalHeight}` }
}

/** What a side says under its title. */
function caption(which: 'before' | 'after') {
  const side = sides.value[which]
  if (!side) return ''
  if (side.missing) return which === 'before' ? 'Not there before' : 'Deleted'
  const parts = [sizes.value[which], bytesLabel(side.size), formatOf(side.mime)].filter(Boolean)
  return parts.join(' · ')
}

function note(side: LoadedImage | null) {
  if (!side) return 'Loading…'
  if (side.missing) return ''
  if (side.too_large) return 'Too large to preview.'
  if (side.lfs) return 'Stored with Git LFS: this is the pointer.'
  return 'Not an image this viewer can show.'
}

const both = computed(() => !!sides.value.before?.url && !!sides.value.after?.url)

watch(
  () => [props.path, props.before.source, props.before.commit, props.after.source, props.after.commit, props.version],
  () => void load(),
  { immediate: true }
)

onBeforeUnmount(() => {
  seq++
  drop()
})
</script>

<template>
  <div class="image-diff">
    <div class="tools">
      <span class="seg-group">
        <button class="seg" :class="{ on: mode === 'side' }" @click="mode = 'side'">Side by side</button>
        <button class="seg" :class="{ on: mode === 'onion' }" :disabled="!both" @click="mode = 'onion'">
          Onion skin
        </button>
      </span>
      <input
        v-if="mode === 'onion' && both"
        v-model.number="mix"
        class="mix"
        type="range"
        min="0"
        max="100"
        title="Before ← → After"
      />
      <span v-if="failed" class="bad">{{ failed }}</span>
    </div>

    <div v-if="mode === 'onion' && both" class="onion checker">
      <div class="stack">
        <img class="picture" :src="sides.before!.url!" alt="Before" draggable="false" />
        <img
          class="picture over"
          :src="sides.after!.url!"
          alt="After"
          draggable="false"
          :style="{ opacity: mix / 100 }"
        />
      </div>
    </div>

    <div v-else class="sides">
      <figure
        v-for="which in (['before', 'after'] as const)"
        :key="which"
        class="side"
        :data-side="which"
        :class="{ empty: sides[which]?.missing }"
      >
        <figcaption>
          <strong>{{ which === 'before' ? 'Before' : 'After' }}</strong>
          <span class="faint">{{ caption(which) }}</span>
        </figcaption>
        <div class="checker">
          <img
            v-if="sides[which]?.url"
            class="picture"
            :src="sides[which]!.url!"
            :alt="which"
            draggable="false"
            @load="measured(which, $event)"
          />
          <p v-else class="note">{{ note(sides[which]) }}</p>
        </div>
      </figure>
    </div>
  </div>
</template>

<style scoped>
.image-diff {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.tools {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 8px;
  border-bottom: 1px solid var(--line-soft);
  font-size: 11.5px;
}

.seg-group {
  display: flex;
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.seg {
  padding: 2px 9px;
  font-size: 11.5px;
  color: var(--text-dim);
}

.seg.on {
  color: var(--text);
  background: var(--bg-active);
}

.mix {
  width: 160px;
}

.bad {
  color: var(--danger);
}

.sides {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: 1fr 1fr;
}

.side {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  margin: 0;
}

.side + .side {
  border-left: 1px solid var(--line);
}

figcaption {
  display: flex;
  gap: 8px;
  align-items: baseline;
  padding: 4px 10px;
  font-size: 11.5px;
  border-bottom: 1px solid var(--line-soft);
}

.side[data-side='before'] strong {
  color: var(--red);
}

.side[data-side='after'] strong {
  color: var(--green-soft);
}

/* The same theme-toned checkerboard as the single image. */
.checker {
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

.side.empty .checker {
  background-image: none;
}

.picture {
  margin: auto;
  max-width: 100%;
  max-height: 100%;
  padding: 12px;
  box-sizing: border-box;
  object-fit: contain;
}

.onion {
  flex: 1;
}

.stack {
  position: relative;
  margin: auto;
  display: grid;
  max-width: 100%;
  max-height: 100%;
}

.stack .picture {
  grid-area: 1 / 1;
}

.note {
  margin: auto;
  padding: 16px;
  font-size: 12px;
  color: var(--text-faint);
}
</style>
