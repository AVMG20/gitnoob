<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check, ExternalLink, Pencil, Plus, Tag } from 'lucide-vue-next'
import PersonFace from './PersonFace.vue'
import PeoplePicker from './PeoplePicker.vue'
import Spinner from './Spinner.vue'
import { useReview } from '~/composables/useReview'
import type { Person } from '~/composables/useReview'
import { useForge, type Label, type Member } from '~/composables/useForge'
import { checkLook, verdictLook } from '~/composables/reviewLook'
import { relativeTime } from '~/composables/useGit'

/**
 * Everything about a review that is not the conversation: who is on it, what
 * it is labelled, how its checks went.
 *
 * All of it editable in place — the point of reading a review here rather than
 * in a browser tab is not having to open the browser tab to change anything.
 */
const review = useReview()
const forge = useForge()
const store = review.store

const detail = computed(() => store.detail)
const status = computed(() => store.status)

/** The verdict each person has standing, by login. */
const verdicts = computed(() => {
  const out: Record<string, ReturnType<typeof verdictLook> & { state: string }> = {}
  for (const verdict of status.value?.verdicts ?? []) {
    out[verdict.author.login] = { ...verdictLook(verdict.state), state: verdict.state }
  }
  return out
})

/**
 * Everyone who has said something about the review as a whole, whether or not
 * they were ever asked to: an approval from a passer-by still counts.
 */
const reviewers = computed<Person[]>(() => {
  const named = detail.value?.reviewers ?? []
  const out = [...named]
  for (const verdict of status.value?.verdicts ?? []) {
    if (!out.some((one) => one.login === verdict.author.login)) out.push(verdict.author)
  }
  return out
})

/** Who has already had their say, and so is not a pending request any more. */
const reviewed = computed(() =>
  reviewers.value.filter((one) => verdicts.value[one.login])
)

const checks = computed(() => checkLook(status.value?.checks_state ?? 'none'))
const checkTally = computed(() => {
  const all = status.value?.checks ?? []
  return {
    all: all.length,
    failed: all.filter((one) => one.state === 'failure').length,
    running: all.filter((one) => one.state === 'pending').length,
    passed: all.filter((one) => one.state === 'success').length
  }
})

function when(iso: string) {
  const at = Date.parse(iso)
  return Number.isNaN(at) ? '' : relativeTime(at / 1000)
}

// --- editing the people
//
// A picker wants ids as well as logins, which only the project's member list
// has; anybody it does not know is still offered back by login so saving does
// not quietly drop them.

const editingPeople = ref<'assignees' | 'reviewers' | null>(null)
const draftAssignees = ref<Member[]>([])
const draftReviewers = ref<Member[]>([])

function asMembers(people: Person[]): Member[] {
  return people.map((one) => {
    const known = forge.store.members.find((member) => member.login === one.login)
    return known ?? { id: 0, login: one.login, name: one.name || one.login }
  })
}

function editPeople(which: 'assignees' | 'reviewers') {
  forge.loadMembers()
  // Exactly who the forge has been asked to send it to: somebody who wandered
  // in and approved is not a request, and saving must not turn them into one.
  draftAssignees.value = asMembers(detail.value?.assignees ?? [])
  draftReviewers.value = asMembers(detail.value?.reviewers ?? [])
  editingPeople.value = which
}

async function savePeople() {
  const done = await review.setPeople(draftAssignees.value, draftReviewers.value)
  if (done) editingPeople.value = null
}

// --- editing the labels

const editingLabels = ref(false)
const known = ref<Label[]>([])
const draftLabels = ref<string[]>([])
const loadingLabels = ref(false)

async function editLabels() {
  draftLabels.value = (detail.value?.labels ?? []).map((one) => one.name)
  editingLabels.value = true
  if (known.value.length) return
  loadingLabels.value = true
  known.value = await review.projectLabels()
  loadingLabels.value = false
}

function toggleLabel(name: string) {
  draftLabels.value = draftLabels.value.includes(name)
    ? draftLabels.value.filter((one) => one !== name)
    : [...draftLabels.value, name]
}

async function saveLabels() {
  const done = await review.setLabels(draftLabels.value)
  if (done) editingLabels.value = false
}

// A different review is a different set of everything.
watch(
  () => store.current?.number,
  () => {
    editingPeople.value = null
    editingLabels.value = false
  }
)
</script>

<template>
  <aside v-if="detail" class="about" data-testid="review-sidebar">
    <!-- How it stands: the one section that changes while the page is open. -->
    <section class="card">
      <button class="fact checks" :class="checks.tone" @click="store.tab = 'checks'">
        <component :is="checks.icon" :size="14" />
        <span class="grow">
          <template v-if="!status || checkTally.all === 0">No checks ran</template>
          <template v-else-if="checkTally.failed">
            {{ checkTally.failed }} of {{ checkTally.all }} failed
          </template>
          <template v-else-if="checkTally.running">
            {{ checkTally.running }} still running
          </template>
          <template v-else>All {{ checkTally.all }} checks passed</template>
        </span>
        <Spinner v-if="store.loadingStatus" :size="11" />
      </button>

      <div v-if="status && status.approvals_required > 0" class="fact approvals">
        <Check :size="14" :class="status.approvals >= status.approvals_required ? 'good' : 'faint'" />
        <span>{{ status.approvals }} of {{ status.approvals_required }} approvals</span>
      </div>
    </section>

    <!-- Who. -->
    <section class="card">
      <div class="head">
        <h4>Reviewers</h4>
        <button class="edit" title="Ask somebody to look at this" @click="editPeople('reviewers')">
          <Pencil :size="11" />
        </button>
      </div>

      <template v-if="editingPeople === 'reviewers'">
        <ul v-if="reviewed.length" class="named done">
          <li v-for="one in reviewed" :key="one.login">
            <PersonFace
              :login="one.login"
              :name="one.name"
              :src="one.avatar"
              :size="18"
              :badge="(verdicts[one.login]?.state as never) ?? null"
            />
            <span class="name truncate">{{ one.name || one.login }}</span>
            <span class="faint small">{{ verdicts[one.login]?.label }}</span>
          </li>
        </ul>
        <p v-if="reviewed.length" class="none faint hint">
          Already read it — ask them again by adding them below.
        </p>

        <PeoplePicker
          v-model="draftReviewers"
          :people="forge.store.members"
          :loading="forge.store.loadingMembers"
          :error="forge.store.membersError"
          placeholder="Add a reviewer"
        />
        <div class="editing">
          <button class="btn btn-ghost tiny" @click="editingPeople = null">Cancel</button>
          <button
            class="btn btn-primary tiny"
            :disabled="store.acting !== null"
            data-testid="save-reviewers"
            @click="savePeople"
          >
            <Spinner v-if="store.acting === 'people'" :size="10" />
            Save
          </button>
        </div>
      </template>

      <ul v-else-if="reviewers.length" class="named">
        <li v-for="one in reviewers" :key="one.login">
          <PersonFace
            :login="one.login"
            :name="one.name"
            :src="one.avatar"
            :size="20"
            :badge="(verdicts[one.login]?.state as never) ?? null"
          />
          <span class="name truncate">{{ one.name || one.login }}</span>
          <span
            v-if="verdicts[one.login]"
            class="verdict"
            :class="verdicts[one.login]!.tone"
            :title="verdicts[one.login]!.label"
          >
            <component :is="verdicts[one.login]!.icon" :size="12" />
          </span>
          <span v-else class="faint waiting">waiting</span>
        </li>
      </ul>
      <p v-else class="none faint">Nobody has been asked yet.</p>
    </section>

    <section class="card">
      <div class="head">
        <h4>Assignees</h4>
        <button class="edit" title="Hand this to somebody" @click="editPeople('assignees')">
          <Pencil :size="11" />
        </button>
      </div>

      <template v-if="editingPeople === 'assignees'">
        <PeoplePicker
          v-model="draftAssignees"
          :people="forge.store.members"
          :loading="forge.store.loadingMembers"
          :error="forge.store.membersError"
          placeholder="Assign somebody"
        />
        <div class="editing">
          <button class="btn btn-ghost tiny" @click="editingPeople = null">Cancel</button>
          <button
            class="btn btn-primary tiny"
            :disabled="store.acting !== null"
            data-testid="save-assignees"
            @click="savePeople"
          >
            <Spinner v-if="store.acting === 'people'" :size="10" />
            Save
          </button>
        </div>
      </template>

      <ul v-else-if="detail.assignees.length" class="named">
        <li v-for="one in detail.assignees" :key="one.login">
          <PersonFace :login="one.login" :name="one.name" :src="one.avatar" :size="20" />
          <span class="name truncate">{{ one.name || one.login }}</span>
        </li>
      </ul>
      <p v-else class="none faint">Nobody owns this yet.</p>
    </section>

    <!-- What it is filed under. -->
    <section class="card">
      <div class="head">
        <h4>Labels</h4>
        <button class="edit" title="Change the labels" @click="editLabels">
          <Pencil :size="11" />
        </button>
      </div>

      <template v-if="editingLabels">
        <p v-if="loadingLabels" class="none faint">Reading the project's labels…</p>
        <div v-else-if="known.length" class="pick-labels">
          <button
            v-for="label in known"
            :key="label.name"
            class="label pick"
            :class="{ on: draftLabels.includes(label.name) }"
            :style="label.color ? { borderColor: label.color, color: label.color } : undefined"
            @click="toggleLabel(label.name)"
          >
            <Check v-if="draftLabels.includes(label.name)" :size="10" />
            <Plus v-else :size="10" />
            {{ label.name }}
          </button>
        </div>
        <p v-else class="none faint">This project has no labels.</p>
        <div class="editing">
          <button class="btn btn-ghost tiny" @click="editingLabels = false">Cancel</button>
          <button
            class="btn btn-primary tiny"
            data-testid="save-labels"
            :disabled="store.acting !== null"
            @click="saveLabels"
          >
            <Spinner v-if="store.acting === 'labels'" :size="10" />
            Save
          </button>
        </div>
      </template>

      <div v-else-if="detail.labels.length" class="labels">
        <span
          v-for="label in detail.labels"
          :key="label.name"
          class="label"
          :style="label.color ? { borderColor: label.color, color: label.color } : undefined"
        >
          <Tag :size="9" />
          {{ label.name }}
        </span>
      </div>
      <p v-else class="none faint">None.</p>
    </section>

    <!-- The rest, which is read once and rarely changed. -->
    <section class="card">
      <dl class="facts">
        <template v-if="detail.milestone">
          <dt>Milestone</dt>
          <dd>{{ detail.milestone }}</dd>
        </template>
        <dt>Comments</dt>
        <dd>{{ detail.comments }}</dd>
        <dt>Threads</dt>
        <dd>
          {{ review.openThreads.value }} open
          <span v-if="review.resolvedThreads.value" class="faint">
            · {{ review.resolvedThreads.value }} settled
          </span>
        </dd>
        <dt>Updated</dt>
        <dd :title="new Date(detail.updated_at).toLocaleString()">{{ when(detail.updated_at) }}</dd>
      </dl>
      <button class="forge-link" @click="forge.open(detail.url)">
        <ExternalLink :size="11" />
        View on {{ forge.store.status?.kind === 'gitlab' ? 'GitLab' : 'GitHub' }}
      </button>
    </section>
  </aside>
</template>

<style scoped>
.about {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.card {
  background: var(--bg-panel);
  border: 1px solid var(--line-soft);
  border-radius: 8px;
  padding: 10px 12px;
}

.head {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 7px;
}

h4 {
  margin: 0;
  flex: 1;
  font-size: 10.5px;
  font-weight: 700;
  letter-spacing: 0.07em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.edit {
  display: inline-flex;
  padding: 3px;
  border-radius: 4px;
  color: var(--text-faint);
}

.edit:hover {
  color: var(--text);
  background: var(--bg-hover);
}

/* The standing card has no title: its rows say what they are. */
.fact {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  font-size: 12px;
  color: var(--text-dim);
  text-align: left;
}

.fact + .fact {
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--line-soft);
}

.checks:hover {
  color: var(--text);
}

.checks.good {
  color: var(--green-soft);
}

.checks.bad {
  color: var(--red-soft);
}

.checks.wait {
  color: var(--amber-soft);
}

.grow {
  flex: 1;
}

.good {
  color: var(--green);
}

/* Named apart from the picker's own list: a scoped rule reaches the root of a
   child component too, and PeoplePicker's root is a `.people`. Sharing the
   name stood its chips on end and centred them. */
.named {
  display: flex;
  flex-direction: column;
  gap: 7px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.named.done {
  margin-bottom: 7px;
  opacity: 0.75;
}

.hint {
  margin: 0 0 7px;
  font-size: 10.5px;
}

.small {
  font-size: 10.5px;
}

.named li {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  font-size: 12px;
}

.name {
  flex: 1;
  min-width: 0;
  color: var(--text-dim);
}

.verdict.good {
  color: var(--green);
}

.verdict.bad {
  color: var(--red);
}

.verdict.none {
  color: var(--text-faint);
}

.waiting {
  font-size: 10.5px;
}

.labels,
.pick-labels {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.label {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 7px;
  border-radius: 999px;
  border: 1px solid var(--line);
  font-size: 10.5px;
  color: var(--text-dim);
}

.label.pick {
  opacity: 0.55;
  cursor: pointer;
}

.label.pick:hover,
.label.pick.on {
  opacity: 1;
  background: var(--bg-hover);
}

.editing {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
  margin-top: 8px;
}

.tiny {
  padding: 3px 9px;
  font-size: 11px;
}

.facts {
  display: grid;
  grid-template-columns: 68px 1fr;
  gap: 5px 10px;
  margin: 0;
  font-size: 11.5px;
  align-items: baseline;
}

.facts dt {
  color: var(--text-faint);
}

.facts dd {
  margin: 0;
  color: var(--text-dim);
  min-width: 0;
}

.none {
  margin: 0;
  font-size: 11.5px;
}

.forge-link {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  margin-top: 9px;
  font-size: 11px;
  color: var(--text-faint);
}

.forge-link:hover {
  color: var(--accent);
}
</style>
