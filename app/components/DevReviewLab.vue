<script setup lang="ts">
import { onMounted } from 'vue'
import ReviewPane from './ReviewPane.vue'
import ReviewFilesPanel from './ReviewFilesPanel.vue'
import { useReview } from '~/composables/useReview'
import { useForge } from '~/composables/useForge'
import { parsePatch } from '~/composables/usePatch'

/**
 * The review page on fixture data, for looking at it in a plain browser.
 *
 * Reached with `?lab=review` on the dev server and compiled out of anything
 * built for release. Nothing here talks to the backend: the store is filled
 * straight from the tables below, so the pane can be read, resized and
 * clicked through without a repository, a forge or a token — which is the
 * only way to look at a two-hundred-file review while designing one.
 */
const review = useReview()
const forge = useForge()
const store = review.store

const now = Date.now()
const ago = (hours: number) => new Date(now - hours * 3600_000).toISOString()
const who = (login: string, name: string) => ({ login, name, avatar: null })

const PATCH = `@@ -18,9 +18,24 @@ import { useForge } from './useForge'
 const store = reactive({
   current: null as Review | null,
   detail: null as ReviewDetail | null,
-  loading: false,
+  loadingDetail: false,
+  loadingStatus: false,
+
+  /** How the review stands: what ran, who said what, whether it can land. */
+  status: null as ReviewStatus | null,

   comments: [] as RComment[],
   files: [] as RFileWithDiff[],
 })
+
+/** Every thread standing on a line, whatever file it belongs to. */
+const diffThreads = computed(() => [...folded.value.byLine.values()].flat())
+
+/** How many are still open, which is what is left to answer. */
+const openThreads = computed(
+  () => diffThreads.value.filter((thread) => !thread.root.resolved).length
+)

 export function useReview() {
   const forge = useForge()`

const FILES = [
  ['app/components/ReviewPane.vue', 'modified', 212, 168],
  ['app/components/ReviewHeader.vue', 'added', 431, 0],
  ['app/components/ReviewSidebar.vue', 'added', 388, 0],
  ['app/components/ReviewChecks.vue', 'added', 210, 0],
  ['app/components/ReviewMergeDialog.vue', 'added', 196, 0],
  ['app/components/ReviewThread.vue', 'modified', 148, 42],
  ['app/composables/useReview.ts', 'modified', 174, 12],
  ['app/composables/reviewLook.ts', 'added', 118, 0],
  ['src-tauri/src/forge.rs', 'modified', 640, 31],
  ['docs/reviewing.md', 'renamed', 12, 4]
] as const

onMounted(() => {
  // Anything that would reach the backend answers with nothing rather than
  // throwing: the lab is for looking, not for merging.
  const shell = window as unknown as { __TAURI_INTERNALS__?: unknown }
  shell.__TAURI_INTERNALS__ ??= {
    invoke: async (command: string) => {
      // Enough of an answer for the pickers to be opened and looked at; the
      // rest succeeds silently, since nothing here is really being merged.
      if (command === 'forge_project_labels') {
        return [
          { name: 'enhancement', color: '#a2eeef' },
          { name: 'ui', color: '#d4c5f9' },
          { name: 'bug', color: '#d73a4a' },
          { name: 'needs discussion', color: '#f0a83c' }
        ]
      }
      return null
    },
    transformCallback: (callback: unknown) => callback
  }

  forge.store.status = {
    kind: 'github',
    host: 'github.com',
    has_token: true,
    user: 'arno',
    slug: { host: 'github.com', owner: 'bigbridge', name: 'nuxtpolymarket' },
    error: null
  }
  forge.store.me = { login: 'arno', id: 1, avatar: null }
  forge.store.members = [
    { id: 1, login: 'arno', name: 'Arno Visker' },
    { id: 2, login: 'nadia', name: 'Nadia Petrova' },
    { id: 3, login: 'kai', name: 'Kai Moens' }
  ]

  store.current = {
    number: 68,
    title: 'Read and answer reviews without leaving the app',
    author: 'kai',
    state: 'open',
    draft: false,
    source_branch: 'feature/review-page',
    target_branch: 'main',
    url: 'https://github.com/bigbridge/nuxtpolymarket/pull/68',
    updated_at: ago(4),
    is_current: false,
    head_sha: 'f'.repeat(40),
    source: null,
    warning: null
  }

  store.detail = {
    number: 68,
    title: store.current.title,
    body: [
      'The review page grew a header of loose buttons and a file tree that stayed',
      'pinned next to the conversation. This gives it a shape:',
      '',
      '- one header, three lines: **what it is**, whose it is, what can be done',
      '- the conversation reads diff threads too, and they can be settled here',
      '- `checks`, verdicts and mergeability come from one status call',
      '- merging asks once, in a dialog that says what it will do',
      '',
      'Closes #64.'
    ].join('\n'),
    state: 'open',
    draft: false,
    author: who('kai', 'Kai Moens'),
    assignees: [who('arno', 'Arno Visker')],
    reviewers: [who('nadia', 'Nadia Petrova'), who('sam', 'Sam Okafor')],
    labels: [
      { name: 'enhancement', color: '#a2eeef' },
      { name: 'ui', color: '#d4c5f9' }
    ],
    milestone: '0.4',
    source_branch: 'feature/review-page',
    target_branch: 'main',
    url: store.current.url,
    created_at: ago(52),
    updated_at: ago(4),
    comments: 6,
    merge_status: 'clean',
    base_sha: 'a'.repeat(40),
    head_sha: 'f'.repeat(40),
    start_sha: 'b'.repeat(40)
  }

  store.comments = [
    {
      id: 1,
      author: who('nadia', 'Nadia Petrova'),
      body: 'Reading this in the app is the whole point — nice.\n\nOne thing: the tab counts should not include settled threads.',
      created_at: ago(30),
      updated_at: ago(30),
      kind: 'issue',
      path: null,
      line: null,
      side: null,
      reply_to: null,
      thread: '',
      resolvable: false,
      resolved: false,
      outdated: false
    },
    {
      id: 2,
      author: who('kai', 'Kai Moens'),
      body: 'Fixed — `openThreads` counts the open ones only.',
      created_at: ago(28),
      updated_at: ago(28),
      kind: 'issue',
      path: null,
      line: null,
      side: null,
      reply_to: 1,
      thread: '',
      resolvable: false,
      resolved: false,
      outdated: false
    },
    {
      id: 3,
      author: who('sam', 'Sam Okafor'),
      body: 'Is `loadingStatus` worth its own flag, or should it ride on `loadingDetail`?',
      created_at: ago(20),
      updated_at: ago(20),
      kind: 'diff',
      path: 'app/composables/useReview.ts',
      line: 22,
      side: 'new',
      reply_to: null,
      thread: 'thread-a',
      resolvable: true,
      resolved: false,
      outdated: false
    },
    {
      id: 4,
      author: who('kai', 'Kai Moens'),
      body: 'Its own: the description arrives in one request and the standing in four.',
      created_at: ago(19),
      updated_at: ago(19),
      kind: 'diff',
      path: 'app/composables/useReview.ts',
      line: 22,
      side: 'new',
      reply_to: 3,
      thread: 'thread-a',
      resolvable: true,
      resolved: false,
      outdated: false
    },
    {
      id: 5,
      author: who('arno', 'Arno Visker'),
      body: 'This one is settled — the flat `.values()` is fine.',
      created_at: ago(12),
      updated_at: ago(12),
      kind: 'diff',
      path: 'app/composables/useReview.ts',
      line: 30,
      side: 'new',
      reply_to: null,
      thread: 'thread-b',
      resolvable: true,
      resolved: true,
      outdated: false
    }
  ]

  store.files = FILES.map(([path, status, additions, deletions]) => ({
    path,
    old_path: status === 'renamed' ? 'docs/review.md' : null,
    status: status as 'added' | 'modified' | 'deleted' | 'renamed',
    additions,
    deletions,
    binary: false,
    patch: PATCH,
    hunks: parsePatch(PATCH).hunks
  }))
  store.selectedPath = 'app/composables/useReview.ts'
  store.viewed = new Set(['docs/reviewing.md', 'app/components/ReviewChecks.vue'])

  store.commits = [
    { sha: 'f'.repeat(40), message: 'Give the review page a header with a shape', author: 'Kai Moens', created_at: ago(50) },
    { sha: 'e'.repeat(40), message: 'Read diff threads in the conversation too', author: 'Kai Moens', created_at: ago(40) },
    { sha: 'd'.repeat(40), message: 'Ask the forge how the review stands', author: 'Arno Visker', created_at: ago(9) }
  ]

  store.status = {
    checks: [
      { name: 'build · install', state: 'success', description: '', url: 'https://ci.test/1' },
      { name: 'build · typecheck', state: 'success', description: '', url: 'https://ci.test/2' },
      { name: 'test · vitest', state: 'failure', description: '2 failed, 116 passed', url: 'https://ci.test/3' },
      { name: 'test · cargo', state: 'pending', description: 'running for 4 minutes', url: 'https://ci.test/4' },
      { name: 'deploy · preview', state: 'skipped', description: 'manual', url: 'https://ci.test/5' }
    ],
    checks_state: 'failure',
    verdicts: [
      { author: who('nadia', 'Nadia Petrova'), state: 'approved', submitted_at: ago(26), body: 'Reads much better.' },
      { author: who('sam', 'Sam Okafor'), state: 'changes_requested', submitted_at: ago(18), body: 'The failing vitest run first, please.' }
    ],
    approvals: 1,
    approvals_required: 2,
    mergeable: true,
    merge_status: 'blocked',
    conflicts: false
  }
})
</script>

<template>
  <div class="lab">
    <ReviewPane />
    <ReviewFilesPanel v-if="store.tab === 'files'" class="panel" />
  </div>
</template>

<style scoped>
/* The same two columns app.vue gives a review, without the rest of the shell:
   the pane, and the file list beside it while the files are being read. */
.lab {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  height: 100%;
  min-height: 0;
  background: var(--bg);
}

.panel {
  width: 320px;
}
</style>
