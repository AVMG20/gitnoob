<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'

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
        <button class="btn" @click="emit('close')">✕</button>
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
.scrim {
  position: fixed;
  inset: 0;
  z-index: 50;
  display: grid;
  place-items: center;
  background: var(--overlay);
}

.modal {
  max-width: calc(100vw - 40px);
  max-height: calc(100vh - 60px);
  display: flex;
  flex-direction: column;
  background: var(--bg-panel);
  border: 1px solid var(--line);
  border-radius: 9px;
  box-shadow: 0 18px 50px var(--shadow-strong);
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 11px 14px;
  border-bottom: 1px solid var(--line);
}

.head h2 {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
}

.content {
  padding: 14px;
  overflow: auto;
}

.footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 11px 14px;
  border-top: 1px solid var(--line);
}
</style>
