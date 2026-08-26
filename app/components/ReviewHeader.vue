<script setup lang="ts">
import { computed } from 'vue'
import {
  ArrowLeft,
  ArrowRight,
  Copy,
  ExternalLink,
  FileDiff,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  MessageSquare,
  MoreHorizontal,
  Pencil,
  PlayCircle,
  RotateCcw,
  RotateCw,
  Send,
  Settings,
  X
} from 'lucide-vue-next'
import PersonFace from './PersonFace.vue'
import ProfileMenu from './ProfileMenu.vue'
import Spinner from './Spinner.vue'
import { useReview } from '~/composables/useReview'
import { useForge } from '~/composables/useForge'
import { useConfig } from '~/composables/useConfig'
import { useContextMenu } from '~/composables/useContextMenu'
import { checkLook } from '~/composables/reviewLook'
import { copyText, relativeTime, useGit } from '~/composables/useGit'

/**
 * The head of a review page: what it is, where it is going, and the one thing
 * a reader is most likely to do with it.
 *
 * Three lines, each answering one question — what am I looking at, who and
 * where is it, and what can I do — instead of a single row of buttons that
 * gave the merge and a refresh the same weight.
 */
const emit = defineEmits<{
  merge: []
  finish: []
  edit: []
}>()

const review = useReview()
const forge = useForge()
const config = useConfig()
const git = useGit()
const menu = useContextMenu()
const store = review.store

const detail = computed(() => store.detail)
const one = computed(() => store.detail ?? store.current)

/** `open`, `draft`, `merged`, `closed` — whichever the forge means. */
const state = computed(() => {
  const here = one.value
  if (!here) return ''
  if (here.draft) return 'draft'
  const raw = here.state.toLowerCase()
  // GitLab says `opened`, GitHub says `open`; they mean the same thing.
  return raw === 'opened' ? 'open' : raw
})

const settled = computed(() => state.value === 'merged' || state.value === 'closed')

/** What each page of the review is called, and what it carries. */
const PAGES = computed(() => [
  {
    id: 'conversation' as const,
    label: 'Conversation',
    icon: MessageSquare,
    count: review.talkThreads().length + review.openThreads.value
  },
  { id: 'files' as const, label: 'Files', icon: FileDiff, count: store.files.length },
  {
    id: 'commits' as const,
    label: 'Commits',
    icon: GitCommitHorizontal,
    count: store.commits.length
  },
  {
    id: 'checks' as const,
    label: 'Checks',
    icon: PlayCircle,
    count: store.status?.checks.length ?? 0
  }
])

const checks = computed(() => checkLook(store.status?.checks_state ?? 'none'))

/** How much the review changes, said once where the title is read. */
const size = computed(() => ({
  files: store.files.length,
  additions: store.files.reduce((sum, file) => sum + file.additions, 0),
  deletions: store.files.reduce((sum, file) => sum + file.deletions, 0)
}))

/** How much the reader has said here, which is what finishing will send. */
const mine = computed(() => {
  const me = forge.store.me?.login
  if (!me) return []
  return store.comments.filter((comment) => comment.author.login === me)
})

function when(iso: string) {
  const at = Date.parse(iso)
  return Number.isNaN(at) ? '' : relativeTime(at / 1000)
}

function checkout() {
  const current = store.current
  if (!current) return
  void git.checkoutReview({
    number: current.number,
    branch: current.source_branch,
    head_sha: current.head_sha,
    source: current.source
  })
}

/** Everything worth doing that is not worth a button of its own. */
function more(event: MouseEvent) {
  const current = store.current
  if (!current) return
  menu.show(
    event,
    [
      { label: 'Edit title and description', icon: Pencil, action: () => emit('edit') },
      ...(state.value === 'draft'
        ? [
            {
              label: 'Mark ready for review',
              icon: Send,
              action: () => void review.setDraft(false)
            }
          ]
        : state.value === 'open'
          ? [
              {
                label: 'Convert back to a draft',
                icon: RotateCcw,
                action: () => void review.setDraft(true)
              }
            ]
          : []),
      ...(settled.value
        ? state.value === 'closed'
          ? [{ label: 'Reopen', icon: RotateCcw, action: () => void review.setState('reopen') }]
          : []
        : [{ label: 'Close without merging', icon: X, action: () => void review.setState('close') }]),
      { label: 'Check out the branch', icon: GitBranch, action: checkout },
      { label: 'Copy the link', icon: Copy, action: () => void copyText(current.url, 'Link') },
      {
        label: 'Copy the branch name',
        icon: Copy,
        action: () => void copyText(current.source_branch, 'Branch')
      },
      {
        label: 'Open on the forge',
        icon: ExternalLink,
        action: () => void forge.open(current.url)
      }
    ],
    `review-${current.number}`
  )
}
</script>

<template>
  <header v-if="one" class="review-header">
    <!-- What it is. -->
    <div class="line title-line">
      <button
        class="btn back"
        data-testid="review-close"
        title="Back to the graph (Esc)"
        @click="review.close()"
      >
        <ArrowLeft :size="14" />
        <span>Back</span>
      </button>

      <span class="number mono" :title="`Review ${forge.sigil.value}${one.number}`">
        {{ forge.sigil.value }}{{ one.number }}
      </span>

      <h2 class="title truncate" :title="one.title">{{ one.title }}</h2>

      <span class="state" :class="state" data-testid="review-state">{{ state }}</span>

      <span class="grow" />

      <button
        class="icon"
        title="Read the review again"
        data-testid="review-refresh"
        :disabled="store.acting !== null"
        @click="review.refreshConversation()"
      >
        <RotateCw :size="14" />
      </button>
      <button class="icon" title="Open on the forge" @click="forge.open(one.url)">
        <ExternalLink :size="14" />
      </button>
      <button class="icon" title="Check out the branch" @click="checkout">
        <GitBranch :size="14" />
      </button>
      <button class="icon" title="More" data-testid="review-more" @click="more">
        <MoreHorizontal :size="14" />
      </button>

      <!-- The toolbar stands down while a review is open, so the two things on
           it that are not about the working tree come along: which account
           this is being read as, and where the settings are. -->
      <span class="sep" />

      <button class="icon" title="Settings" @click="config.openSettings('profiles')">
        <Settings :size="14" />
      </button>
      <ProfileMenu />
    </div>

    <!-- Whose it is, and where it goes. -->
    <div class="line meta-line">
      <template v-if="detail">
        <PersonFace
          :login="detail.author.login"
          :name="detail.author.name"
          :src="detail.author.avatar"
          :size="16"
        />
        <span class="who">{{ detail.author.name || detail.author.login }}</span>
        <span class="faint" :title="new Date(detail.created_at).toLocaleString()">
          opened {{ when(detail.created_at) }}
        </span>
        <span class="dot">·</span>
      </template>

      <span class="branches mono truncate" :title="`${one.source_branch} → ${one.target_branch}`">
        {{ one.source_branch }}
        <ArrowRight :size="10" class="faint" />
        {{ one.target_branch }}
      </span>

      <template v-if="size.files">
        <span class="dot">·</span>
        <span class="faint">{{ size.files }} {{ size.files === 1 ? 'file' : 'files' }}</span>
        <span class="plus">+{{ size.additions }}</span>
        <span class="minus">−{{ size.deletions }}</span>
      </template>

      <template v-if="review.openThreads.value">
        <span class="dot">·</span>
        <span class="open-threads" :title="'Threads still open on the diff'">
          {{ review.openThreads.value }} open
          {{ review.openThreads.value === 1 ? 'thread' : 'threads' }}
        </span>
      </template>
    </div>

    <!-- What can be done with it. -->
    <nav class="line tabs-line">
      <div class="tabs">
        <button
          v-for="page in PAGES"
          :key="page.id"
          class="tab"
          :class="{ on: store.tab === page.id }"
          :data-testid="`tab-${page.id}`"
          @click="store.tab = page.id"
        >
          <component
            :is="page.id === 'checks' && store.status ? checks.icon : page.icon"
            :size="13"
            :class="page.id === 'checks' && store.status ? `tone-${checks.tone}` : ''"
          />
          {{ page.label }}
          <span v-if="page.count" class="count">{{ page.count }}</span>
        </button>
      </div>

      <span class="grow" />

      <button
        class="btn btn-ghost finish"
        data-testid="review-mode-toggle"
        :title="
          store.pending.length
            ? `Finish the review — ${store.pending.length} waiting to go out`
            : `Finish the review — ${mine.length} ${mine.length === 1 ? 'remark' : 'remarks'} made`
        "
        @click="emit('finish')"
      >
        <MessageSquare :size="13" />
        Review
        <span
          v-if="mine.length || store.pending.length"
          class="made"
          :class="{ waiting: store.pending.length }"
          data-testid="review-count"
        >
          {{ store.pending.length || mine.length }}
        </span>
      </button>

      <button
        v-if="state === 'draft'"
        class="btn btn-primary act"
        data-testid="ready-button"
        :disabled="store.acting !== null"
        @click="review.setDraft(false)"
      >
        <Spinner v-if="store.acting === 'ready'" :size="12" />
        <Send v-else :size="14" />
        Mark ready
      </button>
      <button
        v-else-if="!settled"
        class="btn btn-primary act"
        data-testid="merge-button"
        :disabled="store.acting !== null"
        @click="emit('merge')"
      >
        <Spinner v-if="store.acting === 'merge'" :size="12" />
        <GitMerge v-else :size="14" />
        Merge
      </button>
    </nav>
  </header>
</template>

<style scoped>
.review-header {
  display: flex;
  flex-direction: column;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
}

.line {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  padding: 0 16px;
}

.title-line {
  gap: 6px;
  padding-top: 7px;
}

/* The three lines share one left edge: the arrow, the face and the first
   tab all start at the same 12px, whatever padding their own control has. */
.back {
  padding: 3px 8px;
  margin-left: -8px;
  font-size: 12px;
}

.number {
  flex: none;
  font-size: 12px;
  color: var(--text-faint);
}

.title {
  margin: 0;
  font-size: 14.5px;
  font-weight: 600;
  line-height: 1.25;
  min-width: 0;
}

/* The state, said in the colour it means: this is the first thing a reader
   checks and the last thing they should have to hunt for. */
.state {
  flex: none;
  padding: 2px 8px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border: 1px solid var(--line);
  color: var(--text-faint);
}

.state.open {
  color: var(--green-soft);
  background: color-mix(in srgb, var(--green) 14%, transparent);
  border-color: color-mix(in srgb, var(--green) 40%, transparent);
}

.state.draft {
  color: var(--amber-soft);
  background: color-mix(in srgb, var(--amber) 14%, transparent);
  border-color: color-mix(in srgb, var(--amber) 40%, transparent);
}

.state.merged {
  color: var(--purple-soft);
  background: color-mix(in srgb, var(--purple) 14%, transparent);
  border-color: color-mix(in srgb, var(--purple) 40%, transparent);
}

.state.closed {
  color: var(--red-soft);
  background: color-mix(in srgb, var(--red) 14%, transparent);
  border-color: color-mix(in srgb, var(--red) 40%, transparent);
}

/* A hairline between the review's own actions and the app's. */
.sep {
  width: 1px;
  height: 16px;
  margin: 0 3px;
  background: var(--line);
}

.icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 6px;
  color: var(--text-faint);
}

.icon:hover:not(:disabled) {
  color: var(--text);
  background: var(--bg-hover);
}

.icon:disabled {
  opacity: 0.4;
}

.meta-line {
  padding-top: 3px;
  padding-bottom: 7px;
  font-size: 11.5px;
  color: var(--text-dim);
  flex-wrap: wrap;
  gap: 6px;
}

.who {
  color: var(--text);
  font-weight: 600;
}

.dot {
  color: var(--text-faint);
}

.branches {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  max-width: 460px;
  font-size: 11px;
  color: var(--text-dim);
}

.plus {
  color: var(--green);
}

.minus {
  color: var(--red);
}

.open-threads {
  color: var(--amber-soft);
}

/* The tabs carry the counts, so the pages say what is in them before they are
   opened; the actions sit at the far end, weighted by how big a step each is. */
/* The pages and the two big actions are a row of their own business, so a
   hairline separates them from what the review is. */
.tabs-line {
  gap: 8px;
  padding-top: 5px;
  padding-bottom: 6px;
  border-top: 1px solid var(--line-soft);
}

.tabs {
  display: flex;
  gap: 2px;
  min-width: 0;
  margin-left: -10px;
}

.tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border-radius: 6px;
  font-size: 12px;
  color: var(--text-dim);
  white-space: nowrap;
}

.tab:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.tab.on {
  color: var(--text);
  background: var(--bg-active);
  font-weight: 600;
}

.count {
  padding: 0 5px;
  border-radius: 8px;
  background: var(--bg-raised);
  font-size: 10px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: var(--text-dim);
}

.tab.on .count {
  background: rgba(0, 0, 0, 0.22);
  color: var(--text);
}

.tone-good {
  color: var(--green);
}

.tone-bad {
  color: var(--red);
}

.tone-wait {
  color: var(--amber);
}

.tone-none {
  color: var(--text-faint);
}

.finish {
  padding: 4px 10px;
  font-size: 12px;
}

/* Amber while remarks are held back: the count is what is owed rather than
   what has already been said. */
.made.waiting {
  background: color-mix(in srgb, var(--amber) 22%, transparent);
  color: var(--amber-soft);
}

.made {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: 8px;
  background: var(--bg-raised);
  font-size: 10.5px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}

/* The one big step on the page, drawn like one. */
.act {
  padding: 5px 14px;
  font-size: 12.5px;
  min-width: 104px;
  justify-content: center;
}

.grow {
  flex: 1;
}
</style>
