<script setup lang="ts">
import { CODE_FONT_DEFAULT, fontToast, holdFontToast, resetFromToast } from '~/composables/useCodeFont'

/**
 * The code's new size, for a moment, at the foot of the code it was changed
 * over: "Font size 15 px", and a way back to the standard size when it is not
 * at it. Another press changes the number in place and starts the clock again;
 * the pointer on it keeps it up. Not one of the notices in the corner: those
 * are for things that happened, and this is the answer to a key.
 */
</script>

<template>
  <Transition name="font-toast">
    <div
      v-if="fontToast.shown"
      class="font-toast"
      role="status"
      :style="{ left: `${fontToast.x}px`, bottom: `${fontToast.bottom}px` }"
      @mouseenter="holdFontToast(true)"
      @mouseleave="holdFontToast(false)"
    >
      <span class="what">Font size {{ fontToast.size }} px</span>
      <button v-if="fontToast.size !== CODE_FONT_DEFAULT" class="reset" type="button" @click="resetFromToast">
        Reset to {{ CODE_FONT_DEFAULT }} px
      </button>
    </div>
  </Transition>
</template>

<style scoped>
.font-toast {
  position: fixed;
  z-index: 300;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 8px 6px 12px;
  font-size: 12px;
  color: var(--text);
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  box-shadow: var(--shadow-pop);
  white-space: nowrap;
}

.what {
  font-variant-numeric: tabular-nums;
}

.reset {
  padding: 2px 8px;
  font-size: 11.5px;
  color: var(--primary);
  border-radius: var(--radius-sm);
}

.reset:hover {
  background: var(--hover);
}

.font-toast-enter-active,
.font-toast-leave-active {
  transition: opacity 0.15s ease;
}

.font-toast-enter-from,
.font-toast-leave-to {
  opacity: 0;
}
</style>
