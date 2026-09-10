<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import Spinner from './Spinner.vue'
import { useAttachments } from '~/composables/useAttachments'

const props = withDefaults(
  defineProps<{
    placeholder?: string
    busy?: boolean
    sendLabel?: string
    compact?: boolean
    autofocus?: boolean
    /**
     * Text held by the caller rather than by the box.
     *
     * Given, the box stops owning what is typed: it does not clear on send —
     * the caller clears it once the send has actually landed — so a remark is
     * never lost to a failure, and the caller can keep it however long it
     * likes.
     */
    modelValue?: string
    /**
     * Whether a Cancel is worth drawing.
     *
     * A reply or a line remark has a thread to be folded back into, so it can
     * be cancelled; the conversation's own composer has no state of its own to
     * leave, and a button there was a way to do nothing.
     */
    cancellable?: boolean
    /**
     * A second way to send, drawn beside the first.
     *
     * A remark on a line has two of them — held back for the review, or said
     * now — and the box should not have to know which is which.
     */
    secondLabel?: string
  }>(),
  {
    placeholder: 'Leave a comment',
    busy: false,
    sendLabel: 'Comment',
    compact: false,
    autofocus: false,
    modelValue: undefined,
    cancellable: true,
    secondLabel: undefined
  }
)

const emit = defineEmits<{
  send: [body: string]
  second: [body: string]
  'update:modelValue': [body: string]
  cancel: []
}>()

/** Text the caller holds, when it holds it. */
const bound = computed(() => props.modelValue !== undefined)
const own = ref('')

/** One place to read and write whichever copy is in force. */
const text = computed({
  get: () => (bound.value ? (props.modelValue ?? '') : own.value),
  set: (value: string) => {
    if (bound.value) emit('update:modelValue', value)
    else own.value = value
  }
})

const field = ref<HTMLTextAreaElement | null>(null)

/**
 * Dragging or pasting a screenshot onto the box writes it into the body.
 *
 * Lives here rather than at each caller, so a remark on a line, a reply and
 * the conversation's own composer all take a picture the same way.
 */
const attach = useAttachments({
  field: () => field.value,
  text: () => text.value,
  write: (value) => {
    text.value = value
  }
})

onMounted(async () => {
  if (props.autofocus) {
    await nextTick()
    field.value?.focus()
  }
})

function focus() {
  field.value?.focus()
}

defineExpose({ focus })

/** Nothing goes while a picture in it is still on its way. */
const ready = computed(() => Boolean(text.value.trim()) && !props.busy && !attach.sending.value)

function send() {
  if (!ready.value) return
  emit('send', text.value)
  // Cleared by the caller when it holds the text; a failed send then leaves
  // the remark in place to be sent again.
  if (!bound.value) own.value = ''
}

/** The other way to send it, for a box that offers one. */
function sendSecond() {
  if (!ready.value) return
  emit('second', text.value)
  if (!bound.value) own.value = ''
}

function cancel() {
  if (!bound.value) own.value = ''
  emit('cancel')
}

// Enter means a newline, as it does wherever markdown is being written; the
// send is the chord, and Escape gives the text back.
function onKey(event: KeyboardEvent) {
  if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
    event.preventDefault()
    send()
  } else if (event.key === 'Escape') {
    event.preventDefault()
    cancel()
  }
}
</script>

<template>
  <div class="box" :class="{ compact: props.compact, over: attach.over.value }">
    <textarea
      ref="field"
      v-model="text"
      :rows="props.compact ? 2 : 3"
      :placeholder="props.placeholder"
      data-testid="comment-input"
      @keydown="onKey"
      @dragover="attach.onDragOver"
      @dragleave="attach.onDragLeave"
      @drop="attach.onDrop"
      @paste="attach.onPaste"
    />

    <p v-if="attach.failure.value" class="trouble" data-testid="attach-failure">
      {{ attach.failure.value }}
    </p>

    <div class="actions">
      <span v-if="attach.sending.value" class="hint faint" data-testid="attach-busy">
        <Spinner :size="10" />
        Attaching {{ attach.sending.value === 1 ? 'an image' : `${attach.sending.value} images` }}…
      </span>
      <span v-else class="hint faint">Ctrl+Enter to send · drop an image to attach it</span>
      <span class="grow" />
      <button v-if="props.cancellable" class="btn btn-ghost" @click="cancel">Cancel</button>
      <button
        v-if="props.secondLabel"
        class="btn btn-ghost"
        data-testid="comment-second"
        :disabled="!ready"
        @click="sendSecond"
      >
        {{ props.secondLabel }}
      </button>
      <button class="btn btn-primary" data-testid="comment-send" :disabled="!ready" @click="send">
        <Spinner v-if="props.busy" :size="11" />
        {{ props.sendLabel }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.box {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

textarea {
  width: 100%;
  font-family: var(--font);
  font-size: 12px;
  line-height: 1.5;
  resize: vertical;
}

.compact textarea {
  padding: 4px 7px;
}

/* An image is over the box and will be written into it if it is let go. */
.over textarea {
  border-color: var(--accent);
  box-shadow: inset 0 0 0 1px var(--accent);
}

.trouble {
  margin: 0;
  font-size: 11px;
  color: var(--danger-soft);
}

.hint :deep(svg) {
  vertical-align: -1px;
  margin-right: 3px;
}

.actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.hint {
  font-size: 10.5px;
}

.grow {
  flex: 1;
}
</style>
