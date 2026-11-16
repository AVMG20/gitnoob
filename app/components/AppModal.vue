<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue'
import { X } from 'lucide-vue-next'

/**
 * `keep`: a click on the scrim does nothing. For dialogs that hold typed
 * work, where one stray click outside the box would throw it all away.
 * Escape and the ✕ still close them; both are deliberate.
 */
const props = defineProps<{
  title: string
  width?: number
  keep?: boolean
  /** A command palette: no title row, near the top, most of the height. Esc
      and the scrim close it. The title is still its accessible name. */
  palette?: boolean
}>()
const emit = defineEmits<{ close: [] }>()

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') emit('close')
}

/**
 * Focus goes into the dialog, stays in it, and goes back where it was.
 *
 * Left behind the scrim, Tab walked the window under the dialog and Enter
 * could press Pull or Push while a "Delete branch?" was open. The box itself
 * takes focus, not its first button, so Enter on opening confirms nothing; a
 * dialog that puts the caret in a field itself is left alone.
 */
const box = ref<HTMLElement | null>(null)
let before: Element | null = null

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

function focusables() {
  return [...(box.value?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])]
}

function onTab(event: KeyboardEvent) {
  // A palette has its own use for Tab (the search window's modes).
  if (event.key !== 'Tab' || !box.value || props.palette || event.defaultPrevented) return
  const all = focusables()
  if (!all.length) {
    event.preventDefault()
    return
  }
  const first = all[0]!
  const last = all[all.length - 1]!
  const active = document.activeElement
  const inside = active instanceof Node && box.value.contains(active)
  if (!inside || (event.shiftKey && (active === first || active === box.value))) {
    event.preventDefault()
    ;(event.shiftKey ? last : first).focus()
  } else if (!event.shiftKey && active === last) {
    event.preventDefault()
    first.focus()
  }
}

onMounted(async () => {
  window.addEventListener('keydown', onKey)
  before = document.activeElement
  await nextTick()
  const active = document.activeElement
  if (box.value && !(active instanceof Node && box.value.contains(active))) box.value.focus()
})
onUnmounted(() => {
  window.removeEventListener('keydown', onKey)
  if (before instanceof HTMLElement && before.isConnected) before.focus()
})

/**
 * A click lands on the scrim whenever the press and the release straddle
 * it: select text in a field, drag past the edge, let go, and the browser
 * reports a click on the scrim. Only a press that started there counts.
 */
const pressed = ref(false)

function onDown(event: PointerEvent) {
  pressed.value = event.target === event.currentTarget
}

function onScrim() {
  const started = pressed.value
  pressed.value = false
  if (started && !props.keep) emit('close')
}
</script>

<template>
  <div class="scrim" :class="{ palette: props.palette }" @pointerdown="onDown" @click.self="onScrim">
    <div
      ref="box"
      class="modal"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      :aria-label="props.title"
      :style="{ width: `${props.width ?? 460}px` }"
      @keydown="onTab"
    >
      <div v-if="!props.palette" class="head">
        <h2>{{ props.title }}</h2>
        <button class="btn" title="Close" @click="emit('close')"><X :size="16" /></button>
      </div>
      <div class="content">
        <slot />
      </div>
      <div v-if="$slots.footer" class="footer">
        <slot name="footer" />
      </div>
    </div>
  </div>
</template>

<style scoped>
/* The scrim dims the window so the dialog is the thing on screen. No blur:
   what is behind stays readable, which is often why the dialog was opened. */
.scrim {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: grid;
  place-items: center;
  background: var(--overlay);
  animation: scrim-in 0.1s ease-out;
}

/* A box with a hairline and a shadow: the head, the body, and the actions in a
   footer of their own, in the chrome's tone. */
.modal {
  max-width: calc(100vw - 40px);
  max-height: calc(100vh - 60px);
  display: flex;
  flex-direction: column;
  background: var(--bg);
  border-radius: 10px;
  box-shadow: var(--shadow-pop);
  overflow: hidden;
  animation: modal-in 0.1s ease-out;
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 44px;
  padding: 8px 8px 8px 18px;
  border-bottom: 1px solid var(--line-soft);
}

.head h2 {
  margin: 0;
  font-size: 15px;
  font-weight: 650;
}

.head .btn {
  width: 28px;
  padding: 0;
  color: var(--text-faint);
}

.content {
  padding: 16px 18px 18px;
  overflow: auto;
}

.footer {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  background: var(--canvas);
  border-top: 1px solid var(--line-soft);
}

/* A palette sits near the top, like a command palette, and takes most of the
   height for what it lists. */
.scrim.palette {
  place-items: start center;
  padding-top: 4vh;
}

.modal:focus {
  outline: none;
}

.palette .modal {
  height: 88vh;
}

.palette .content {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  padding: 10px;
  overflow: hidden;
}

@keyframes scrim-in {
  from {
    opacity: 0;
  }
}

@keyframes modal-in {
  from {
    opacity: 0;
    transform: scale(0.985);
  }
}
</style>
