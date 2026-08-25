<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { ExternalLink, Sparkles, UserPlus } from 'lucide-vue-next'
import { useAi } from '~/composables/useAi'
import { useForge, type Member } from '~/composables/useForge'
import { useGit } from '~/composables/useGit'
import type { Choice } from '~/components/SearchSelect.vue'

const emit = defineEmits<{ close: [] }>()

const git = useGit()
const forge = useForge()
const ai = useAi()
const store = git.store

/** Every branch that could be either end of a review, local ones first. */
const locals = computed(() => store.refs?.locals ?? [])
const remoteOnly = computed(() => {
  const known = new Set(locals.value.map((b) => b.name))
  const seen = new Set<string>()
  return (store.refs?.remotes ?? []).filter((b) => {
    if (known.has(b.name) || seen.has(b.name)) return false
    seen.add(b.name)
    return true
  })
})

const source = ref(store.repo?.detached ? '' : store.repo?.head ?? '')
const target = ref('')
const title = ref('')
const body = ref('')
const draft = ref(false)
const assignees = ref<Member[]>([])
const reviewers = ref<Member[]>([])
const error = ref<string | null>(null)
const working = ref(false)
/** Set once the title has been typed in, so a branch change stops guessing. */
const edited = ref(false)

const label = computed(() => (forge.store.status?.kind === 'gitlab' ? 'merge request' : 'pull request'))
const forgeName = computed(() => (forge.store.status?.kind === 'gitlab' ? 'GitLab' : 'GitHub'))

/** The local branch behind a name, when there is one; remote-only has none. */
function local(name: string) {
  return locals.value.find((b) => b.name === name) ?? null
}

function options(exclude: string): Choice[] {
  const rows: Choice[] = locals.value
    .filter((b) => b.name !== exclude)
    .map((b) => ({
      value: b.name,
      label: b.name,
      note: !b.upstream ? 'not pushed' : b.ahead ? `${b.ahead} ahead` : undefined
    }))
  for (const b of remoteOnly.value) {
    if (b.name !== exclude) rows.push({ value: b.name, label: b.name, note: b.remote, hint: 'remote' })
  }
  return rows
}

const sourceOptions = computed(() => options(target.value))
const targetOptions = computed(() => options(source.value))

const branch = computed(() => local(source.value))

/**
 * A branch the forge has never seen cannot be reviewed, so the dialog offers
 * the push rather than letting the API refuse with something less helpful.
 */
const unpushed = computed(
  () => !!branch.value && (!branch.value.upstream || branch.value.ahead > 0)
)

/** main, master, whatever this repository actually calls its trunk. */
function guessTarget(): string {
  const names = [...locals.value.map((b) => b.name), ...remoteOnly.value.map((b) => b.name)].filter(
    (name) => name !== source.value
  )
  return names.find((n) => n === 'main') ?? names.find((n) => n === 'master') ?? names[0] ?? 'main'
}

/** The tip commit's subject says what a branch is for better than its name. */
function guessTitle(): string {
  const tip = store.rows.find((row) => row.oid === local(source.value)?.oid)
  return tip?.summary ?? source.value
}

target.value = guessTarget()
title.value = guessTitle()

// The title is a guess until it is typed in; while it is still a guess it
// follows whichever branch is being merged.
watch(source, () => {
  if (target.value === source.value) target.value = guessTarget()
  if (!edited.value) title.value = guessTitle()
})

onMounted(() => {
  forge.loadMembers()
  ai.refreshStatus()
})

/** Everyone on the project, with the signed-in account first. */
const people = computed(() => {
  const me = forge.store.me?.login
  if (!me) return forge.store.members
  return [...forge.store.members].sort((a, b) => Number(b.login === me) - Number(a.login === me))
})

/** The signed-in account as a member, when the forge has said who that is. */
const myself = computed<Member | null>(() => {
  const me = forge.store.me
  if (!me) return null
  return (
    forge.store.members.find((person) => person.login === me.login) ?? {
      id: me.id,
      login: me.login,
      name: me.login
    }
  )
})

const mine = computed(() => !!myself.value && assignees.value.some((a) => a.login === myself.value!.login))

function assignToMe() {
  const me = myself.value
  if (!me || mine.value) return
  assignees.value = [...assignees.value, me]
}

async function push() {
  if (!source.value) return
  working.value = true
  await git.pushBranch(source.value, !branch.value?.upstream)
  working.value = false
}

/** Writes both fields from the commits this branch has and the target does not. */
async function write() {
  if (!source.value || !target.value || ai.store.busy) return
  error.value = null
  try {
    const message = await ai.reviewMessage(source.value, target.value)
    if (!message) return
    title.value = message.summary
    body.value = message.body
    edited.value = true
  } catch (e) {
    error.value = String(e)
  }
}

/** Hands the half-written review to the forge's own page. */
async function handOver() {
  if (!source.value || !target.value) return
  working.value = true
  error.value = null
  try {
    const url = await forge.compareUrl(source.value, target.value, title.value.trim(), body.value)
    await forge.open(url)
    emit('close')
  } catch (e) {
    error.value = String(e)
  } finally {
    working.value = false
  }
}

async function submit(andOpen: boolean) {
  if (!ready.value || working.value) return
  working.value = true
  error.value = null
  try {
    const review = await forge.createReview({
      source: source.value,
      target: target.value,
      title: title.value.trim(),
      body: body.value,
      draft: draft.value,
      assignees: assignees.value,
      reviewers: reviewers.value
    })
    await forge.loadReviews()
    git.note(`Opened !${review.number} ${review.title}`)
    // GitHub takes the people in requests of their own, after the pull request
    // itself; if one of those failed the review still exists, and saying so is
    // more use than an error that suggests nothing happened.
    if (review.warning) git.note(review.warning)
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

const ready = computed(
  () => !!source.value && !!target.value && source.value !== target.value && !!title.value.trim()
)
</script>

<template>
  <AppModal :title="`New ${label}`" :width="560" @close="emit('close')">
    <p v-if="!locals.length" class="hint bad">
      This repository has no branches to merge yet.
    </p>

    <template v-else>
      <div class="ends">
        <label class="end">
          <span class="label">Merge</span>
          <SearchSelect
            v-model="source"
            :options="sourceOptions"
            placeholder="Branch to merge"
            mono
          />
        </label>
        <label class="end">
          <span class="label">Into</span>
          <SearchSelect v-model="target" :options="targetOptions" placeholder="Target branch" mono />
        </label>
      </div>

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

      <div class="field">
        <div class="head">
          <span class="label">Title and description</span>
          <button
            class="btn btn-ghost write"
            :disabled="!ai.configured.value || !!ai.store.busy || !ready"
            :title="
              ai.configured.value
                ? `Read the commits on ${source} that ${target} does not have, and write both`
                : 'Choose a model in Settings › AI first'
            "
            @click="write"
          >
            <Sparkles :size="13" />
            {{ ai.store.busy ? 'Reading the commits…' : 'Write with AI' }}
          </button>
        </div>
        <input
          v-model="title"
          type="text"
          autofocus
          placeholder="What this branch does"
          @input="edited = true"
          @keyup.enter="submit(false)"
        />
        <textarea
          v-model="body"
          rows="7"
          placeholder="What this changes, and why."
          @input="edited = true"
        />
      </div>

      <div class="who">
        <div class="row">
          <span class="label">Assignee</span>
          <PeoplePicker
            v-model="assignees"
            :people="people"
            :loading="forge.store.loadingMembers"
            :error="forge.store.membersError"
            placeholder="Assign someone"
          />
          <button
            v-if="myself && !mine"
            class="btn btn-ghost me"
            :title="`Assign this ${label} to ${myself.login}`"
            @click="assignToMe"
          >
            <UserPlus :size="12" />
            Assign to me
          </button>
        </div>
        <div class="row">
          <span class="label">Reviewers</span>
          <PeoplePicker
            v-model="reviewers"
            :people="people"
            :loading="forge.store.loadingMembers"
            :error="forge.store.membersError"
            placeholder="Ask someone to review"
          />
        </div>
      </div>

      <label class="check">
        <input v-model="draft" type="checkbox" />
        Open it as a draft
      </label>

      <p v-if="error" class="hint bad">{{ error }}</p>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn btn-ghost hand"
        :disabled="!source || !target || working"
        :title="`Open ${forgeName}'s own form with this already filled in`"
        @click="handOver"
      >
        Continue on {{ forgeName }}
        <ExternalLink :size="12" />
      </button>
      <button class="btn btn-ghost" :disabled="!ready || working" @click="submit(true)">
        Create and open
      </button>
      <button class="btn btn-primary" :disabled="!ready || working" @click="submit(false)">
        {{ working ? 'Working…' : 'Create' }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.ends {
  display: flex;
  align-items: flex-end;
  gap: 10px;
  margin-bottom: 12px;
}

.end {
  flex: 1;
  min-width: 0;
}

.label {
  display: block;
  margin-bottom: 4px;
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.field {
  margin-bottom: 12px;
}

.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.head .label {
  margin-bottom: 0;
}

.write,
.me {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px;
  font-size: 11.5px;
}

.write:not(:disabled) {
  color: var(--purple);
}

.field input,
.field textarea {
  width: 100%;
  margin-top: 6px;
  resize: vertical;
}

.who {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 12px;
}

.who .row {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}

.who .label {
  width: 74px;
  flex: none;
  margin-bottom: 0;
}

.hand {
  display: flex;
  align-items: center;
  gap: 5px;
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
