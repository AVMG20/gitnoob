<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { ChevronDown, ChevronUp, Copy, Check, GitCompare, Undo2, X } from 'lucide-vue-next'
import { Compartment, EditorState, RangeSetBuilder, StateEffect, StateField } from '@codemirror/state'
import { Decoration, EditorView, type DecorationSet } from '@codemirror/view'
import { editorLanguage, readOnly, syntaxColours, chrome } from '~/composables/useCodeMirror'
import { version } from '~/composables/useHighlight'
import { wordChanges, type Change } from '~/composables/changeCard'
import { keyLabel, keysFor } from '~/composables/useShortcuts'

/**
 * What was there before one change, in a card under it: the bar in the
 * gutter clicked, the editor left as it is.
 *
 * The old lines are drawn by CodeMirror with the editor's own language and
 * colours, on the diff's red, with the words that changed picked out. From
 * here a change can be put back (one edit, so ⌘Z takes it back again),
 * staged, copied, or read in the Diff view.
 */

const props = defineProps<{
  path: string
  change: Change
  count: number
  /** False for a file the editor will not write: no Revert, no Stage. */
  editable: boolean
}>()

const emit = defineEmits<{
  move: [number]
  revert: []
  stage: []
  copy: []
  diff: []
  close: []
}>()

const host = ref<HTMLElement | null>(null)
let view: EditorView | null = null
const language = new Compartment()
const colours = new Compartment()

const setWas = StateEffect.define<Change>()

const was = Decoration.line({ class: 'cm-was' })
const word = Decoration.mark({ class: 'cm-was-word' })

/** The red under every old line, and the darker red under what changed. */
const wasField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    for (const effect of tr.effects) {
      if (!effect.is(setWas)) continue
      const builder = new RangeSetBuilder<Decoration>()
      const words = wordChanges(effect.value)
      for (let n = 1; n <= tr.state.doc.lines; n++) {
        const line = tr.state.doc.line(n)
        builder.add(line.from, line.from, was)
        for (const [from, to] of words[n - 1] ?? []) {
          // Only where a line was changed, not removed outright.
          if (effect.value.kind === 'modified' && to > from && n <= effect.value.added.length) {
            builder.add(line.from + Math.min(from, line.length), line.from + Math.min(to, line.length), word)
          }
        }
      }
      return builder.finish()
    }
    return value
  },
  provide: (field) => EditorView.decorations.from(field)
})

function show() {
  if (!view) return
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: props.change.removed.join('\n') },
    effects: setWas.of(props.change)
  })
}

onMounted(() => {
  if (!host.value) return
  view = new EditorView({
    parent: host.value,
    state: EditorState.create({
      doc: '',
      extensions: [
        chrome,
        language.of([]),
        colours.of(syntaxColours()),
        readOnly(true),
        EditorView.editable.of(false),
        EditorView.lineWrapping,
        wasField
      ]
    })
  })
  show()
  void editorLanguage(props.path).then((support) => {
    if (support && view) view.dispatch({ effects: language.reconfigure(support) })
  })
})

onBeforeUnmount(() => {
  view?.destroy()
  view = null
})

watch(() => props.change, show)
watch(version, () => view?.dispatch({ effects: colours.reconfigure(syntaxColours()) }))

const nextKey = keyLabel(keysFor('file.nextChange'))
const previousKey = keyLabel(keysFor('file.previousChange'))
</script>

<template>
  <div class="change-card" role="dialog" aria-label="What was here before" @mousedown.stop>
    <div class="head">
      <span class="title">Change {{ props.change.index + 1 }} of {{ props.count }}</span>
      <button class="icon" aria-label="Previous change" :title="`Previous change (${previousKey})`" @click="emit('move', -1)">
        <ChevronUp :size="14" />
      </button>
      <button class="icon" aria-label="Next change" :title="`Next change (${nextKey})`" @click="emit('move', 1)">
        <ChevronDown :size="14" />
      </button>
      <span class="grow" />
      <template v-if="props.editable">
        <button
          class="act"
          data-act="revert"
          title="Put the old lines back in the editor (⌘Z takes it back)"
          @click="emit('revert')"
        >
          <Undo2 :size="13" /> Revert
        </button>
        <button class="act" data-act="stage" title="Stage just this change" @click="emit('stage')">
          <Check :size="13" /> Stage this change
        </button>
      </template>
      <button
        v-if="props.change.removed.length"
        class="act"
        data-act="copy"
        title="Copy the old lines"
        @click="emit('copy')"
      >
        <Copy :size="13" /> Copy old text
      </button>
      <button class="act" data-act="diff" title="Open this change in the Diff view" @click="emit('diff')">
        <GitCompare :size="13" /> Show in Diff
      </button>
      <button class="icon" aria-label="Close" title="Close (Esc)" @click="emit('close')">
        <X :size="14" />
      </button>
    </div>
    <p v-if="props.change.kind === 'added'" class="note">Added lines; nothing was here before.</p>
    <div v-show="props.change.kind !== 'added'" ref="host" class="was" />
  </div>
</template>

<style scoped>
.change-card {
  display: flex;
  flex-direction: column;
  max-height: 260px;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  box-shadow: var(--shadow-pop);
  overflow: hidden;
}

.head {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 4px 3px 10px;
  background: var(--canvas);
  border-bottom: 1px solid var(--line-soft);
  font-size: 12px;
}

.title {
  font-weight: 600;
  color: var(--text-dim);
  margin-right: 4px;
}

.grow {
  flex: 1;
}

.icon {
  display: grid;
  place-items: center;
  width: 22px;
  height: 22px;
  color: var(--text-faint);
  border-radius: var(--radius-sm);
}

.icon:hover,
.act:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.act {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 22px;
  padding: 0 7px;
  font-size: 11.5px;
  color: var(--text-dim);
  border-radius: var(--radius-sm);
  white-space: nowrap;
}

.note {
  margin: 0;
  padding: 8px 10px;
  font-size: 12px;
  color: var(--text-faint);
}

.was {
  min-height: 0;
  overflow: auto;
}

.was :deep(.cm-editor) {
  background: transparent;
}

.was :deep(.cm-was) {
  background: var(--diff-del-bg);
}

.was :deep(.cm-was-word) {
  background: var(--diff-del-line);
  border-radius: 2px;
}
</style>
