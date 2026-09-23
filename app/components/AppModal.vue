<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import { X } from 'lucide-vue-next'

/**
 * `keep`: a click on the scrim does nothing. For dialogs that hold typed
 * work, where one stray click outside the box would throw it all away.
 * Escape and the ✕ still close them; both are deliberate.
 */
const props = defineProps<{ title: string; width?: number; keep?: boolean }>()
const emit = defineEmits<{ close: [] }>()

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') emit('close')
}

onMounted(() => window.addEventListener('keydown', onKey))
onUnmounted(() => window.removeEventListener('keydown', onKey))

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
  <div class="scrim" @pointerdown="onDown" @click.self="onScrim">
    <div class="modal" :style="{ width: `${props.width ?? 460}px` }">
      <div class="head">
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
