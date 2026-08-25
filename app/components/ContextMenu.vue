<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { ChevronRight } from 'lucide-vue-next'
import { useContextMenu, type MenuItem } from '~/composables/useContextMenu'

const menu = useContextMenu()
const state = menu.state
const box = ref<HTMLElement | null>(null)
const offset = ref({ x: 0, y: 0 })

/** Which row's nested menu is open, and whether it had to open leftwards. */
const sub = ref<number | null>(null)
const flipSub = ref(false)

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
  // A row with children is a door, not a button: clicking it opens them rather
  // than doing something the user has not chosen yet.
  if (item.children?.length) {
    openSub(index)
    return
  }
  menu.close()
  await item.action?.()
}

async function runChild(child: MenuItem) {
  if (child.disabled || child.separator) return
  menu.close()
  await child.action?.()
}

/**
 * Opens a row's nested menu, deciding first whether it fits to the right.
 *
 * The menu is at most 320px wide, so anything closer than that to the right
 * edge of the window opens leftwards instead.
 */
function openSub(index: number) {
  const item = state.items[index]
  if (!item?.children?.length) {
    sub.value = null
    return
  }
  const right = box.value?.getBoundingClientRect().right ?? 0
  flipSub.value = right + 200 > window.innerWidth - 8
  sub.value = index
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape') menu.close()
}

watch(
  () => state.open,
  async (open) => {
    offset.value = { x: 0, y: 0 }
    sub.value = null
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
          :class="{ danger: item.danger, off: item.disabled, parent: item.children?.length }"
          :disabled="item.disabled"
          @click="run(index)"
          @mouseenter="openSub(index)"
        >
          <component :is="item.icon" v-if="item.icon" :size="14" class="icon" />
          <span class="label">{{ item.label }}</span>
          <span v-if="item.hint" class="hint">{{ item.hint }}</span>
          <ChevronRight v-if="item.children?.length" :size="13" class="arrow" />

          <!-- The nested menu, anchored to its row. Flipped to the left when
               there is no room on the right, the same way the parent menu is
               nudged back inside the window. -->
          <span
            v-if="item.children?.length && sub === index"
            class="submenu"
            :class="{ flip: flipSub }"
            @mouseenter="openSub(index)"
          >
            <button
              v-for="(child, childIndex) in item.children"
              :key="childIndex"
              class="item"
              :class="{ danger: child.danger, off: child.disabled }"
              :disabled="child.disabled"
              @click.stop="runChild(child)"
            >
              <component :is="child.icon" v-if="child.icon" :size="14" class="icon" />
              <span class="label">{{ child.label }}</span>
              <span v-if="child.hint" class="hint">{{ child.hint }}</span>
            </button>
          </span>
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
  /* Wide enough that the longest action, with its hint beside it, is not
     ellipsised — a truncated verb is worse than a wide menu. Branch names can
     run long, so the ceiling is generous but still leaves the fit() nudge room
     to keep the menu inside the window. */
  max-width: min(560px, calc(100vw - 24px));
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
  position: relative;
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

.arrow {
  flex: none;
  opacity: 0.6;
}

/* The nested menu hangs off its row, overlapping the parent's edge slightly so
   the pointer can cross between the two without falling into the gap and
   closing it. */
.submenu {
  position: absolute;
  top: -5px;
  left: 100%;
  z-index: 1;
  min-width: 232px;
  max-width: min(560px, calc(100vw - 24px));
  margin-left: -3px;
  padding: 4px;
  background: var(--bg-raised);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: 0 12px 34px rgba(0, 0, 0, 0.55);
}

.submenu.flip {
  left: auto;
  right: 100%;
  margin-left: 0;
  margin-right: -3px;
}

.item:hover:not(:disabled) {
  background: var(--bg-active);
}

.item.danger {
  color: var(--red-soft);
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
  min-width: 0;
  /* A wrapped label makes a row twice as tall as its neighbours and turns an
     even list into a ragged one; the menu grows sideways instead. */
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
