<script setup lang="ts">
import { computed } from 'vue'
import { useGit } from '~/composables/useGit'

/**
 * The question asked before a stash goes.
 *
 * Dropping is the one stash action nothing undoes — the commit it holds is
 * unreachable the moment the entry is gone — so every way of asking for it
 * comes through here: the toolbar, the sidebar's menu and the graph's.
 */
const props = defineProps<{ index: number }>()
const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const stash = computed(() => store.stashes.find((one) => one.index === props.index) ?? null)

async function drop() {
  const said = await git.stashDrop(props.index)
  if (said !== null) git.note(said)
  emit('close')
}
</script>

<template>
  <AppModal title="Drop this stash?" :width="440" @close="emit('close')">
    <template v-if="stash">
      <p class="line">
        <strong>{{ stash.message }}</strong>
      </p>
      <p class="line dim">
        {{ stash.files }} {{ stash.files === 1 ? 'file' : 'files' }}{{
          stash.branch ? `, stashed on ${stash.branch}` : ''
        }}. The changes in it are thrown away and there is no undo.
      </p>
    </template>
    <p v-else class="line dim">That stash is no longer on the list.</p>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-danger" :disabled="store.busy || !stash" @click="drop">
        Drop stash
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.line {
  margin: 0 0 8px;
  font-size: 12.5px;
  line-height: 1.55;
  word-break: break-word;
}

.line:last-child {
  margin-bottom: 0;
}
</style>
