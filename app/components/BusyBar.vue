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
/* A line along the top edge of the panes, drawn only while something is
   running. It takes no height of its own, so the panes never jump when it
   comes and goes. */
.wrap {
  position: relative;
  height: 2px;
  margin: 0 0 -2px;
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
  background: color-mix(in srgb, var(--primary) 12%, transparent);
}

/* An indeterminate sweep: git gives no progress, so pretending otherwise would
   be a lie. This only says "still working". */
.sweep {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 30%;
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

/* What is running, as a small tag under the line's right end. */
.label {
  position: absolute;
  right: 8px;
  top: 8px;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--line);
  font-size: 11.5px;
  color: var(--text-dim);
  background: var(--bg);
  white-space: nowrap;
}
</style>
