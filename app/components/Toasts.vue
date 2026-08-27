<script setup lang="ts">
import { ref } from 'vue'
import { CircleAlert, Copy, Info, X } from 'lucide-vue-next'
import { useToasts } from '~/composables/useToasts'

/**
 * The stack in the bottom-left corner.
 *
 * Above the activity log rather than inside it: the log is a record you go and
 * look at, and a failure is something that has to come to you.
 */
const toasts = useToasts()

/** Which notices have been opened up to show what git actually said. */
const opened = ref(new Set<number>())

/** True while the pointer or the keyboard is in the stack. */
const inside = ref(false)

/**
 * Stops the clocks while the stack is being read.
 *
 * Being read is either of two things: the pointer is on it, or something in it
 * has been opened up to show what git said. A notice taken away in the middle
 * of either is a notice that may as well not have been shown.
 */
function watching(on: boolean) {
  inside.value = on
  toasts.hold(on || opened.value.size > 0)
}

function toggle(id: number) {
  const next = new Set(opened.value)
  if (!next.delete(id)) next.add(id)
  opened.value = next
  toasts.hold(inside.value || next.size > 0)
}

async function copy(text: string) {
  try {
    await navigator.clipboard.writeText(text)
  } catch {
    // Nothing to say about it: the text is on screen either way.
  }
}
</script>

<template>
  <div
    v-if="toasts.items.value.length"
    class="toasts"
    @mouseenter="watching(true)"
    @mouseleave="watching(false)"
    @focusin="watching(true)"
    @focusout="watching(false)"
  >
    <button
      v-if="toasts.items.value.length > 1"
      class="clear"
      @click="toasts.clear()"
    >
      Dismiss all
    </button>

    <div
      v-for="toast in toasts.items.value"
      :key="toast.id"
      class="toast"
      :class="toast.level"
      role="status"
    >
      <div class="head">
        <component :is="toast.level === 'error' ? CircleAlert : Info" :size="14" class="icon" />
        <span class="title">{{ toast.title }}</span>
        <span v-if="toast.count > 1" class="count">×{{ toast.count }}</span>
        <button class="close" title="Dismiss" @click="toasts.dismiss(toast.id)">
          <X :size="13" />
        </button>
      </div>

      <div v-if="toast.detail" class="more">
        <button class="link" @click="toggle(toast.id)">
          {{ opened.has(toast.id) ? 'Hide what git said' : 'What git said' }}
        </button>
        <button v-if="opened.has(toast.id)" class="link" @click="copy(toast.detail)">
          <Copy :size="11" /> Copy
        </button>
      </div>
      <pre v-if="toast.detail && opened.has(toast.id)" class="detail">{{ toast.detail }}</pre>
    </div>
  </div>
</template>

<style scoped>
.toasts {
  position: fixed;
  left: 12px;
  bottom: 36px;
  z-index: 60;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 8px;
  /* A stated width rather than one taken from the longest sentence in the
     stack: notices that arrive one after another should not each be a
     different shape, and a fixed flex box shrinks to its content. */
  width: min(420px, calc(100vw - 24px));
}

.clear {
  align-self: flex-start;
  padding: 2px 8px;
  font-size: 11px;
  color: var(--text-dim);
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: 999px;
}

.clear:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.toast {
  width: 100%;
  padding: 9px 10px;
  border: 1px solid var(--line);
  border-left: 3px solid var(--text-faint);
  border-radius: 8px;
  background: var(--bg-panel);
  box-shadow: 0 6px 20px var(--shadow);
}

.toast.error {
  border-left-color: var(--red);
}

.toast.info {
  border-left-color: var(--accent);
}

.head {
  display: flex;
  align-items: flex-start;
  gap: 7px;
}

.icon {
  flex: none;
  margin-top: 1px;
}

.error .icon {
  color: var(--red);
}

.info .icon {
  color: var(--accent);
}

.title {
  flex: 1;
  min-width: 0;
  font-size: 12.5px;
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.count {
  flex: none;
  align-self: center;
  padding: 0 5px;
  font-size: 10.5px;
  color: var(--text-dim);
  background: var(--bg-raised);
  border-radius: 999px;
}

.close {
  flex: none;
  color: var(--text-faint);
  border-radius: 5px;
  padding: 1px;
}

.close:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.more {
  display: flex;
  gap: 10px;
  margin-top: 5px;
  padding-left: 21px;
}

.link {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--text-dim);
}

.link:hover {
  color: var(--accent);
  text-decoration: underline;
}

.detail {
  margin: 6px 0 0 21px;
  padding: 7px 8px;
  max-height: 160px;
  overflow: auto;
  font-family: var(--mono);
  font-size: 11px;
  line-height: 1.45;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--text-dim);
  background: var(--bg-deep);
  border: 1px solid var(--line-soft);
  border-radius: 6px;
}
</style>
