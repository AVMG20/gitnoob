<script setup lang="ts">
import { computed } from 'vue'
import { useGit } from '~/composables/useGit'
import { useAi } from '~/composables/useAi'

const git = useGit()
const ai = useAi()

/** The AI calls take seconds, so they get named too. */
const label = computed(() => {
  if (ai.store.busy) return `Asking the model for a ${ai.store.busy}…`
  if (git.store.busyLabel) return `${git.store.busyLabel}…`
  return null
})
</script>

<template>
  <div class="wrap" :class="{ on: !!label }">
    <div class="track"><div class="sweep" /></div>
    <span v-if="label" class="label">{{ label }}</span>
  </div>
</template>

<style scoped>
/* A hairline between the toolbar and the cards, drawn only while something is
   running. It takes no height of its own, so the cards never jump when it
   comes and goes. */
.wrap {
  position: relative;
  height: 2px;
  margin: -2px calc(var(--gutter) + var(--radius-lg)) 0;
  opacity: 0;
  transition: opacity 0.2s;
  pointer-events: none;
  z-index: 7;
}

.wrap.on {
  opacity: 1;
}

.track {
  position: absolute;
  inset: 0;
  overflow: hidden;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, var(--primary) 10%, transparent);
}

/* An indeterminate sweep: git gives no progress, so pretending otherwise would
   be a lie. This only says "still working". */
.sweep {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 30%;
  border-radius: var(--radius-pill);
  background: linear-gradient(90deg, transparent, var(--primary), transparent);
  animation: sweep 1.3s cubic-bezier(0.45, 0, 0.55, 1) infinite;
}

@keyframes sweep {
  0% {
    left: -30%;
  }
  100% {
    left: 100%;
  }
}

/* What is running, as a small floating chip under the line's right end. */
.label {
  position: absolute;
  right: 0;
  top: 10px;
  padding: 3px 11px;
  border-radius: var(--radius-pill);
  font-size: 11.5px;
  font-weight: 500;
  color: var(--text);
  background: var(--bg);
  box-shadow: var(--shadow-pop);
  white-space: nowrap;
}
</style>
