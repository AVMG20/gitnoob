<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  CornerDownRight,
  ExternalLink,
  MoreHorizontal,
  Quote
} from 'lucide-vue-next'
import PersonFace from './PersonFace.vue'
import CommentBox from './CommentBox.vue'
import { renderMarkdown } from '~/composables/useMd'
import { copyText, relativeTime } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useForge } from '~/composables/useForge'
import { useReview } from '~/composables/useReview'
import { quotedInto } from '~/composables/reviewThreads'
import type { RComment, Thread } from '~/composables/useReview'

const props = withDefaults(
  defineProps<{
    thread: Thread
    busy?: boolean
    /** The thread whose reply box is open, keyed by the thread's root. */
    replyTo?: number | null
    showLocation?: boolean
  }>(),
  { busy: false, replyTo: null, showLocation: false }
)

const emit = defineEmits<{
  reply: [id: number, body: string]
  toggleReply: [id: number]
  locate: [thread: Thread]
  resolve: [thread: Thread, resolved: boolean]
  quote: [comment: RComment]
}>()

const review = useReview()
const forge = useForge()
const menu = useContextMenu()

/**
 * Whether this thread can be answered where it stands.
 *
 * A remark on a line is a thread on both forges. A conversation comment is
 * not one on GitHub — there is no reply to it, only a quote into the box at
 * the foot of the page — so that is what is offered there instead of a button
 * that would post nothing.
 */
const answerable = computed(
  () =>
    props.thread.root.kind === 'diff' ||
    (forge.store.status?.kind === 'gitlab' && props.thread.replies.length > 0)
)

/** What can be done with one remark, in the menu GitHub keeps it in. */
function remarkMenu(event: MouseEvent, comment: RComment) {
  const url = review.commentUrl(comment)
  menu.show(
    event,
    [
      {
        label: 'Quote reply',
        icon: Quote,
        action: () => quote(comment)
      },
      {
        label: 'Copy link',
        icon: Copy,
        action: () => void copyText(url, 'Link')
      },
      {
        label: 'Copy markdown',
        icon: Copy,
        action: () => void copyText(comment.body, 'Markdown')
      },
      {
        label: `Open on ${forge.forgeName.value}`,
        icon: ExternalLink,
        action: () => void forge.open(url)
      }
    ],
    `remark-${comment.id}`
  )
}

const root = computed(() => props.thread.root)

const resolved = computed(() => root.value.resolved)

/**
 * A settled thread folds itself away.
 *
 * What it says has been dealt with, and a page of them is a page of noise
 * between the reader and what has not been — but it stays one click from
 * being read, since "why was that settled" is a real question.
 */
const open = ref(!root.value.resolved)
watch(resolved, (now) => (open.value = !now))

/**
 * What is being written in answer, held here rather than by the box.
 *
 * Quoting a remark writes into it, and a send that the forge refuses leaves it
 * where it was: the box closes, the words are still there when it opens again.
 */
const answer = ref('')

/**
 * Answers a remark by quoting it.
 *
 * A thread takes the quotation in its own box. A conversation comment has no
 * thread to take it — GitHub has no reply to one, only a quote — so it goes up
 * to the page, which writes it into the composer at the foot.
 */
function quote(comment: RComment) {
  if (!answerable.value) {
    emit('quote', comment)
    return
  }
  answer.value = quotedInto(answer.value, comment.body)
  if (props.replyTo !== props.thread.id) emit('toggleReply', props.thread.id)
}


function when(iso: string) {
  const at = Date.parse(iso)
  return Number.isNaN(at) ? '' : relativeTime(at / 1000)
}

function fullWhen(iso: string) {
  const at = Date.parse(iso)
  return Number.isNaN(at) ? '' : new Date(at).toLocaleString()
}
</script>

<template>
  <article class="thread" :class="{ settled: resolved }" data-testid="thread">
    <!-- Settled: one line saying so, and a way back into it. -->
    <button v-if="resolved && !open" class="folded" data-testid="thread-folded" @click="open = true">
      <Check :size="12" class="tick" />
      <span class="truncate">
        Settled ·
        {{ root.author.name || root.author.login }}: {{ root.body.split('\n')[0] }}
      </span>
      <span v-if="props.thread.replies.length" class="faint">
        {{ props.thread.replies.length }}
        {{ props.thread.replies.length === 1 ? 'reply' : 'replies' }}
      </span>
      <ChevronRight :size="12" class="faint" />
    </button>

    <template v-else>
      <div class="remark root">
        <PersonFace
          :login="root.author.login"
          :name="root.author.name"
          :src="root.author.avatar"
          :size="22"
        />
        <div class="said">
          <div class="meta">
            <span class="author">{{ root.author.name || root.author.login }}</span>
            <span class="faint" :title="fullWhen(root.created_at)">{{ when(root.created_at) }}</span>
            <button
              v-if="props.showLocation && root.kind === 'diff' && root.path"
              class="where mono truncate"
              :title="`Read it where it stands: ${root.path}`"
              @click="emit('locate', props.thread)"
            >
              {{ root.path }}{{ root.line !== null ? `:${root.line}` : '' }}
            </button>
            <span v-if="root.outdated" class="chip" title="The lines it was written on have moved">
              outdated
            </span>

            <span class="grow" />

            <button
              class="more"
              data-testid="remark-menu"
              title="More"
              @click="remarkMenu($event, root)"
            >
              <MoreHorizontal :size="12" />
            </button>
            <button
              v-if="resolved"
              class="fold"
              title="Fold this settled thread away again"
              @click="open = false"
            >
              <ChevronDown :size="12" />
            </button>
            <button
              v-if="root.resolvable"
              class="resolve"
              :class="{ on: resolved }"
              data-testid="thread-resolve"
              :title="resolved ? 'Open this thread again' : 'Mark this thread settled'"
              @click="emit('resolve', props.thread, !resolved)"
            >
              <Check :size="11" />
              {{ resolved ? 'Settled' : 'Resolve' }}
            </button>
          </div>
          <div class="md-body" v-html="renderMarkdown(root.body)" />
        </div>
      </div>

      <div v-if="props.thread.replies.length" class="replies">
        <div v-for="one in props.thread.replies" :key="one.id" class="remark">
          <PersonFace
            :login="one.author.login"
            :name="one.author.name"
            :src="one.author.avatar"
            :size="20"
          />
          <div class="said">
            <div class="meta">
              <span class="author">{{ one.author.name || one.author.login }}</span>
              <span class="faint" :title="fullWhen(one.created_at)">{{ when(one.created_at) }}</span>
              <span class="grow" />
              <button
                class="more"
                title="More"
                @click="remarkMenu($event, one)"
              >
                <MoreHorizontal :size="12" />
              </button>
            </div>
            <div class="md-body" v-html="renderMarkdown(one.body)" />
          </div>
        </div>
      </div>

      <div class="foot">
        <CommentBox
          v-if="answerable && props.replyTo === props.thread.id"
          v-model="answer"
          compact
          autofocus
          :busy="props.busy"
          send-label="Reply"
          placeholder="Reply…"
          @send="(body) => emit('reply', props.thread.id, body)"
          @cancel="emit('toggleReply', props.thread.id)"
        />
        <button
          v-else-if="answerable"
          class="reply"
          @click="emit('toggleReply', props.thread.id)"
        >
          <CornerDownRight :size="11" />
          Reply…
        </button>
        <!-- No thread to answer into: the forge takes a quotation in the box
             at the foot of the conversation, which is what this writes. -->
        <button v-else class="reply" data-testid="quote-reply" @click="quote(root)">
          <Quote :size="11" />
          Quote reply
        </button>
      </div>
    </template>
  </article>
</template>

<style scoped>
.thread {
  display: flex;
  flex-direction: column;
  gap: 7px;
  min-width: 0;
}

.folded {
  display: flex;
  align-items: center;
  gap: 7px;
  width: 100%;
  padding: 4px 2px;
  font-size: 11.5px;
  color: var(--text-faint);
  text-align: left;
  min-width: 0;
}

.folded:hover {
  color: var(--text-dim);
}

.folded .tick {
  color: var(--green);
  flex: none;
}

.remark {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  min-width: 0;
}

/* The root speaks with the accent behind it; the answers only line up under
   it, connected by a thread of their own. A settled root goes green: it is
   still a conversation, but not one anybody has to act on. */
.remark.root {
  border-left: 2px solid var(--accent);
  padding-left: 10px;
}

.settled .remark.root {
  border-left-color: var(--green);
}

.replies {
  margin-left: 18px;
  padding-left: 11px;
  border-left: 1px solid var(--line-soft);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.said {
  flex: 1;
  min-width: 0;
}

.meta {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11.5px;
  min-width: 0;
}

.grow {
  flex: 1;
}

.author {
  color: var(--text);
  font-weight: 600;
}

/* Where a diff thread stands, clickable because the conversation is not
   always where the reader already knows the line to be. */
.where {
  min-width: 0;
  max-width: 260px;
  font-size: 10.5px;
  color: var(--accent-soft);
  padding: 0 4px;
  border-radius: 3px;
}

.where:hover {
  background: var(--bg-hover);
  color: var(--accent);
}

.chip {
  flex: none;
  padding: 0 6px;
  border-radius: 999px;
  border: 1px solid color-mix(in srgb, var(--amber) 45%, transparent);
  color: var(--amber-soft);
  font-size: 10px;
}

.resolve,
.fold {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 999px;
  border: 1px solid var(--line);
  color: var(--text-faint);
  font-size: 10.5px;
}

.fold {
  padding: 2px 5px;
}

.resolve:hover,
.fold:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.resolve.on {
  color: var(--green-soft);
  border-color: color-mix(in srgb, var(--green) 45%, transparent);
}

.foot {
  margin-left: 18px;
  padding-left: 12px;
}

.more {
  flex: none;
  display: inline-flex;
  padding: 2px 4px;
  border-radius: 4px;
  color: var(--text-faint);
  opacity: 0;
}

.remark:hover .more,
.more:focus-visible {
  opacity: 1;
}

.more:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.reply {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 9px;
  border-radius: 999px;
  border: 1px solid var(--line);
  color: var(--text-dim);
  font-size: 11px;
}

.reply:hover {
  color: var(--text);
  background: var(--bg-hover);
}

/* The markdown a comment was written in, drawn as plain elements. The rules
   live under :deep because v-html content carries no scope attribute. */
.md-body {
  font-size: 12px;
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
