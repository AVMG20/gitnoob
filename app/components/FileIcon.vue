<script setup lang="ts">
/**
 * The type of a file or folder, as a small glyph in its language's colour.
 *
 * Only the type: a file's status (modified, new, conflicted) stays on its
 * name. The mapping is in `fileIcons.ts`; the colours in
 * `assets/css/file-icons.css`. The glyphs are drawn on lucide's grid (24
 * units, stroke 2, round ends) so they sit with the rest of the app's icons;
 * most paths are lucide's own (ISC licence, https://lucide.dev), the letter
 * badges, the Vue mark and the git diamond are drawn here.
 *
 * With "File icons" off in Settings it renders nothing.
 */
import { computed } from 'vue'
import { iconFor } from '~/composables/fileIcons'
import { useConfig } from '~/composables/useConfig'

const props = withDefaults(
  defineProps<{
    path: string
    folder?: boolean
    open?: boolean
    size?: number
  }>(),
  { folder: false, open: false, size: 14 }
)

const config = useConfig()
const shown = computed(() => config.settings.value?.file_icons !== false)
const icon = computed(() => iconFor(props.path, props.folder, props.open))
const small = computed(() => (icon.value.text?.length ?? 0) > 1)

const FILE = 'M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z M14 2v4a2 2 0 0 0 2 2h4'

/** Stroked paths per glyph; `letters` is drawn separately. */
const PATHS: Record<string, string> = {
  file: FILE,
  folder:
    'M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z',
  'folder-open':
    'm6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2',
  braces:
    'M8 3H7a2 2 0 0 0-2 2v5a2 2 0 0 1-2 2 2 2 0 0 1 2 2v5c0 1.1.9 2 2 2h1 M16 21h1a2 2 0 0 0 2-2v-5c0-1.1.9-2 2-2a2 2 0 0 1-2-2V5a2 2 0 0 0-2-2h-1',
  markup: 'm18 16 4-4-4-4 m-12 0-4 4 4 4 m8.5-12-5 16',
  hash: 'M4 9h16 M4 15h16 M10 3 8 21 M16 3l-2 18',
  terminal: 'M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z m2 8 2-2-2-2 M11 13h4',
  sliders: 'M20 7h-9 M14 17H5 M20 17a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z M10 7a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z',
  text: `${FILE} M10 9H8 M16 13H8 M16 17H8`,
  table: 'M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z M3 9h18 M3 15h18 M9 9v12 M15 9v12',
  image:
    'M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z M11 9a2 2 0 1 1-4 0 2 2 0 0 1 4 0Z m10 6-3.086-3.086a2 2 0 0 0-2.828 0L6 21',
  archive: 'M3 3h18a1 1 0 0 1 1 1v3a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1Z M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8 M10 12h4',
  lock: 'M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2Z M7 11V7a5 5 0 0 1 10 0v4',
  git: 'M12 2.5 21.5 12 12 21.5 2.5 12Z M12 8v8 M12 8l3.5 3.5',
  box: 'M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73z M12 22V12 m-8.7-5 7.703 4.734a2 2 0 0 0 1.994 0L20.7 7 m-13.2-2.73 9 5.15',
  book: 'M12 7v14 M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z',
  key: 'M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z M16.5 7.5h.01',
  scale:
    'm16 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1Z m-14 0 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1Z M7 21h10 M12 3v18 M3 7h2c2 0 5-1 7-2 2 1 5 2 7 2h2',
  vue: 'M2 4h4.5L12 13l5.5-9H22L12 21Z M6.5 4 12 13l5.5-9'
}
</script>

<template>
  <svg
    v-if="shown"
    class="file-icon"
    :class="`t-${icon.tone}`"
    :data-glyph="icon.glyph"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <template v-if="icon.glyph === 'letters'">
      <rect x="1.5" y="3" width="21" height="18" rx="3.5" fill="currentColor" stroke="none" />
      <text
        class="badge-text"
        x="12"
        y="12.6"
        text-anchor="middle"
        dominant-baseline="middle"
        :font-size="small ? 11 : 14"
        font-weight="700"
        :letter-spacing="small ? -0.6 : 0"
        stroke="none"
        >{{ icon.text }}</text
      >
    </template>
    <path v-else :d="PATHS[icon.glyph]" />
  </svg>
</template>

<style scoped>
.file-icon {
  display: inline-block;
  vertical-align: -2px;
  font-family: var(--font);
}
</style>
