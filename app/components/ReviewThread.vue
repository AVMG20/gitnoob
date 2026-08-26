<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check, ChevronDown, ChevronRight, CornerDownRight } from 'lucide-vue-next'
import PersonFace from './PersonFace.vue'
import CommentBox from './CommentBox.vue'
import { renderMarkdown } from '~/composables/useMd'
import { relativeTime } from '~/composables/useGit'
import type { Thread } from '~/composables/useReview'

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
}>()

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
            </div>
            <div class="md-body" v-html="renderMarkdown(one.body)" />
          </div>
        </div>
      </div>

      <div class="foot">
        <CommentBox
          v-if="props.replyTo === props.thread.id"
          compact
          autofocus
          :busy="props.busy"
          send-label="Reply"
          placeholder="Reply…"
          @send="(body) => emit('reply', props.thread.id, body)"
          @cancel="emit('toggleReply', props.thread.id)"
        />
        <button v-else class="reply" @click="emit('toggleReply', props.thread.id)">
          <CornerDownRight :size="11" />
          Reply…
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
