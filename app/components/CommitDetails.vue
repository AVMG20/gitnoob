<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import {
  ArrowRight,
  Check,
  Copy,
  ExternalLink,
  FileText,
  GitCommitHorizontal,
  Hash,
  Pencil,
  Sparkles,
  TriangleAlert,
  X
} from 'lucide-vue-next'
import { copyText, fullTime, relativeTime, useGit } from '~/composables/useGit'
import type { RewordCheck } from '~/composables/useGit'
import { initials, tint } from '~/composables/useAvatars'
import { useContextMenu } from '~/composables/useContextMenu'
import { useFileView } from '~/composables/useFileView'
import { useForge } from '~/composables/useForge'
import { useAi } from '~/composables/useAi'

const git = useGit()
const store = git.store
const menu = useContextMenu()
const view = useFileView()
const forge = useForge()
const ai = useAi()

const openFile = computed(() =>
  store.viewer?.commit === store.detail?.oid ? store.viewer?.path : null
)

const detail = computed(() => store.detail)
const stats = computed(() => {
  const files = detail.value?.files ?? []
  return {
    files: files.length,
    additions: files.reduce((sum, f) => sum + f.additions, 0),
    deletions: files.reduce((sum, f) => sum + f.deletions, 0)
  }
})

/** Opens the file across the graph area rather than inline. */
function show(path: string) {
  if (!detail.value) return
  store.viewer =
    store.viewer?.path === path && store.viewer?.commit === detail.value.oid
      ? null
      : { path, commit: detail.value.oid }
}

// Moving to another commit closes whatever file was open from the last one,
// and abandons a message being edited: both belong to the commit they were
// opened from.
watch(
  () => store.detail?.oid,
  () => {
    if (store.viewer?.commit) store.viewer = null
    if (editing.value) cancel()
  }
)

// --- editing the message
//
// The mistake this is for is typing a message in haste, committing, and
// wanting it back a second later, so it goes no further than git's own amend:
// the newest commit, and only while it is the newest. Anything older would
// have to be replayed, which rewrites every commit above it — a different
// operation with different consequences, and not this one.

const editing = ref(false)
const draft = ref('')
const check = ref<RewordCheck | null>(null)
const saving = ref(false)
const editor = ref<HTMLTextAreaElement | null>(null)

/** The tip of the checked-out branch: the one commit a message can be changed on. */
const headOid = computed(() => store.refs?.locals.find((branch) => branch.is_head)?.oid ?? null)
/** Detached HEAD has no branch to move, so there is nothing to offer. */
const canEdit = computed(() => !!headOid.value && detail.value?.oid === headOid.value)

/** The first line is the subject, the way git reads it. */
const subject = computed(() => draft.value.split('\n')[0]?.trim() ?? '')
/** Git's own soft limit; past it `git log --oneline` starts truncating. */
const SUBJECT_LIMIT = 72

/**
 * Asks first, then opens.
 *
 * The button is only there for the newest commit, but the answer also carries
 * whether a remote already has it — which is the one thing worth saying before
 * the message is changed rather than after.
 */
async function edit() {
  const oid = detail.value?.oid
  if (!oid) return
  const answer = await git.rewordCheck(oid)
  if (!answer) return
  if (!answer.can) {
    git.note(answer.reason ?? 'That commit cannot be reworded', 'error')
    return
  }
  check.value = answer
  draft.value = answer.body ? `${answer.summary}\n\n${answer.body}` : answer.summary
  editing.value = true
  await nextTick()
  editor.value?.focus()
}

function cancel() {
  editing.value = false
  check.value = null
  draft.value = ''
}

/**
 * Normalises the typed text into a git commit message: git takes the first
 * line as the subject whatever follows it, but everything that shows a subject
 * and a body expects a blank line between them.
 */
function composed() {
  const lines = draft.value.split('\n')
  const head = (lines.shift() ?? '').trim()
  const rest = lines.join('\n').replace(/^\s*\n/, '').trimEnd()
  return rest ? `${head}\n\n${rest}` : head
}

/** Writes a message from the commit's own diff, for the user to read and edit. */
async function generate() {
  const oid = detail.value?.oid
  if (!oid || ai.store.busy) return
  try {
    const written = await ai.commitMessageFor(oid)
    if (!written) return
    draft.value = written.body ? `${written.summary}\n\n${written.body}` : written.summary
    git.note('Commit message written by the model — read it before saving')
  } catch (error) {
    git.note(`Commit message: ${String(error)}`, 'error')
  }
}

async function save() {
  const oid = detail.value?.oid
  if (!oid || !subject.value || saving.value) return
  saving.value = true
  // Amending writes a new object, so the commit comes back with a different
  // id. Following it keeps the panel on what the user was reading rather than
  // dropping them back at the working tree.
  const now = await git.reword(oid, composed())
  saving.value = false
  if (!now) return
  cancel()
  await git.select(now)
}

// --- the review this commit is the head of
//
// Clicking a merge request in the sidebar lands on the commit at its tip, so
// this panel is where somebody reading one ends up. What the forge knows and
// git does not — who it is assigned to, what it is labelled, whether it can be
// merged — belongs here rather than a browser tab away.

/** The review whose branch tip this commit is, if it is one. */
const review = computed(
  () => forge.store.reviews.find((one) => one.head_sha && one.head_sha === detail.value?.oid) ?? null
)
const reviewDetail = computed(() =>
  review.value ? (forge.store.details[review.value.number] ?? null) : null
)
const loadingReview = computed(() => forge.store.loadingDetail === review.value?.number)

// Asked for when a review's commit is opened, and not before: it is a request
// per review, and most commits are not the tip of one.
watch(
  review,
  (one) => {
    if (one) forge.loadReviewDetail(one.number)
  },
  { immediate: true }
)

/** `open`, `draft`, `merged`, `closed` — whichever the forge means. */
const reviewState = computed(() => {
  const one = reviewDetail.value ?? review.value
  if (!one) return ''
  if (one.draft) return 'draft'
  const state = one.state.toLowerCase()
  // GitLab says `opened`, GitHub says `open`; they mean the same thing.
  return state === 'opened' ? 'open' : state
})

/** Times arrive as ISO strings here rather than as git's seconds. */
function when(iso: string) {
  const at = Date.parse(iso)
  return Number.isNaN(at) ? '' : relativeTime(at / 1000)
}

function fullWhen(iso: string) {
  const at = Date.parse(iso)
  return Number.isNaN(at) ? '' : fullTime(at / 1000)
}

/**
 * A merge status worth showing.
 *
 * The forges answer this in their own words and most of them are noise —
 * GitHub's `clean`, GitLab's `mergeable` both amount to "nothing in the way".
 * The ones worth a line are the ones that say something is.
 */
const mergeNote = computed(() => {
  const status = reviewDetail.value?.merge_status
  if (!status) return null
  const quiet = ['clean', 'mergeable', 'can_be_merged', 'unstable', 'has_conflicts_resolved']
  if (quiet.includes(status)) return null
  return status.replace(/_/g, ' ')
})

function fileMenu(event: MouseEvent, path: string) {
  menu.show(
    event,
    [
      { label: 'Copy path', icon: Copy, action: () => copyText(path, 'Path') },
      { label: git.revealLabel, icon: FileText, action: () => git.reveal(path) }
    ],
    path
  )
}
</script>

<template>
  <div class="details">
    <p v-if="!detail" class="empty dim">Select a commit to see what it changed.</p>

    <template v-else>
      <div class="head">
        <div class="head-top">
          <button class="oid mono faint" title="Copy hash" @click="copyText(detail.oid, 'Hash')">
            <Hash :size="12" /> {{ detail.short }}
            <Copy :size="11" class="copy" />
          </button>
          <div class="controls" :class="{ live: editing }">
            <template v-if="editing">
              <button
                v-if="ai.configured.value"
                class="icon-btn ai"
                :disabled="saving || !!ai.store.busy"
                title="Write the message from this commit's diff"
                @click="generate"
              >
                <Spinner v-if="ai.store.busy" :size="13" />
                <Sparkles v-else :size="13" />
              </button>
              <button class="icon-btn" :disabled="saving" title="Leave it as it was" @click="cancel">
                <X :size="13" />
              </button>
              <button
                class="icon-btn save"
                :disabled="saving || !subject.length"
                title="Give the commit this message"
                @click="save"
              >
                <Spinner v-if="saving" :size="13" />
                <Check v-else :size="13" />
              </button>
            </template>
            <button
              v-else-if="canEdit"
              class="icon-btn"
              title="Change this commit's message"
              @click="edit"
            >
              <Pencil :size="13" />
            </button>
          </div>
        </div>

        <template v-if="editing">
          <div class="field">
            <textarea
              ref="editor"
              v-model="draft"
              rows="5"
              placeholder="Summary on the first line, why it changed below"
              @keydown.meta.enter="save"
              @keydown.ctrl.enter="save"
              @keydown.esc="cancel"
            />
            <span
              v-if="subject.length"
              class="counter"
              :class="{ over: subject.length > SUBJECT_LIMIT }"
              :title="`Subject length — git truncates past ${SUBJECT_LIMIT}`"
            >
              {{ subject.length }}
            </span>
          </div>
          <p v-if="check?.is_pushed" class="warn-line">
            <TriangleAlert :size="12" /> It is already on a remote — publishing this needs a force
            push.
          </p>
        </template>

        <template v-else>
          <h3>{{ detail.summary }}</h3>
          <pre v-if="detail.body" class="body">{{ detail.body }}</pre>
        </template>

        <div class="who">
          <Avatar :name="detail.author" :email="detail.email" :size="28" />
          <div>
            <div>
              <span class="dim">{{ detail.author }}</span>
              <span class="faint"> &lt;{{ detail.email }}&gt;</span>
            </div>
            <div class="faint">{{ fullTime(detail.time) }}</div>
            <div v-if="detail.committer !== detail.author" class="faint">
              committed by {{ detail.committer }}
            </div>
          </div>
        </div>

        <div class="parents">
          <GitCommitHorizontal :size="12" class="faint" />
          <button
            v-for="parent in detail.parents"
            :key="parent"
            class="parent mono"
            @click="git.select(parent)"
          >
            {{ parent.slice(0, 7) }}
          </button>
          <span v-if="!detail.parents.length" class="faint">root commit</span>
        </div>
      </div>

      <section v-if="review" class="review">
        <div class="review-top">
          <span class="number mono faint">
            {{ forge.shortLabel.value }} {{ forge.sigil.value }}{{ review.number }}
          </span>
          <span class="state" :class="reviewState">{{ reviewState }}</span>
          <span class="branches mono faint">
            {{ review.source_branch }} <ArrowRight :size="10" /> {{ review.target_branch }}
          </span>
          <button class="link" :title="`Open on ${forge.forgeName.value}`" @click="forge.open(review.url)">
            <ExternalLink :size="12" />
          </button>
        </div>

        <p v-if="loadingReview && !reviewDetail" class="pending faint">
          <Spinner :size="12" /> Reading the review…
        </p>
        <p v-else-if="!reviewDetail && forge.store.detailError" class="pending faint">
          {{ forge.store.detailError }}
        </p>

        <template v-if="reviewDetail">
          <!-- Only when it says something the commit does not: the two are the
               same sentence far more often than not. -->
          <h4 v-if="reviewDetail.title !== detail.summary">{{ reviewDetail.title }}</h4>
          <pre v-if="reviewDetail.body" class="body">{{ reviewDetail.body }}</pre>

          <div v-if="reviewDetail.labels.length" class="labels">
            <span
              v-for="label in reviewDetail.labels"
              :key="label.name"
              class="label"
              :style="
                label.color
                  ? { borderColor: label.color, color: label.color }
                  : undefined
              "
            >
              {{ label.name }}
            </span>
          </div>

          <dl class="facts">
            <dt>Opened by</dt>
            <dd>
              <span class="person">
                <span class="face" :style="{ background: reviewDetail.author.avatar ? 'transparent' : tint(reviewDetail.author.login) }">
                  <img v-if="reviewDetail.author.avatar" :src="reviewDetail.author.avatar" alt="" />
                  <template v-else>{{ initials(reviewDetail.author.name, reviewDetail.author.login) }}</template>
                </span>
                {{ reviewDetail.author.name }}
              </span>
              <span class="faint" :title="fullWhen(reviewDetail.created_at)">
                {{ when(reviewDetail.created_at) }}
              </span>
            </dd>

            <dt>Assigned to</dt>
            <dd v-if="reviewDetail.assignees.length">
              <span v-for="one in reviewDetail.assignees" :key="one.login" class="person">
                <span class="face" :style="{ background: one.avatar ? 'transparent' : tint(one.login) }">
                  <img v-if="one.avatar" :src="one.avatar" alt="" />
                  <template v-else>{{ initials(one.name, one.login) }}</template>
                </span>
                {{ one.name }}
              </span>
            </dd>
            <dd v-else class="faint">nobody</dd>

            <dt>Reviewers</dt>
            <dd v-if="reviewDetail.reviewers.length">
              <span v-for="one in reviewDetail.reviewers" :key="one.login" class="person">
                <span class="face" :style="{ background: one.avatar ? 'transparent' : tint(one.login) }">
                  <img v-if="one.avatar" :src="one.avatar" alt="" />
                  <template v-else>{{ initials(one.name, one.login) }}</template>
                </span>
                {{ one.name }}
              </span>
            </dd>
            <dd v-else class="faint">nobody yet</dd>

            <template v-if="reviewDetail.milestone">
              <dt>Milestone</dt>
              <dd>{{ reviewDetail.milestone }}</dd>
            </template>

            <dt>Comments</dt>
            <dd>
              {{ reviewDetail.comments }}
              <span class="faint">
                · updated <span :title="fullWhen(reviewDetail.updated_at)">{{ when(reviewDetail.updated_at) }}</span>
              </span>
            </dd>
          </dl>

          <p v-if="mergeNote" class="warn-line">
            <TriangleAlert :size="12" /> {{ mergeNote }}
          </p>
        </template>
      </section>

      <div class="files-head">
        <span>{{ stats.files }} {{ stats.files === 1 ? 'file' : 'files' }}</span>
        <span class="plus">+{{ stats.additions }}</span>
        <span class="minus">−{{ stats.deletions }}</span>
        <span class="toggle">
          <button
            class="seg"
            :class="{ on: view.state.mode === 'path' }"
            @click="view.state.mode = 'path'"
          >
            Path
          </button>
          <button
            class="seg"
            :class="{ on: view.state.mode === 'tree' }"
            @click="view.state.mode = 'tree'"
          >
            Tree
          </button>
        </span>
      </div>

      <FileList
        :files="
          detail.files.map((file) => ({
            path: file.path,
            kind: file.status,
            additions: file.additions,
            deletions: file.deletions
          }))
        "
        :selected="openFile"
        empty="No files in this commit."
        @select="show"
        @menu="(event, entry) => fileMenu(event, entry.path)"
      />
    </template>
  </div>
</template>

<style scoped>
.details {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}

.empty {
  padding: 18px 14px;
  font-size: 12px;
}

.head {
  padding: 12px 14px;
  border-bottom: 1px solid var(--line);
}

.head-top {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-bottom: 6px;
}

/* Faded until the pointer is in the panel: changing a message is an occasional
   thing and the button should not compete with the message the rest of the
   time. Icons rather than words for the same reason — the panel is narrow, and
   what each one does is a hover away. */
.controls {
  margin-left: auto;
  display: flex;
  gap: 2px;
  opacity: 0;
  transition: opacity 0.1s;
}

.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 22px;
  border-radius: 5px;
  color: var(--text-faint);
}

.icon-btn:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
}

.icon-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.icon-btn.ai:not(:disabled) {
  color: var(--purple);
}

.icon-btn.save:not(:disabled) {
  color: var(--accent);
}

.details:hover .controls,
.controls:focus-within,
.controls.live {
  opacity: 1;
}

.field {
  position: relative;
  margin-bottom: 7px;
}

.field textarea {
  width: 100%;
  display: block;
  /* Room for the counter in the corner. */
  padding-right: 38px;
}

.counter {
  position: absolute;
  top: 6px;
  right: 8px;
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
  color: var(--text-faint);
  pointer-events: none;
}

.counter.over {
  color: var(--amber);
}

.warn-line {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 0 7px;
  font-size: 11.5px;
  color: var(--amber);
}

.oid {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 6px;
  border-radius: 4px;
}

.oid:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.copy {
  opacity: 0;
}

.oid:hover .copy {
  opacity: 0.7;
}

h3 {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  line-height: 1.35;
}

.body {
  margin: 8px 0 0;
  font-family: var(--font);
  font-size: 12px;
  color: var(--text-dim);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 160px;
  overflow: auto;
}

.who {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  margin-top: 11px;
  font-size: 12px;
  line-height: 1.5;
}

.parents {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 9px;
}

.parent {
  font-size: 11px;
  color: var(--text-faint);
  padding: 1px 5px;
  border-radius: 4px;
}

.parent:hover {
  background: var(--bg-hover);
  color: var(--accent);
}

/* The forge's own account of the branch, between what the commit says and what
   it changed: the same reading order as the sidebar row that leads here. */
.review {
  padding: 10px 14px 12px;
  border-bottom: 1px solid var(--line);
  /* A shade off the panel, so the forge's account reads as a block of its own
     rather than as more of the commit above it. */
  background: var(--bg-deep);
}

.review-top {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 11px;
}

.number {
  font-size: 11px;
}

.state {
  padding: 1px 6px;
  border-radius: 9px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  border: 1px solid var(--line);
  color: var(--text-faint);
}

.state.open {
  color: var(--green);
  border-color: color-mix(in srgb, var(--green) 45%, transparent);
}

.state.draft {
  color: var(--amber);
  border-color: color-mix(in srgb, var(--amber) 45%, transparent);
}

.state.merged {
  color: var(--purple);
  border-color: color-mix(in srgb, var(--purple) 45%, transparent);
}

.state.closed {
  color: var(--red);
  border-color: color-mix(in srgb, var(--red) 45%, transparent);
}

/* The branch pair gives way first: it is the one thing on the row that is
   already spelled out on the sidebar row this panel was opened from. */
.branches {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.link {
  margin-left: auto;
  flex: none;
  color: var(--text-faint);
  padding: 2px;
  border-radius: 4px;
}

.link:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.pending {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px 0 0;
  font-size: 11.5px;
}

.review h4 {
  margin: 8px 0 0;
  font-size: 12.5px;
  font-weight: 600;
  line-height: 1.35;
}

.labels {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 8px;
}

.label {
  padding: 1px 6px;
  border-radius: 9px;
  border: 1px solid var(--line);
  font-size: 10.5px;
  color: var(--text-dim);
}

/* Two columns, the labels narrow and steady so the eye can run down them. */
.facts {
  display: grid;
  grid-template-columns: 74px 1fr;
  gap: 3px 10px;
  margin: 9px 0 0;
  font-size: 11.5px;
  align-items: baseline;
}

.facts dt {
  color: var(--text-faint);
}

.facts dd {
  margin: 0;
  color: var(--text-dim);
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 8px;
  min-width: 0;
}

.person {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
}

.face {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  overflow: hidden;
  color: #fff;
  font-size: 7.5px;
  font-weight: 600;
  line-height: 1;
  user-select: none;
}

.face img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.review .body {
  max-height: 120px;
}

.review .warn-line {
  margin: 9px 0 0;
  text-transform: capitalize;
}

.files-head {
  display: flex;
  gap: 10px;
  padding: 6px 14px;
  font-size: 11px;
  color: var(--text-faint);
  border-bottom: 1px solid var(--line-soft);
}

.toggle {
  margin-left: auto;
  display: flex;
  border: 1px solid var(--line);
  border-radius: 5px;
  overflow: hidden;
}

.seg {
  padding: 1px 7px;
  font-size: 10.5px;
  color: var(--text-faint);
}

.seg:hover {
  color: var(--text);
}

.seg.on {
  background: var(--bg-active);
  color: var(--text);
}

.plus {
  color: var(--green);
  font-size: 11px;
}

.minus {
  color: var(--red);
  font-size: 11px;
}

</style>
