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
.wrap {
  position: relative;
  height: 2px;
  opacity: 0;
  transition: opacity 0.14s;
  pointer-events: none;
}

.wrap.on {
  opacity: 1;
}

.track {
  position: absolute;
  inset: 0;
  overflow: hidden;
  background: rgba(79, 156, 249, 0.14);
}

/* An indeterminate sweep: git gives no progress, so pretending otherwise would
   be a lie. This only says "still working". */
.sweep {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 34%;
  background: linear-gradient(90deg, transparent, var(--accent), transparent);
  animation: sweep 1.15s ease-in-out infinite;
}

@keyframes sweep {
  0% {
    left: -34%;
  }
  100% {
    left: 100%;
  }
}

.label {
  position: absolute;
  right: 10px;
  top: 4px;
  padding: 2px 8px;
  border-radius: 0 0 5px 5px;
  font-size: 11px;
  color: var(--accent);
  background: var(--bg-panel);
  border: 1px solid var(--line);
  border-top: none;
  white-space: nowrap;
}
</style>
