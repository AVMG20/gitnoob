<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import FileEditor from '~/components/FileEditor.vue'
import ImageView from '~/components/ImageView.vue'
import { isImagePath } from '~/composables/useImage'
import type { DiffHunk, FileDiff } from '~/composables/useGit'
import { aimedAt, invoke } from '~/composables/useInvoke'
import { saveStateOf } from '~/composables/useFileEdit'
import { tabId } from '~/composables/useTabs'
import type { LineHits } from '~/composables/useCodeMirror'

/**
 * The search window's preview: the file in the editor, at the search's line.
 *
 * The editor the tabs use (`FileEditor` in its `preview` form), so it edits
 * and saves the same way: the language, the colours, the indentation, the
 * change bars, saving as you type against the hash it read, the bar when the
 * file changed on disk. Every match in the file is marked, the selected one
 * brighter; lines wrap, so nothing needs scrolling sideways. A file open in a
 * tab is the tab's buffer, borrowed, never a second copy.
 *
 * Read-only where the tab would be: binary, LFS, too large, mixed endings.
 * An image is shown as one.
 */

const props = defineProps<{
  path: string | null
  line: number | null
  /** Every match in this file, to mark. */
  hits?: LineHits[]
  /** What the window is looking for, for ⌘F inside the preview. */
  query?: { search: string; caseSensitive: boolean; wholeWord: boolean; regexp: boolean } | null
}>()

const editor = ref<InstanceType<typeof FileEditor> | null>(null)
const hunks = ref<DiffHunk[]>([])
const image = computed(() => !!props.path && isImagePath(props.path))
const repo = aimedAt()
let seq = 0

/** The file against the last commit, for the change bars. */
async function loadHunks() {
  const path = props.path
  const mine = ++seq
  if (!path || image.value) {
    hunks.value = []
    return
  }
  const diff = await invoke<FileDiff>('head_file_diff', { path }).catch(() => null)
  if (mine === seq) hunks.value = diff?.hunks ?? []
}

watch(() => props.path, () => void loadHunks(), { immediate: true })

// Saved: the bars are measured against the file as it is now.
watch(
  () => (props.path ? saveStateOf(repo, tabId(props.path)) : undefined),
  (now, before) => {
    if (now === 'saved' && before && before !== 'saved') void loadHunks()
  }
)

/** The caret on the search's line at its first match, and the focus in the editor. */
function focusAt(line: number | null, column = 0) {
  editor.value?.focusAt(line, column)
}

defineExpose({ focusAt })
</script>

<template>
  <div class="preview">
    <p v-if="!props.path" class="none faint">Nothing to preview.</p>
    <ImageView v-else-if="image" :key="props.path!" :path="props.path!" :side="{ source: 'work' }" compact />
    <FileEditor
      v-else
      :key="props.path!"
      ref="editor"
      :tab-id="tabId(props.path!)"
      :path="props.path!"
      :hunks="hunks"
      :runs="null"
      :blame="false"
      :find="null"
      :fallback="null"
      preview
      :line="props.line"
      :hits="props.hits ?? []"
      :query="props.query ?? null"
    />
  </div>
</template>

<style scoped>
.preview {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}

.preview > .editor-pane {
  flex: 1;
  min-height: 0;
}

.none {
  margin: 0;
  padding: 16px;
  font-size: 12px;
}
</style>
