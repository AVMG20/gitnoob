<script setup lang="ts">
import { computed } from 'vue'

/**
 * A name too long for its column, cut in the middle rather than at the end.
 *
 * Branch names in this line of work are prefix-heavy — `origin/ASANA-1216293…`
 * four times over — and it is the tail that says which one this is. Two spans
 * do it rather than measuring: the head is allowed to shrink and take the
 * ellipsis, the tail never shrinks, so the cut lands wherever the real width
 * puts it and no character is counted anywhere.
 */
const props = withDefaults(defineProps<{ text: string; tail?: number }>(), { tail: 7 })

const head = computed(() => props.text.slice(0, Math.max(0, props.text.length - props.tail)))
const rest = computed(() => props.text.slice(head.value.length))
</script>

<template>
  <span class="mid"><span class="head">{{ head }}</span><span class="tail">{{ rest }}</span></span>
</template>

<style scoped>
.mid {
  display: flex;
  align-items: baseline;
  min-width: 0;
  overflow: hidden;
}

.head {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

.tail {
  flex: none;
  white-space: nowrap;
}
</style>
