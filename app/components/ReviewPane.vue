<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { Check, ChevronDown, ChevronUp, Copy, Eye, EyeOff, MessageSquare, X } from 'lucide-vue-next'
import AppModal from './AppModal.vue'
import PersonFace from './PersonFace.vue'
import Spinner from './Spinner.vue'
import ReviewChecks from './ReviewChecks.vue'
import ReviewConversation from './ReviewConversation.vue'
import ReviewDiff from './ReviewDiff.vue'
import ReviewHeader from './ReviewHeader.vue'
import ReviewMergeDialog from './ReviewMergeDialog.vue'
import { useReview } from '~/composables/useReview'
import { useForge } from '~/composables/useForge'
import { verdictLook } from '~/composables/reviewLook'
import { copyText, relativeTime, useGit } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { initials, tint } from '~/composables/useAvatars'

/**
 * A review, read and answered without leaving the app.
 *
 * The pane is four pages over one header: what was said, what changed, what
 * it took to get there, and what ran against it. Everything that acts on the
 * review — merging, verdicts, assigning, labelling — happens from here.
 */
const review = useReview()
const forge = useForge()
const git = useGit()
const menu = useContextMenu()
const store = review.store

const merging = ref(false)
const finishing = ref(false)
const editing = ref(false)
const summary = ref('')

// --- finishing the review

/**
 * How much has been said here by the person reading.
 *
 * The forge knows who the token belongs to, and the remarks carry their
 * authors, so the count is a filter rather than a second opinion.
 */
const mine = computed(() => {
  const me = forge.store.me?.login
  if (!me) return []
  return store.comments.filter((comment) => comment.author.login === me)
})

/** The verdict the reader already has standing, when they have one. */
const standing = computed(() => review.myVerdict.value)

/** Hands the verdict over and puts the modal away, whatever it was. */
async function finish(event: 'approve' | 'request_changes' | 'comment') {
  await review.verdict(event, summary.value)
  summary.value = ''
  finishing.value = false
}

// --- reading

/** Times arrive as ISO strings here rather than as git's seconds. */
function when(iso: string) {
  const at = Date.parse(iso)
  return Number.isNaN(at) ? '' : relativeTime(at / 1000)
}

/** Whether the page on screen has nothing to show yet because it is loading. */
const reading = computed(() => {
  if (store.tab === 'files') return store.loadingFiles && !store.files.length
  if (store.tab === 'commits') return store.loadingCommits && !store.commits.length
  if (store.tab === 'checks') return store.loadingStatus && !store.status
  return (store.loadingDetail && !store.detail) || (store.loadingComments && !store.comments.length)
})

// --- keyboard

/** True when the keystroke belongs to whatever is being written in. */
function typing(event: KeyboardEvent) {
  const element = event.target as HTMLElement | null
  if (!element) return false
  return element.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(element.tagName)
}

/** True while something sits on top of the pane. */
function covered() {
  return !!document.querySelector('.scrim, .overlay')
}

function onKey(event: KeyboardEvent) {
  if (typing(event) || covered()) return
  if (event.key === 'Escape') {
    // A remark being written is the smaller thing Esc closes first.
    if (store.draft) review.cancelDraft()
    else if (store.replyingTo !== null) store.replyingTo = null
    else if (editing.value) editing.value = false
    else review.close()
    return
  }
  // Walking the review's files, without leaving the keyboard: the arrows step
  // through the list the way they step through any list, and the chord reads
  // this one and jumps to the next that has not been read.
  if (store.tab === 'files') {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
      event.preventDefault()
      viewedNext()
      return
    }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      step(event.key === 'ArrowDown' ? 1 : -1)
      return
    }
  }
  // Alt with the sideways arrows does the same, and keeps working while the
  // diff itself has the focus and is being scrolled.
  if (event.altKey && (event.key === 'ArrowRight' || event.key === 'ArrowLeft')) {
    event.preventDefault()
    step(event.key === 'ArrowRight' ? 1 : -1)
  }
}

onMounted(() => window.addEventListener('keydown', onKey))
onUnmounted(() => window.removeEventListener('keydown', onKey))

// --- the files page
//
// One file at a time, opened from the panel like the graph opens one: the
// list is the panel's to draw, and the page is the file being read.

const pane = ref<HTMLElement | null>(null)

const openFile = computed(() =>
  store.tab === 'files'
    ? (store.files.find((file) => file.path === store.selectedPath) ?? null)
    : null
)

/** Where the open file sits in the review, for the strip that walks them. */
const fileAt = computed(() =>
  openFile.value ? store.files.findIndex((file) => file.path === openFile.value!.path) : -1
)

/** Steps through the review's files, stopping at the ends rather than wrapping. */
function step(delta: number) {
  const at = fileAt.value + delta
  const file = store.files[at]
  if (file) store.selectedPath = file.path
}

/** The file bar's menu, which is the file list's menu for the open file. */
function fileMenu(event: MouseEvent) {
  const here = openFile.value
  if (!here) return
  const read = store.viewed.has(here.path)
  menu.show(
    event,
    [
      {
        label: read ? 'Mark as not viewed' : 'Mark as viewed',
        icon: read ? EyeOff : Eye,
        action: () => review.toggleViewed(here.path)
      },
      { label: 'Copy path', icon: Copy, action: () => void copyText(here.path, 'Path') },
      ...(here.old_path
        ? [
            {
              label: 'Copy the path it moved from',
              icon: Copy,
              action: () => void copyText(here.old_path!, 'Path')
            }
          ]
        : []),
      { label: git.revealLabel, action: () => void git.reveal(here.path) }
    ],
    here.path
  )
}

/**
 * Ticks the open file read and moves on — past what has been said about the
 * rest of the review's files, so a pass through them costs one click each.
 */
function viewedNext() {
  const here = openFile.value
  if (!here) return
  if (!store.viewed.has(here.path)) review.toggleViewed(here.path)
  const fresh = store.files.findIndex(
    (file) => !store.viewed.has(file.path) && file.path !== here.path
  )
  if (fresh >= 0) store.selectedPath = store.files[fresh]!.path
}

// Arriving at the files page with nothing chosen reads the first file, so the
// page never opens empty when there is something to show.
watch(
  () => store.tab,
  () => {
    if (store.tab === 'files' && !store.selectedPath && store.files.length) {
      store.selectedPath = store.files[0]!.path
    }
  }
)

// The panel chooses; the page follows. `nextTick` because choosing can mount
// the files page itself, the draft box having switched to it.
watch(
  () => store.selectedPath,
  async () => {
    await nextTick()
    pane.value?.scrollTo?.({ top: 0 })
  }
)
</script>

<template>
  <section v-if="store.current" class="review-pane" data-review-open>
    <ReviewHeader
      @merge="merging = true"
      @finish="finishing = true"
      @edit="
        () => {
          store.tab = 'conversation'
          editing = true
        }
      "
    />

    <div ref="pane" class="page">
      <div v-if="reading" class="reading faint">
        <Spinner :size="14" />
        Reading the review…
      </div>

      <template v-else>
        <p v-if="store.detailError" class="warn">{{ store.detailError }}</p>
        <p v-if="store.commentsError" class="warn">{{ store.commentsError }}</p>

        <ReviewConversation
          v-if="store.tab === 'conversation'"
          v-model:editing="editing"
          @merge="merging = true"
        />

        <div v-else-if="store.tab === 'files'" class="files-page">
          <!-- One bar over the file: where you are in the review, how to read
               it, and how to move on. The path and its counts are the panel's
               to say, and saying them twice is what made this two bars. -->
          <div v-if="store.files.length" class="filebar" @contextmenu="fileMenu">
            <button
              class="step"
              title="The file above (↑)"
              :disabled="fileAt <= 0"
              data-testid="file-prev"
              @click="step(-1)"
            >
              <ChevronUp :size="14" />
            </button>
            <button
              class="step"
              title="The file below (↓)"
              :disabled="fileAt < 0 || fileAt >= store.files.length - 1"
              data-testid="file-next"
              @click="step(1)"
            >
              <ChevronDown :size="14" />
            </button>
            <span class="pos faint">
              {{ fileAt + 1 }} of {{ store.files.length }} ·
              {{ review.viewedCount.value }} read
            </span>

            <span class="grow" />


            <label v-if="openFile" class="viewed" title="Mark this file read">
              <input
                type="checkbox"
                data-testid="viewed-tick"
                :checked="store.viewed.has(openFile.path)"
                @change="review.toggleViewed(openFile.path)"
              />
              Viewed
            </label>

            <button
              class="next"
              data-testid="viewed-next"
              title="Mark this file read and open the next one that is not (Ctrl+Enter)"
              :disabled="!openFile"
              @click="viewedNext"
            >
              <Eye :size="12" />
              Read, next
            </button>
          </div>

          <!-- One file at a time, chosen from the panel: the same shape as
               the graph opening one file across itself. -->
          <ReviewDiff v-if="openFile" :key="openFile.path" :file="openFile" />
          <p v-else class="none faint">Pick a file from the list to read it.</p>

          <!-- The end of a file is where the next one is wanted, so the step
               is drawn there rather than left as a scroll back to the bar. -->
          <div v-if="openFile" class="fileend">
            <span class="faint">
              End of <span class="mono">{{ openFile.path }}</span>
            </span>
            <button class="next" data-testid="viewed-next-end" @click="viewedNext">
              <Eye :size="12" />
              {{ store.viewed.has(openFile.path) ? 'Next unread file' : 'Read, next file' }}
              <kbd>Ctrl↵</kbd>
            </button>
          </div>
        </div>

        <ReviewChecks v-else-if="store.tab === 'checks'" />

        <div v-else class="commits-page">
          <div
            v-for="commit in store.commits"
            :key="commit.sha"
            class="commit-row"
            data-testid="commit-row"
            :title="commit.message"
          >
            <span class="face" :style="{ background: tint(commit.author) }">
              {{ initials(commit.author, commit.author) }}
            </span>
            <div class="what">
              <span class="message truncate">{{ commit.message.split('\n')[0] }}</span>
              <span class="faint by">
                {{ commit.author }} ·
                <span :title="new Date(Date.parse(commit.created_at)).toLocaleString()">
                  {{ when(commit.created_at) }}
                </span>
              </span>
            </div>
            <button class="sha mono" title="Copy hash" @click="copyText(commit.sha, 'Hash')">
              {{ commit.sha.slice(0, 7) }}
            </button>
          </div>
          <p v-if="!store.commits.length" class="none faint">No commits yet.</p>
        </div>
      </template>
    </div>

    <ReviewMergeDialog v-if="merging" @close="merging = false" />

    <!-- Finishing: what was said, and the verdict it adds up to. -->
    <AppModal v-if="finishing" title="Finish review" :width="540" @close="finishing = false">
      <p v-if="store.pending.length" class="tally waiting" data-testid="pending-tally">
        <MessageSquare :size="14" class="glyph" />
        <strong>
          {{ store.pending.length }}
          {{ store.pending.length === 1 ? 'remark waiting' : 'remarks waiting' }}
        </strong>
        on the diff — the verdict is what sends them
      </p>

      <p class="tally">
        <MessageSquare :size="14" class="glyph" />
        You have already sent
        <strong>{{ mine.length }} {{ mine.length === 1 ? 'remark' : 'remarks' }}</strong>
        on this review
        <span v-if="mine.length" class="faint">
          · the newest {{ when(mine[mine.length - 1]!.created_at) }}
        </span>
      </p>

      <p v-if="standing" class="already" :class="verdictLook(standing.state).tone">
        <PersonFace
          :login="standing.author.login"
          :name="standing.author.name"
          :src="standing.author.avatar"
          :size="18"
        />
        You already {{ verdictLook(standing.state).label }} this review. Sending another
        replaces it.
      </p>

      <p v-if="review.openThreads.value" class="open-left faint">
        {{ review.openThreads.value }} thread{{ review.openThreads.value === 1 ? '' : 's' }}
        on the diff {{ review.openThreads.value === 1 ? 'is' : 'are' }} still open.
      </p>

      <textarea
        v-model="summary"
        rows="4"
        class="summary"
        placeholder="Anything to say alongside the verdict (optional)"
      />

      <template #footer>
        <button class="btn" @click="finishing = false">Close</button>
        <button
          class="btn btn-ghost verdict"
          data-testid="finish-comment"
          :disabled="store.acting !== null || (!summary.trim() && !store.pending.length)"
          :title="
            !summary.trim() && !store.pending.length
              ? 'Write something, or hold a remark back on the diff first'
              : 'Send the remarks and the note without approving or refusing'
          "
          @click="finish('comment')"
        >
          <Spinner v-if="store.acting === 'comment'" :size="12" />
          Send without a verdict
        </button>
        <button
          class="btn btn-ghost verdict request"
          data-testid="finish-request"
          :disabled="store.acting !== null"
          @click="finish('request_changes')"
        >
          <Spinner v-if="store.acting === 'request_changes'" :size="12" />
          <X v-else :size="13" />
          Request changes
        </button>
        <button
          class="btn btn-ghost verdict approve"
          data-testid="finish-approve"
          :disabled="store.acting !== null"
          @click="finish('approve')"
        >
          <Spinner v-if="store.acting === 'approve'" :size="12" />
          <Check v-else :size="13" />
          Approve
        </button>
      </template>
    </AppModal>
  </section>
</template>

<style scoped>
.review-pane {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  min-height: 0;
  background: var(--bg);
}

.page {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: auto;
}

.reading {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 48px 0;
  font-size: 12px;
}

.warn {
  margin: 10px 16px 0;
  padding: 6px 10px;
  border-left: 2px solid var(--amber);
  color: var(--amber);
  font-size: 11.5px;
}

/* One file, filling the page it is read in. */
.files-page {
  display: flex;
  flex-direction: column;
  min-height: 100%;
}

/* What is under a file once it has been read: the next one. Grows into
   whatever the diff left over, so a short file does not end in a hole. */
.fileend {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: flex-start;
  gap: 10px;
  padding: 22px 16px 40px;
  border-top: 1px solid var(--line-soft);
  font-size: 11.5px;
}

.fileend .mono {
  font-size: 11px;
  color: var(--text-dim);
}

.fileend .next {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 4px 12px;
  border: 1px solid var(--line);
  border-radius: 999px;
  font-size: 11.5px;
  color: var(--text-dim);
}

.fileend .next:hover {
  color: var(--green-soft);
  border-color: color-mix(in srgb, var(--green) 45%, transparent);
}

kbd {
  padding: 0 4px;
  border-radius: 3px;
  background: var(--bg-raised);
  border: 1px solid var(--line-soft);
  font-family: var(--mono);
  font-size: 9.5px;
  color: var(--text-faint);
}

/* The one bar over a file: where you are, how to read it, how to move on. It
   sticks to the top of the page, since a file worth scrolling is a file whose
   controls should still be there at the bottom of it. */
.filebar {
  position: sticky;
  top: 0;
  z-index: 3;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
  font-size: 11px;
}

.filebar .viewed {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--text-faint);
  cursor: pointer;
  user-select: none;
}

.filebar .viewed:hover {
  color: var(--text-dim);
}

.filebar .viewed input {
  accent-color: var(--accent);
  margin: 0;
}

.filebar .step {
  display: inline-flex;
  padding: 2px 4px;
  border-radius: 4px;
  color: var(--text-faint);
}

.filebar .step:hover:not(:disabled) {
  color: var(--text);
  background: var(--bg-hover);
}

.filebar .step:disabled {
  opacity: 0.35;
}

.filebar .pos {
  margin-left: 6px;
  font-variant-numeric: tabular-nums;
}

.filebar .grow {
  flex: 1;
}

.filebar .next {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 10px;
  border: 1px solid var(--line);
  border-radius: 999px;
  font-size: 11px;
  color: var(--text-dim);
}

.filebar .next:hover:not(:disabled) {
  color: var(--green-soft);
  border-color: color-mix(in srgb, var(--green) 45%, transparent);
}

.commits-page {
  display: flex;
  flex-direction: column;
  width: 100%;
  max-width: 900px;
  margin: 0 auto;
  padding: 12px 0 40px;
}

/* A commit is a face, what it says, and where to find it again: the hash
   moves to the far end where a lookup belongs, not first where a title
   belongs. */
.commit-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 16px;
  border-bottom: 1px solid var(--line-soft);
  font-size: 12px;
}

.commit-row:hover {
  background: var(--bg-hover);
}

.commit-row .face {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  color: #fff;
  font-size: 8.5px;
  font-weight: 600;
  line-height: 1;
  user-select: none;
}

.commit-row .what {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.message {
  min-width: 0;
  color: var(--text);
}

.by {
  font-size: 11px;
}

.sha {
  flex: none;
  padding: 1px 6px;
  border-radius: 4px;
  color: var(--accent-soft);
}

.sha:hover {
  background: var(--bg-hover);
  color: var(--accent);
}

.none {
  margin: 0;
  padding: 12px 16px;
  font-size: 12px;
}

.tally {
  display: flex;
  align-items: center;
  gap: 7px;
  margin: 0 0 10px;
  font-size: 12.5px;
  color: var(--text-dim);
}

/* What has not gone out yet leads, in the colour of something unfinished. */
.tally.waiting {
  color: var(--amber-soft);
}

.tally.waiting .glyph {
  color: var(--amber);
}

.tally .glyph {
  color: var(--accent);
}

.already {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 0 0 10px;
  padding: 7px 10px;
  border-radius: 6px;
  background: var(--bg-raised);
  font-size: 12px;
  color: var(--text-dim);
}

.already.good {
  color: var(--green-soft);
}

.already.bad {
  color: var(--red-soft);
}

.open-left {
  margin: 0 0 10px;
  font-size: 11.5px;
}

.summary {
  width: 100%;
  font-family: var(--font);
  font-size: 12px;
  resize: vertical;
}

/* The verdicts, tinted towards what they say, matching the composer's. */
.verdict.approve:not(:disabled) {
  color: var(--green-soft);
  border-color: color-mix(in srgb, var(--green) 45%, transparent);
}

.verdict.request:not(:disabled) {
  color: var(--red-soft);
  border-color: color-mix(in srgb, var(--red) 45%, transparent);
}
</style>
