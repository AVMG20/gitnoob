<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useContextMenu } from '~/composables/useContextMenu'

const menu = useContextMenu()
const state = menu.state
const box = ref<HTMLElement | null>(null)
const offset = ref({ x: 0, y: 0 })

const style = computed(() => ({
  left: `${state.x + offset.value.x}px`,
  top: `${state.y + offset.value.y}px`
}))

/** Nudges the menu back inside the window if it would hang off an edge. */
function fit() {
  const element = box.value
  if (!element) return
  const rect = element.getBoundingClientRect()
  const dx = rect.right > window.innerWidth - 8 ? window.innerWidth - 8 - rect.right : 0
  const dy = rect.bottom > window.innerHeight - 8 ? window.innerHeight - 8 - rect.bottom : 0
  offset.value = { x: dx, y: dy }
}

async function run(index: number) {
  const item = state.items[index]
  if (!item || item.disabled || item.separator) return
  menu.close()
  await item.action?.()
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') menu.close()
}

watch(
  () => state.open,
  async (open) => {
    offset.value = { x: 0, y: 0 }
    if (open) {
      await new Promise((resolve) => requestAnimationFrame(resolve))
      fit()
    }
  }
)

onMounted(() => window.addEventListener('keydown', onKey))
onUnmounted(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <div v-if="state.open" class="scrim" @click="menu.close()" @contextmenu.prevent="menu.close()">
    <div ref="box" class="menu" :style="style" @click.stop>
      <template v-for="(item, index) in state.items" :key="index">
        <div v-if="item.separator" class="divider" />
        <button
          v-else
          class="item"
          :class="{ danger: item.danger, off: item.disabled }"
          :disabled="item.disabled"
          @click="run(index)"
        >
          <component :is="item.icon" v-if="item.icon" :size="14" class="icon" />
          <span class="label">{{ item.label }}</span>
          <span v-if="item.hint" class="hint">{{ item.hint }}</span>
        </button>
      </template>
      <!-- What the menu is about, kept below the actions: it is there to
           confirm the right row was hit, not to be read first. -->
      <div v-if="state.title" class="title truncate">{{ state.title }}</div>
    </div>
  </div>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 80;
}

.menu {
  position: fixed;
  min-width: 216px;
  max-width: 320px;
  padding: 4px;
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: 0 12px 34px rgba(0, 0, 0, 0.55);
}

.title {
  padding: 6px 9px 5px;
  font-size: 11px;
  color: var(--text-faint);
  border-top: 1px solid var(--line-soft);
  margin-top: 4px;
}

.item {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 5px 9px;
  border-radius: 5px;
  text-align: left;
  font-size: 12.5px;
  color: var(--text);
}

.item:hover:not(:disabled) {
  background: var(--bg-active);
}

.item.danger {
  color: #ef8d9c;
}

.item.off {
  opacity: 0.4;
}

.icon {
  flex: none;
  opacity: 0.75;
}

.label {
  flex: 1;
}

.hint {
  font-size: 10.5px;
  color: var(--text-faint);
  white-space: nowrap;
}

.divider {
  height: 1px;
  margin: 4px 6px;
  background: var(--line);
}
</style>
