<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { Compartment, EditorState } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import type { FileDiff } from '~/composables/useGit'
import { invoke } from '~/composables/useInvoke'
import { version } from '~/composables/useHighlight'
import { readFile } from '~/composables/useFileEdit'
import {
  barsFor,
  basics,
  changeBars,
  editorLanguage,
  readOnly,
  searchHits,
  setBars,
  setHits,
  setLanding,
  syntaxColours,
  type LineHits
} from '~/composables/useCodeMirror'

/**
 * A file, read-only, at one line: the search window's preview.
 *
 * The same view the editor is built from, with the change bars, so whether a
 * match sits on a line that changed can be seen before it is opened. Every
 * match in the file is marked, the selected one brighter; lines wrap, so
 * nothing needs scrolling sideways. A click on a line opens the file there.
 */

const props = defineProps<{
  path: string | null
  line: number | null
  /** Every match in this file, to mark. */
  hits?: LineHits[]
}>()

const emit = defineEmits<{ open: [number] }>()

const host = ref<HTMLElement | null>(null)
const empty = ref(false)
let view: EditorView | null = null
const language = new Compartment()
const colours = new Compartment()
let loaded: string | null = null
/** Whether the file last read had nothing to show. */
let blank = false
let seq = 0

function center(line: number | null) {
  if (!view || !line) return
  const number = Math.min(Math.max(1, line), view.state.doc.lines)
  const at = view.state.doc.line(number).from
  view.dispatch({
    effects: [EditorView.scrollIntoView(at, { y: 'center' }), setLanding.of(number)]
  })
}

function marks() {
  if (!view) return
  view.dispatch({ effects: setHits.of({ hits: props.hits ?? [], now: props.line }) })
}

async function show() {
  const path = props.path
  const mine = ++seq
  if (!path) {
    empty.value = true
    return
  }
  // The same file coming back after a search with nothing in it: what was
  // read is still there, and so is whether it had anything to show.
  empty.value = blank
  if (path !== loaded) {
    const read = await readFile(path)
    if (mine !== seq || !view) return
    loaded = path
    const text = read && !read.missing && read.readonly !== 'binary' ? read.text : ''
    blank = !text
    empty.value = blank
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: text },
      effects: language.reconfigure([])
    })
    void editorLanguage(path).then((support) => {
      if (support && view && loaded === path) view.dispatch({ effects: language.reconfigure(support) })
    })
    invoke<FileDiff>('head_file_diff', { path })
      .then((diff) => {
        if (view && loaded === path) {
          view.dispatch({ effects: setBars.of(barsFor(view.state.doc.toString(), diff?.hunks ?? [])) })
        }
      })
      .catch(() => undefined)
  }
  marks()
  center(props.line)
}

onMounted(() => {
  if (!host.value) return
  view = new EditorView({
    parent: host.value,
    state: EditorState.create({
      doc: '',
      extensions: [
        basics(),
        changeBars(() => undefined),
        language.of([]),
        colours.of(syntaxColours()),
        readOnly(true),
        searchHits,
        EditorView.lineWrapping,
        EditorView.domEventHandlers({
          click(event, editor) {
            // A click that selected text is a selection, not a jump.
            if (!editor.state.selection.main.empty) return false
            const at = editor.posAtCoords({ x: event.clientX, y: event.clientY })
            if (at === null) return false
            emit('open', editor.state.doc.lineAt(at).number)
            return true
          }
        })
      ]
    })
  })
  void show()
})

onBeforeUnmount(() => {
  view?.destroy()
  view = null
})

watch(() => [props.path, props.line], () => void show())
// More matches arriving while a search runs are marked where the view is: it
// does not go back to the selected line under whoever is reading.
watch(
  () => props.hits,
  () => marks()
)
watch(version, () => view?.dispatch({ effects: colours.reconfigure(syntaxColours()) }))
</script>

<template>
  <div class="preview">
    <p v-if="!props.path" class="none faint">Nothing to preview.</p>
    <p v-else-if="empty" class="none faint">Nothing to show for this file.</p>
    <div ref="host" class="host" :class="{ hidden: !props.path }" />
  </div>
</template>

<style scoped>
.preview {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}

.host {
  flex: 1;
  min-height: 0;
}

.host.hidden {
  display: none;
}

.host :deep(.cm-editor) {
  height: 100%;
}

.host :deep(.cm-content) {
  cursor: pointer;
}

.none {
  margin: 0;
  padding: 16px;
  font-size: 12px;
}
</style>
