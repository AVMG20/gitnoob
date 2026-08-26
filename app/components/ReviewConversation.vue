<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Check, GitMerge, MessageSquare, Pencil, X } from 'lucide-vue-next'
import PersonFace from './PersonFace.vue'
import CommentBox from './CommentBox.vue'
import ReviewSidebar from './ReviewSidebar.vue'
import ReviewThread from './ReviewThread.vue'
import Spinner from './Spinner.vue'
import { useReview } from '~/composables/useReview'
import type { RComment, ReviewVerdict, Thread } from '~/composables/useReview'
import { useForge } from '~/composables/useForge'
import { mergeSummary, verdictLook } from '~/composables/reviewLook'
import { renderMarkdown } from '~/composables/useMd'
import { quotedInto } from '~/composables/reviewThreads'
import { relativeTime } from '~/composables/useGit'

/**
 * The conversation: the description, everything said about it in the order it
 * was said, and the state the review has ended up in.
 *
 * Diff threads are read here too rather than only on the files page — what was
 * asked for on line 40 is part of the conversation, and burying it behind a
 * tab is how a review ends up answered in a browser tab instead.
 */
const props = defineProps<{ editing: boolean }>()
const emit = defineEmits<{ 'update:editing': [boolean]; merge: [] }>()

const review = useReview()
const forge = useForge()
const store = review.store

const detail = computed(() => store.detail)

/** Everything said about the review, in the order it was said. */
type Entry =
  | { kind: 'thread'; at: number; key: string; thread: Thread }
  | { kind: 'verdict'; at: number; key: string; verdict: ReviewVerdict }

const timeline = computed<Entry[]>(() => {
  const time = (iso: string) => {
    const at = Date.parse(iso)
    return Number.isNaN(at) ? 0 : at
  }
  // The diff threads come from the same folding the files page reads, so a
  // thread answered there is the same thread here, replies and all.
  const all: Entry[] = [
    ...review.talkThreads().map((thread) => ({
      kind: 'thread' as const,
      at: time(thread.root.created_at),
      key: `t${thread.key}`,
      thread
    })),
    ...review.diffThreads.value.map((thread) => ({
      kind: 'thread' as const,
      at: time(thread.root.created_at),
      key: `d${thread.key}`,
      thread
    })),
    // A verdict with nothing said is already drawn in the sidebar as a tick;
    // repeating it here would be a timeline of empty cards.
    ...(store.status?.verdicts ?? [])
      .filter((verdict) => verdict.body.trim() || verdict.state !== 'commented')
      .map((verdict) => ({
        kind: 'verdict' as const,
        at: time(verdict.submitted_at),
        key: `v${verdict.author.login}${verdict.submitted_at}`,
        verdict
      }))
  ]
  // GitLab's approvals arrive without a time on them. Undated goes last rather
  // than at the epoch, where it would sit above the description it answers.
  return all.sort((a, b) => (a.at || Infinity) - (b.at || Infinity))
})

const state = computed(() => {
  const one = store.detail ?? store.current
  if (!one) return ''
  if (one.draft) return 'draft'
  const raw = one.state.toLowerCase()
  return raw === 'opened' ? 'open' : raw
})

const standing = computed(() =>
  mergeSummary(store.status, state.value, store.detail?.draft ?? false)
)

const settled = computed(() => state.value === 'merged' || state.value === 'closed')

function when(iso: string) {
  const at = Date.parse(iso)
  return Number.isNaN(at) ? '' : relativeTime(at / 1000)
}

function fullWhen(iso: string) {
  const at = Date.parse(iso)
  return Number.isNaN(at) ? '' : new Date(at).toLocaleString()
}

/** The store keeps the open reply box as a number; this is its toggle. */
function toggleReply(id: number) {
  store.replyingTo = store.replyingTo === id ? null : id
}

/** A thread's location chip: take the reader to the file it stands on. */
function locate(thread: Thread) {
  if (!thread.root.path) return
  store.tab = 'files'
  store.selectedPath = thread.root.path
}

/**
 * A remark quoted into the composer, which is how a conversation comment is
 * answered: neither forge threads those, so the answer is a new comment that
 * carries what it is answering.
 */
const composer = ref<InstanceType<typeof CommentBox> | null>(null)

function quote(comment: RComment) {
  store.drafts.talk = quotedInto(store.drafts.talk, comment.body)
  review.saveDrafts()
  void nextTick(() => composer.value?.focus())
}


// --- editing what the review says about itself

const title = ref('')
const body = ref('')

watch(
  () => props.editing,
  (now) => {
    if (!now) return
    title.value = detail.value?.title ?? store.current?.title ?? ''
    body.value = detail.value?.body ?? ''
  },
  { immediate: true }
)

async function save() {
  const done = await review.updateReview(title.value, body.value)
  if (done) emit('update:editing', false)
}
</script>

<template>
  <div class="conversation" data-testid="conversation">
    <div class="talk">
      <!-- What was asked for. -->
      <section v-if="detail" class="card description">
        <div class="meta">
          <PersonFace
            :login="detail.author.login"
            :name="detail.author.name"
            :src="detail.author.avatar"
            :size="22"
          />
          <span class="author">{{ detail.author.name || detail.author.login }}</span>
          <span class="faint" :title="fullWhen(detail.created_at)">
            opened this {{ when(detail.created_at) }}
          </span>
          <span class="grow" />
          <button
            v-if="!props.editing"
            class="quiet"
            data-testid="edit-description"
            title="Edit the title and description"
            @click="emit('update:editing', true)"
          >
            <Pencil :size="12" />
          </button>
        </div>

        <template v-if="props.editing">
          <input v-model="title" class="title-field" type="text" placeholder="Title" />
          <textarea v-model="body" rows="8" class="body-field" placeholder="What this changes, and why." />
          <div class="editing">
            <button class="btn btn-ghost" @click="emit('update:editing', false)">Cancel</button>
            <button
              class="btn btn-primary"
              data-testid="save-description"
              :disabled="!title.trim() || store.acting !== null"
              @click="save"
            >
              <Spinner v-if="store.acting === 'edit'" :size="11" />
              Save
            </button>
          </div>
        </template>

        <template v-else>
          <div v-if="detail.body" class="md-body" v-html="renderMarkdown(detail.body)" />
          <p v-else class="faint no-body">No description.</p>
        </template>
      </section>

      <!-- Everything said since, in the order it was said. -->
      <template v-for="entry in timeline" :key="entry.key">
        <section v-if="entry.kind === 'thread'" class="card conversation-item">
          <ReviewThread
            :thread="entry.thread"
            :busy="store.sending"
            :reply-to="store.replyingTo"
            show-location
            @reply="review.reply"
            @toggle-reply="toggleReply"
            @locate="locate"
            @resolve="review.resolveThread"
            @quote="quote"
          />
        </section>

        <!-- A verdict is an event, not a card: one line with a face on it. -->
        <div
          v-else
          class="event"
          :class="verdictLook(entry.verdict.state).tone"
          data-testid="verdict-event"
        >
          <span class="glyph">
            <component :is="verdictLook(entry.verdict.state).icon" :size="12" />
          </span>
          <PersonFace
            :login="entry.verdict.author.login"
            :name="entry.verdict.author.name"
            :src="entry.verdict.author.avatar"
            :size="18"
          />
          <span class="what">
            <strong>{{ entry.verdict.author.name || entry.verdict.author.login }}</strong>
            {{ verdictLook(entry.verdict.state).label }}
            <span v-if="entry.verdict.submitted_at" class="faint">
              {{ when(entry.verdict.submitted_at) }}
            </span>
            <span v-if="entry.verdict.body.trim()" class="said">{{ entry.verdict.body }}</span>
          </span>
        </div>
      </template>

      <p v-if="!timeline.length" class="empty faint">Nothing has been said yet.</p>

      <!-- Where it has ended up, and the one step left. -->
      <section class="card standing" :class="standing.tone" data-testid="merge-box">
        <div class="verdict-line">
          <span class="glyph">
            <GitMerge v-if="!settled" :size="15" />
            <Check v-else-if="state === 'merged'" :size="15" />
            <X v-else :size="15" />
          </span>
          <div class="words">
            <strong>{{ standing.title }}</strong>
            <span v-if="standing.detail" class="faint">{{ standing.detail }}</span>
          </div>
          <button
            v-if="!settled && !store.detail?.draft"
            class="btn btn-primary go"
            data-testid="merge-from-conversation"
            :disabled="store.acting !== null"
            @click="emit('merge')"
          >
            <Spinner v-if="store.acting === 'merge'" :size="12" />
            <GitMerge v-else :size="13" />
            Merge
          </button>
          <button
            v-else-if="store.detail?.draft"
            class="btn btn-primary go"
            :disabled="store.acting !== null"
            @click="review.setDraft(false)"
          >
            <Spinner v-if="store.acting === 'ready'" :size="12" />
            Mark ready
          </button>
        </div>

        <p v-if="review.openThreads.value && !settled" class="left faint">
          <MessageSquare :size="11" />
          {{ review.openThreads.value }} thread{{ review.openThreads.value === 1 ? '' : 's' }}
          on the diff still open
        </p>
      </section>

      <!-- The last word, which is usually the reader's. -->
      <section class="card composer">
        <CommentBox
          ref="composer"
          v-model="store.drafts.talk"
          :busy="store.sending"
          :cancellable="false"
          placeholder="Leave a comment"
          @update:model-value="review.saveDrafts()"
          @send="review.post"
        />
      </section>
    </div>

    <ReviewSidebar />
  </div>
</template>

<style scoped>
/* The reading column and the facts beside it, centred as a pair: a review
   read on a wide monitor should not be a column of text against one edge and
   a field of empty panel against the other. */
.conversation {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 264px;
  gap: 18px;
  align-items: start;
  width: 100%;
  max-width: 1180px;
  margin: 0 auto;
  padding: 16px 22px 64px;
}

@media (max-width: 940px) {
  .conversation {
    grid-template-columns: minmax(0, 1fr);
  }
}

.talk {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 9px;
}

.card {
  background: var(--bg-panel);
  border: 1px solid var(--line-soft);
  border-radius: 8px;
  padding: 12px 14px;
}

.meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  min-width: 0;
}

.author {
  color: var(--text);
  font-weight: 600;
}

.grow {
  flex: 1;
}

.quiet {
  padding: 3px;
  border-radius: 4px;
  color: var(--text-faint);
}

.quiet:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.no-body {
  margin: 8px 0 0;
  font-size: 12px;
}

.title-field {
  width: 100%;
  margin-top: 10px;
  font-family: var(--font);
  font-size: 13px;
  font-weight: 600;
}

.body-field {
  width: 100%;
  margin-top: 7px;
  font-family: var(--font);
  font-size: 12px;
  line-height: 1.55;
  resize: vertical;
}

.editing {
  display: flex;
  justify-content: flex-end;
  gap: 7px;
  margin-top: 9px;
}

/* An event is quieter than a card: it happened, it is not being discussed. */
.event {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  padding: 2px 4px 2px 0;
  font-size: 12px;
  color: var(--text-dim);
}

.event .glyph {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: var(--bg-raised);
  color: var(--text-faint);
  flex: none;
}

.event.good .glyph {
  color: var(--green);
}

.event.bad .glyph {
  color: var(--red);
}

.event .what {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 5px;
  min-width: 0;
  padding-top: 3px;
}

.event strong {
  color: var(--text);
}

.event .said {
  flex-basis: 100%;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-dim);
}

/* Where the review has ended up: the one card that is about doing rather than
   reading, so it is the one card with a filled button on it. */
.standing {
  border-left: 3px solid var(--line);
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

.verdict-line {
  display: flex;
  align-items: center;
  gap: 11px;
  min-width: 0;
}

.verdict-line .glyph {
  flex: none;
  color: var(--text-faint);
}

.standing.good .verdict-line .glyph {
  color: var(--green);
}

.standing.bad .verdict-line .glyph {
  color: var(--red);
}

.standing.wait .verdict-line .glyph {
  color: var(--amber);
}

.words {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 12.5px;
}

.words .faint {
  font-size: 11.5px;
}

.go {
  flex: none;
  padding: 5px 14px;
  font-size: 12.5px;
}

.left {
  display: flex;
  align-items: center;
  gap: 5px;
  margin: 9px 0 0;
  padding-top: 9px;
  border-top: 1px solid var(--line-soft);
  font-size: 11.5px;
}

.empty {
  margin: 0;
  padding: 6px 2px;
  font-size: 12px;
}

/* The same markdown dressing the threads wear, for the description. */
.md-body {
  margin-top: 9px;
  font-size: 12.5px;
  color: var(--text-dim);
  min-width: 0;
}

.md-body :deep(p) {
  margin: 6px 0;
  white-space: pre-wrap;
  word-break: break-word;
}

.md-body :deep(code) {
  font-family: var(--mono);
  background: var(--bg-raised);
  padding: 1px 4px;
  border-radius: 3px;
  font-size: 11px;
}

.md-body :deep(pre) {
  background: var(--bg-deep);
  border: 1px solid var(--line-soft);
  border-radius: 5px;
  padding: 8px;
  overflow: auto;
}

.md-body :deep(pre code) {
  background: none;
  padding: 0;
}

.md-body :deep(blockquote) {
  border-left: 2px solid var(--line);
  margin: 6px 0;
  padding: 2px 10px;
  color: var(--text-dim);
}

.md-body :deep(.mention),
.md-body :deep(.ref) {
  color: var(--accent-soft);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  border-radius: 3px;
  padding: 0 3px;
}

.md-body :deep(a) {
  color: var(--accent);
}
</style>
