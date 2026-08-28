<script setup lang="ts">
/**
 * A question with two answers.
 *
 * For the actions that want a moment's thought but nothing to type: the name
 * back in a box is for what cannot be undone — a branch deleted for everyone,
 * uncommitted work thrown away — and asking for it anywhere else is a hurdle,
 * not a safeguard.
 */
const props = defineProps<{
  title: string
  /** What the action does, in a sentence or two. */
  hint?: string
  confirm?: string
  danger?: boolean
}>()
const emit = defineEmits<{ close: []; confirm: [] }>()
</script>

<template>
  <AppModal :title="props.title" :width="420" @close="emit('close')">
    <p v-if="props.hint" class="hint">{{ props.hint }}</p>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn"
        :class="props.danger ? 'btn-danger' : 'btn-primary'"
        autofocus
        @click="emit('confirm')"
      >
        {{ props.confirm ?? 'Continue' }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.hint {
  margin: 0;
  font-size: 12.5px;
  line-height: 1.55;
}
</style>
