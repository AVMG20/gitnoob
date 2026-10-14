<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { GitMerge } from 'lucide-vue-next'
import AppModal from './AppModal.vue'
import Spinner from './Spinner.vue'
import { useReview } from '~/composables/useReview'
import { useForge, type MergeMethod } from '~/composables/useForge'
import { mergeSummary } from '~/composables/reviewLook'
import { useGit } from '~/composables/useGit'
import { offerBranchCleanup } from '~/composables/useForgeNudges'

/**
 * The last step before a branch lands.
 *
 * Merging used to be a button that changed its own label for three seconds
 * and then did it. This says what is about to happen instead — which commits,
 * in what shape, and what becomes of the branch — and asks once. It offers
 * only the ways the project allows, and stays open with the forge's answer
 * when the merge is refused, so the reason is read where the click was made.
 */
const emit = defineEmits<{ close: [] }>()

const review = useReview()
const forge = useForge()
const git = useGit()
const store = review.store

const ALL: { value: MergeMethod; title: string; text: string }[] = [
  { value: 'merge', title: 'Merge commit', text: 'Every commit of the branch, kept as it was written.' },
  { value: 'squash', title: 'Squash and merge', text: 'One commit on the target, whatever the branch took.' },
  { value: 'rebase', title: 'Rebase and merge', text: 'The commits replayed onto the target, one by one, with no merge commit.' }
]

/** The ways this project allows, in the order above; every one until it says. */
const methods = computed(() => {
  const allowed = forge.facts.value?.merge_methods
  // GitLab merges the way the project is set up to; rebasing is not a
  // choice made at the button there.
  const offered = forge.store.status?.kind === 'gitlab' ? ALL.filter((m) => m.value !== 'rebase') : ALL
  if (!allowed?.length) return offered
  const kept = offered.filter((m) => allowed.includes(m.value))
  return kept.length ? kept : offered
})

const method = ref<MergeMethod>('merge')
const removeBranch = ref(false)
const error = ref<string | null>(null)

/** The first allowed way, unless the one chosen is still allowed. */
function settle() {
  if (!methods.value.some((m) => m.value === method.value)) {
    method.value = methods.value[0]?.value ?? 'merge'
  }
}
settle()

onMounted(async () => {
  await forge.loadFacts()
  settle()
})

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

const label = computed(() => ALL.find((m) => m.value === method.value)?.title ?? 'Merge')

async function merge() {
  error.value = null
  const source = one.value?.source_branch ?? ''
  const head = store.detail?.head_sha || store.current?.head_sha || null
  const note = await review.merge(method.value, removeBranch.value && ours.value)
  if (!note) {
    // Refused: say why here, and leave the choice where it was.
    error.value = forge.store.error ?? 'The forge did not merge it'
    return
  }
  git.note(note)
  emit('close')
  // The clone catches up with what just happened on the forge.
  if (source) await offerBranchCleanup(source, head)
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
      <label v-for="way in methods" :key="way.value" class="choice">
        <input v-model="method" type="radio" :value="way.value" :data-testid="`merge-method-${way.value}`" />
        <span>
          <strong>{{ way.title }}</strong>
          <em class="faint">{{ way.text }}</em>
        </span>
      </label>
    </div>

    <label v-if="ours" class="after">
      <input v-model="removeBranch" type="checkbox" data-testid="merge-delete-branch" />
      Delete <span class="mono">{{ one?.source_branch }}</span> afterwards
    </label>

    <p v-if="error" class="refused" data-testid="merge-error">{{ error }}</p>

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
        {{ method === 'merge' ? 'Merge' : label }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.standing {
  display: flex;
  flex-direction: column;
  gap: 3px;
  margin: 0 0 14px;
  padding: 9px 12px;
  border-radius: var(--radius);
  border: 1px solid var(--line);
  background: var(--surface);
  font-size: 13px;
}

.standing.good {
  background: var(--success-bg);
  border-color: var(--success-line);
  color: var(--success-soft);
}

.standing.bad {
  background: var(--danger-bg);
  border-color: var(--danger-line);
  color: var(--danger-soft);
}

.standing.wait {
  background: var(--warning-bg);
  border-color: var(--warning-line);
  color: var(--warning-soft);
}

.standing .faint {
  font-size: 12px;
}

.what {
  margin: 0 0 12px;
  font-size: 12.5px;
  color: var(--text-dim);
}

/* The three ways to merge, as one bordered group of radio rows: the chosen
   one is tinted in the accent. */
.choices {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  overflow: hidden;
}

.choice {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 9px 12px;
  cursor: pointer;
  font-size: 12.5px;
}

.choice + .choice {
  border-top: 1px solid var(--line);
}

.choice:hover {
  background: var(--bg-hover);
}

.choice:has(input:checked) {
  background: var(--primary-bg);
}

.choice input {
  margin-top: 3px;
  accent-color: var(--accent);
}

.choice span {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.choice em {
  font-style: normal;
  font-size: 11.5px;
}

.after {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 14px;
  font-size: 12.5px;
  color: var(--text-dim);
  cursor: pointer;
}

.after input {
  accent-color: var(--accent);
}

.refused {
  margin: 12px 0 0;
  padding: 8px 11px;
  border-radius: var(--radius);
  border: 1px solid var(--danger-line);
  background: var(--danger-bg);
  color: var(--danger-soft);
  font-size: 12px;
  line-height: 1.45;
}

.go {
  min-width: 128px;
  justify-content: center;
}
</style>
