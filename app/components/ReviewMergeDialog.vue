<script setup lang="ts">
import { computed, ref } from 'vue'
import { GitMerge } from 'lucide-vue-next'
import AppModal from './AppModal.vue'
import Spinner from './Spinner.vue'
import { useReview } from '~/composables/useReview'
import { useForge } from '~/composables/useForge'
import { mergeSummary } from '~/composables/reviewLook'
import { useGit } from '~/composables/useGit'

/**
 * The last step before a branch lands.
 *
 * Merging used to be a button that changed its own label for three seconds
 * and then did it. This says what is about to happen instead — which commits,
 * in what shape, and what becomes of the branch — and asks once.
 */
const emit = defineEmits<{ close: [] }>()

const review = useReview()
const forge = useForge()
const git = useGit()
const store = review.store

const squash = ref(false)
const removeBranch = ref(false)

const one = computed(() => store.detail ?? store.current)
const state = computed(() => {
  const here = one.value
  if (!here) return ''
  if (here.draft) return 'draft'
  const raw = here.state.toLowerCase()
  return raw === 'opened' ? 'open' : raw
})

const summary = computed(() =>
  mergeSummary(store.status, state.value, store.detail?.draft ?? false)
)

/** A fork's branch is not ours to delete, so the offer is not made. */
const ours = computed(() => !store.current?.source?.is_fork)

const commits = computed(() => store.commits.length)

async function merge() {
  const note = await review.merge(squash.value, removeBranch.value && ours.value)
  if (note) git.note(note)
  emit('close')
}
</script>

<template>
  <AppModal :title="`Merge ${forge.shortLabel.value} ${forge.sigil.value}${one?.number}`" :width="520" @close="emit('close')">
    <p class="standing" :class="summary.tone">
      <strong>{{ summary.title }}</strong>
      <span v-if="summary.detail" class="faint">{{ summary.detail }}</span>
    </p>

    <p class="what">
      <span class="mono">{{ one?.source_branch }}</span>
      goes into
      <span class="mono">{{ one?.target_branch }}</span>
      <span v-if="commits" class="faint">
        · {{ commits }} {{ commits === 1 ? 'commit' : 'commits' }}
      </span>
    </p>

    <div class="choices">
      <label class="choice">
        <input v-model="squash" type="radio" :value="false" />
        <span>
          <strong>Merge commit</strong>
          <em class="faint">Every commit of the branch, kept as it was written.</em>
        </span>
      </label>
      <label class="choice">
        <input v-model="squash" type="radio" :value="true" />
        <span>
          <strong>Squash and merge</strong>
          <em class="faint">One commit on the target, whatever the branch took.</em>
        </span>
      </label>
    </div>

    <label v-if="ours" class="after">
      <input v-model="removeBranch" type="checkbox" data-testid="merge-delete-branch" />
      Delete <span class="mono">{{ one?.source_branch }}</span> afterwards
    </label>

    <template #footer>
      <button class="btn" @click="emit('close')">Cancel</button>
      <button
        class="btn btn-primary go"
        data-testid="merge-confirm"
        :disabled="store.acting !== null"
        @click="merge"
      >
        <Spinner v-if="store.acting === 'merge'" :size="12" />
        <GitMerge v-else :size="14" />
        {{ squash ? 'Squash and merge' : 'Merge' }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.standing {
  display: flex;
  flex-direction: column;
  gap: 3px;
  margin: 0 0 12px;
  padding: 9px 11px;
  border-radius: 6px;
  border-left: 3px solid var(--line);
  background: var(--bg-raised);
  font-size: 12.5px;
}

.standing.good {
  border-left-color: var(--green);
}

.standing.bad {
  border-left-color: var(--red);
}

.standing.wait {
  border-left-color: var(--amber);
}

.standing .faint {
  font-size: 11.5px;
}

.what {
  margin: 0 0 12px;
  font-size: 12px;
  color: var(--text-dim);
}

.choices {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.choice {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  padding: 8px 10px;
  border: 1px solid var(--line-soft);
  border-radius: 6px;
  cursor: pointer;
  font-size: 12px;
}

.choice:hover {
  background: var(--bg-hover);
}

.choice:has(input:checked) {
  border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  background: color-mix(in srgb, var(--accent) 9%, transparent);
}

.choice input {
  margin-top: 2px;
  accent-color: var(--accent);
}

.choice span {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.choice em {
  font-style: normal;
  font-size: 11px;
}

.after {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 12px;
  font-size: 12px;
  color: var(--text-dim);
  cursor: pointer;
}

.after input {
  accent-color: var(--accent);
}

.go {
  min-width: 120px;
  justify-content: center;
}
</style>
