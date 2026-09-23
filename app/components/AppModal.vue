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
/* The scrim dims the window and softens it, so the dialog is the one sharp
   thing on screen. */
.scrim {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: grid;
  place-items: center;
  background: var(--overlay);
  backdrop-filter: blur(3px);
  animation: scrim-in 0.14s ease-out;
}

/* A card lifted off everything, with no rule round it: the shadow is the edge. */
.modal {
  max-width: calc(100vw - 40px);
  max-height: calc(100vh - 60px);
  display: flex;
  flex-direction: column;
  background: var(--bg);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-pop);
  animation: modal-in 0.16s ease-out;
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 18px 16px 4px 22px;
}

.head h2 {
  margin: 0;
  font-size: 16.5px;
  font-weight: 650;
  letter-spacing: -0.015em;
}

.head .btn {
  width: 30px;
  min-height: 30px;
  padding: 0;
  border-radius: var(--radius-pill);
  color: var(--text-faint);
}

.content {
  padding: 12px 22px 20px;
  overflow: auto;
}

/* No rule over the actions: the space above them is the separation, and the
   pills on the right are where the eye ends up anyway. */
.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 14px 22px 20px;
  background: var(--bg);
  border-bottom-left-radius: var(--radius-lg);
  border-bottom-right-radius: var(--radius-lg);
}

@keyframes scrim-in {
  from {
    opacity: 0;
  }
}

@keyframes modal-in {
  from {
    opacity: 0;
    transform: translateY(4px) scale(0.99);
  }
}
</style>
