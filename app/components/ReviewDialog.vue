<script setup lang="ts">
import { computed, ref } from 'vue'
import { useForge } from '~/composables/useForge'
import { useGit } from '~/composables/useGit'

const emit = defineEmits<{ close: [] }>()

const git = useGit()
const forge = useForge()
const store = git.store

/** The branch the review is opened from; the forge takes it from HEAD. */
const source = computed(() => (store.repo?.detached ? null : store.repo?.head ?? null))

const branch = computed(() => store.refs?.locals.find((b) => b.name === source.value) ?? null)

/**
 * A branch the forge has never seen cannot be reviewed, so the dialog offers
 * the push rather than letting the API refuse with something less helpful.
 */
const unpushed = computed(() => !!source.value && (!branch.value?.upstream || !!branch.value?.ahead))

/** Everything the target could reasonably be, nearest first. */
const targets = computed(() => {
  const locals = store.refs?.locals.map((b) => b.name) ?? []
  return locals.filter((name) => name !== source.value)
})

/** main, master, whatever this repository actually calls its trunk. */
function guessTarget(): string {
  const names = targets.value
  return names.find((n) => n === 'main') ?? names.find((n) => n === 'master') ?? names[0] ?? 'main'
}

/** The tip commit's subject says what the branch is for better than its name. */
function guessTitle(): string {
  const tip = store.rows.find((row) => row.oid === branch.value?.oid)
  return tip?.summary ?? (source.value ?? '')
}

const title = ref(guessTitle())
const body = ref('')
const target = ref(guessTarget())
const draft = ref(false)
const error = ref<string | null>(null)
const working = ref(false)

const label = computed(() => (forge.store.status?.kind === 'gitlab' ? 'merge request' : 'pull request'))

async function push() {
  if (!source.value) return
  working.value = true
  await git.pushBranch(source.value, !branch.value?.upstream)
  working.value = false
}

async function submit(andOpen: boolean) {
  if (!title.value.trim() || working.value) return
  working.value = true
  error.value = null
  try {
    const review = await forge.createReview(title.value.trim(), body.value, target.value, draft.value)
    await forge.loadReviews()
    git.note(`Opened !${review.number} ${review.title}`)
    if (andOpen) await forge.open(review.url)
    emit('close')
  } catch (e) {
    // The forge's own words: it names the real reason — no commits between the
    // branches, a review already open, a token without the scope.
    error.value = String(e)
  } finally {
    working.value = false
  }
}
</script>

<template>
  <AppModal :title="`New ${label}`" :width="520" @close="emit('close')">
    <p v-if="!source" class="hint bad">
      HEAD is detached. Check out a branch before opening a {{ label }}.
    </p>

    <template v-else>
      <p class="from">
        <span class="mono">{{ source }}</span>
        <span class="faint">into</span>
        <select v-model="target" class="target">
          <option v-for="name in targets" :key="name" :value="name">{{ name }}</option>
        </select>
      </p>

      <p v-if="unpushed" class="hint warn">
        <template v-if="!branch?.upstream">
          This branch is not on the remote yet, so the forge cannot see it.
        </template>
        <template v-else>
          {{ branch.ahead }} commit{{ branch.ahead === 1 ? '' : 's' }} here are not on the remote
          yet, so they would be missing from the {{ label }}.
        </template>
        <button class="btn btn-ghost inline" :disabled="working" @click="push">Push now</button>
      </p>

      <label class="field">
        <span class="label">Title</span>
        <input v-model="title" type="text" autofocus @keyup.enter="submit(false)" />
      </label>

      <label class="field">
        <span class="label">Description</span>
        <textarea v-model="body" rows="6" placeholder="What this changes, and why." />
      </label>

      <label class="check">
        <input v-model="draft" type="checkbox" />
        Open it as a draft
      </label>

      <p v-if="error" class="hint bad">{{ error }}</p>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn btn-ghost"
        :disabled="!source || working || !title.trim()"
        @click="submit(true)"
      >
        Create and open
      </button>
      <button
        class="btn btn-primary"
        :disabled="!source || working || !title.trim()"
        @click="submit(false)"
      >
        {{ working ? 'Working…' : 'Create' }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.from {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 14px;
}

.target {
  flex: 1;
  min-width: 0;
}

.field {
  display: block;
  margin-bottom: 14px;
}

.label {
  display: block;
  margin-bottom: 4px;
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.field input,
.field textarea {
  width: 100%;
  resize: vertical;
}

.check {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  color: var(--text-dim);
}

.hint {
  display: block;
  margin: 10px 0;
  font-size: 11px;
  line-height: 1.5;
}

.inline {
  margin-left: 6px;
  padding: 1px 7px;
}

.bad {
  color: var(--red);
}

.warn {
  color: var(--amber);
}
</style>
