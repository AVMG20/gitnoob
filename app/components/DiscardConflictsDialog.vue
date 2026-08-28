<script setup lang="ts">
import { computed } from 'vue'
import { useGit } from '~/composables/useGit'

/**
 * The question asked before conflicts are thrown away.
 *
 * The way out of a merge nobody wanted: the files go back to what the branch
 * already had. Asked because the other side lives only in the index — once the
 * conflict is cleared there is nothing left holding what was coming in.
 */
const props = defineProps<{ paths: string[] }>()
const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const one = computed(() => props.paths.length === 1)

async function discard() {
  if (props.paths.length) await git.conflictDiscard(props.paths)
  emit('close')
}
</script>

<template>
  <AppModal
    :title="one ? 'Throw this conflict away?' : `Throw ${props.paths.length} conflicts away?`"
    :width="440"
    @close="emit('close')"
  >
    <p class="line">
      {{ one ? 'The file goes' : 'The files go' }} back to what
      {{ store.repo?.head ? `${store.repo.head} already had` : 'the branch already had' }}. What the
      other side was bringing in is thrown away, and there is no undo.
    </p>
    <ul class="paths mono">
      <li v-for="path in props.paths.slice(0, 8)" :key="path" class="truncate">{{ path }}</li>
      <li v-if="props.paths.length > 8" class="faint">
        …and {{ props.paths.length - 8 }} more
      </li>
    </ul>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-danger" :disabled="store.busy || !props.paths.length" @click="discard">
        {{ one ? 'Throw it away' : `Throw ${props.paths.length} away` }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.line {
  margin: 0 0 10px;
  font-size: 12.5px;
  line-height: 1.55;
}

.paths {
  margin: 0;
  padding: 8px 10px;
  list-style: none;
  max-height: 160px;
  overflow: auto;
  font-size: 11.5px;
  color: var(--text-dim);
  background: var(--bg-deep);
  border: 1px solid var(--line-soft);
  border-radius: 6px;
}
</style>
