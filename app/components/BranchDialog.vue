<script setup lang="ts">
import { computed, ref } from 'vue'
import { useGit } from '~/composables/useGit'
import { branchNameProblem } from '~/composables/useBranchName'

const props = defineProps<{
  /** Where the branch starts. Empty means whatever is checked out. */
  start?: string
  /** Shown instead of the raw starting point, when the caller has something
      more readable to say than a hash. */
  startLabel?: string
  /**
   * Starting work rather than only making a branch: the start point's remote
   * is fetched first, and the branch tracks nothing until it is pushed. The
   * start point is the trunk as its remote has it unless one is given.
   */
  fresh?: boolean
}>()
const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const name = ref('')
const start = ref(props.start ?? '')

// Starting work from the trunk: ask the backend what that is right now, so
// the box shows `origin/main` rather than staying empty.
if (props.fresh && !props.start) {
  void git.trunkTarget().then((target) => {
    if (target && !start.value) start.value = target
  })
}

const taken = computed(() =>
  store.refs?.locals.some((b) => b.name === name.value.trim()) ?? false
)
/** What is wrong with the name, or null once git would take it. */
const problem = computed(() => branchNameProblem(name.value, taken.value))
const invalid = computed(() => problem.value !== null)

async function submit() {
  if (invalid.value) return
  // Null is the only failure: a command that succeeds quietly still returns a
  // string, and an empty one would otherwise leave the dialog open saying the
  // branch it had just made already exists.
  const created = props.fresh
    ? await git.startWork(name.value.trim(), start.value.trim() || undefined)
    : await git.createBranch(name.value.trim(), start.value.trim() || undefined)
  if (created !== null) emit('close')
}
</script>

<template>
  <AppModal :title="props.fresh ? 'Start work' : 'New branch'" :width="440" @close="emit('close')">
    <label class="field">
      <span class="label">Name</span>
      <input
        v-model="name"
        type="text"
        placeholder="feature/thing"
        autofocus
        @keyup.enter="submit"
      />
      <span v-if="name && problem" class="hint bad">{{ problem }}</span>
    </label>

    <label class="field">
      <span class="label">Starting point</span>
      <input v-model="start" type="text" :placeholder="store.repo?.head ?? 'HEAD'" />
      <span v-if="props.fresh" class="hint faint">
        Fetched first, so this is the newest there is. The new branch tracks nothing until you
        push it, and your open changes come along.
      </span>
      <span v-else-if="props.startLabel" class="hint faint">
        Branching from <span class="mono">{{ props.startLabel }}</span
        >. Change it here to start somewhere else.
      </span>
      <span v-else class="hint faint">A branch, tag or commit. Defaults to the current HEAD.</span>
    </label>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-primary" :disabled="store.busy || invalid" @click="submit">
        {{ props.fresh ? 'Fetch and start' : 'Create and check out' }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.field {
  display: block;
  margin-bottom: 14px;
}

.label {
  display: block;
  margin-bottom: 6px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-dim);
}

.field input {
  width: 100%;
  height: 30px;
  padding: 4px 9px;
}

.hint {
  display: block;
  margin-top: 4px;
  font-size: 11.5px;
}

.bad {
  color: var(--red);
}
</style>
