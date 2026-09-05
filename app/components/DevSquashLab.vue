<script setup lang="ts">
import { onMounted, ref } from 'vue'
import GraphList from './GraphList.vue'
import WorkingChanges from './WorkingChanges.vue'
import DiffViewer from './DiffViewer.vue'
import SideBar from './SideBar.vue'
import ContextMenu from './ContextMenu.vue'
import ActivityLog from './ActivityLog.vue'
import SettingsModal from './SettingsModal.vue'
import { useGit, type GraphRow, type SquashPreview } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'

/**
 * Squashing, moved files and the branch menu, on fixtures, in a browser.
 *
 * Reached with `?lab=squash` on the dev server and compiled out of anything
 * built for release. The three things it shows are the ones that are otherwise
 * only reachable by arranging a repository into a particular state: a run of
 * commits worth folding, a file that was moved and edited afterwards, and the
 * menu on the branch you are standing on.
 */
const git = useGit()
const config = useConfig()

const author = 'Robin Vale'
const now = Math.floor(Date.now() / 1000)

function commit(oid: string, summary: string, ago: number, over: Partial<GraphRow> = {}): GraphRow {
  return {
    oid,
    short: oid.slice(0, 7),
    summary,
    author,
    email: 'robin@example.com',
    time: now - ago,
    parents: [],
    lane: 0,
    color: 0,
    width: 1,
    segments: [{ x1: 0, y1: 1, x2: 0, y2: 2, color: 0, dashed: false, faint: false, current: false }],
    labels: [],
    unpushed: false,
    unpulled: false,
    carries: [],
    stash: null,
    ...over
  }
}

const ROWS: GraphRow[] = [
  commit('a1111111111111111111111111111111111111a1', 'Disallow unmerging tickets after replies', 60, {
    labels: [{ kind: 'local', name: 'tickets', head: true }],
    parents: ['a2222222222222222222222222222222222222a2']
  }),
  commit('a2222222222222222222222222222222222222a2', 'wip: fix the migration name', 3600, {
    parents: ['a3333333333333333333333333333333333333a3']
  }),
  commit('a3333333333333333333333333333333333333a3', 'Squash ticket and email migrations', 7200, {
    parents: ['a4444444444444444444444444444444444444a4']
  }),
  commit('a4444444444444444444444444444444444444a4', 'Send an automatic reply', 68400, {
    labels: [{ kind: 'remote', name: 'origin/tickets', head: false }],
    parents: ['a5555555555555555555555555555555555555a5']
  }),
  commit('a5555555555555555555555555555555555555a5', 'Show the inherited subject', 75600, {
    parents: []
  })
]

const MESSAGES: Record<string, string> = {
  a2222222222222222222222222222222222222a2: 'wip: fix the migration name',
  a3333333333333333333333333333333333333a3:
    'Squash ticket and email migrations\n\nThe two tables were added a week apart and there is no\nrelease between them, so one migration is honest and two\nis archaeology.',
  a4444444444444444444444444444444444444a4: 'Send an automatic reply'
}

/** The same words the backend's own default carries, near enough to look at. */
const DEFAULT_COMMIT_PROMPT = [
  'You write git commit messages for a working developer. Reply with the message and nothing else: no preamble, no markdown, no code fences, no quotes.',
  '',
  'Line 1 is the message, and usually the whole of it: imperative mood, no trailing period, under 72 characters, specific about what changed.',
  '',
  'Add a body only where the summary cannot carry the change on its own. When you do, leave a blank line after the summary and keep it to one or two sentences on WHY. Most commits need no body at all. Never list the files, never restate the diff, never pad.'
].join('\n')

/** What the fixture config has stored, so saving from the box is visible. */
const written = ref<string | null>(null)

const MOVED_TO = 'tests/Feature/Filament/Tickets/CreateTicketFormTest.php'
const MOVED_FROM = 'tests/Feature/Filament/CreateTicketFormTest.php'

/** What the backend would answer for the commits picked out in the graph. */
function squashPreview(oids: string[]): SquashPreview {
  const ordered = ROWS.filter((row) => oids.includes(row.oid)).sort((a, b) => a.time - b.time)
  const positions = ordered.map((row) => ROWS.indexOf(row))
  const run = positions.every((at, index) => index === 0 || at === positions[index - 1]! - 1)
  const commits = ordered.map((row) => ({
    oid: row.oid,
    short: row.short,
    summary: row.summary,
    message: MESSAGES[row.oid] ?? row.summary,
    author: row.author,
    time: row.time,
    // Anything at or below the remote chip has left this machine.
    pushed: ROWS.indexOf(row) >= 3
  }))
  const highest = Math.max(...positions)
  return {
    commits,
    message: commits.map((one) => one.message).join('\n\n'),
    onto: ROWS[highest + 1]?.short ?? null,
    above: Math.min(...positions),
    branch: 'tickets',
    refusal: run
      ? null
      : `Those commits are not next to each other: ${
          highest + 1 - positions.length
        } other commits sit between them. Squashing folds a run with nothing in the middle — use the rebase plan to move them together first.`
  }
}

/**
 * Answers the commands these three panels send.
 *
 * `invoke` goes through this hook, which is the seam the Tauri window fills in;
 * in a browser there is nothing there, so every call would reject and the
 * panels would sit empty.
 */
function install() {
  const internals = ((window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ ??=
    {}) as Record<string, unknown>
  internals.invoke = async (cmd: string, args: Record<string, unknown> = {}) => {
    if (cmd === 'squash_preview') return squashPreview((args.oids ?? []) as string[])
    if (cmd === 'squash') return `Squashed ${(args.oids as string[]).length} commits into one`
    if (cmd === 'commit_detail') {
      const row = ROWS.find((one) => one.oid === args.oid)
      return row
        ? {
            oid: row.oid,
            short: row.short,
            summary: row.summary,
            body: '',
            author: row.author,
            email: row.email,
            time: row.time,
            committer: row.author,
            commit_time: row.time,
            parents: row.parents,
            files: []
          }
        : null
    }
    if (cmd === 'commit_message_text') return MESSAGES[String(args.oid)] ?? ''
    if (cmd === 'working_file_diff') {
      // The staged side of the moved file is a pure rename: nothing inside it
      // changed, so the viewer draws the "moved, contents unchanged" page.
      // Kept in the fixture because that page is otherwise hard to reach on
      // purpose.
      if (args.side === 'staged' && String(args.path) === MOVED_TO) {
        return { path: MOVED_TO, from: MOVED_FROM, binary: false, truncated: 0, hunks: [] }
      }
      return {
        path: String(args.path),
        binary: false,
        truncated: 0,
        hunks: [
          {
            header: '@@ -18,6 +18,7 @@',
            lines: [
              { origin: ' ', old_lineno: 18, new_lineno: 18, content: '    public function setUp(): void' },
              { origin: '+', old_lineno: null, new_lineno: 19, content: '        $this->actingAs($this->ticketAgent());' },
              { origin: ' ', old_lineno: 19, new_lineno: 20, content: '    }' }
            ]
          }
        ]
      }
    }
    if (cmd === 'ai_status') {
      return {
        configured: true,
        model: 'anthropic/claude-sonnet-4.5',
        default_commit_prompt: DEFAULT_COMMIT_PROMPT
      }
    }
    if (cmd === 'reset_preview') {
      // The hard-reset question, which is the only reset that asks one.
      const at = ROWS.findIndex((one) => one.oid === args.oid)
      return {
        target: String(args.oid),
        short: ROWS[at]?.short ?? '0000000',
        summary: ROWS[at]?.summary ?? '',
        branch: 'tickets',
        dropped: ROWS.slice(0, Math.max(at, 0)).map((one) => ({
          oid: one.oid,
          short: one.short,
          summary: one.summary,
          author: one.author,
          time: one.time
        })),
        diverges: false,
        staged_files: 1,
        unstaged_files: 3
      }
    }
    if (cmd === 'ai_squash_message') {
      // What the model is for here: one message about the fold, in place of
      // the join of the messages being folded.
      return {
        summary: 'Add the ticket and email migrations together',
        body: 'They were written a week apart with no release between them, so one migration is the honest record.'
      }
    }
    if (cmd === 'ai_models') return []
    if (cmd === 'config_get' || cmd === 'config_set_global') {
      const patched = (args.global ?? null) as { ai?: { commit_prompt?: string | null } } | null
      if (patched) written.value = patched.ai?.commit_prompt ?? null
      return {
        version: 1,
        active_profile: null,
        global: {
          show_avatars: true,
          auto_fetch_minutes: 10,
          graph_page_size: 500,
          ai: {
            model: 'anthropic/claude-sonnet-4.5',
            max_tokens: 1500,
            reasoning: 'off',
            commit_style: 'plain',
            commit_prompt: written.value
          }
        },
        profiles: []
      }
    }
    return null
  }
}

onMounted(async () => {
  install()
  // `?lab=squash&settings=ai` opens the settings on a section, for looking at
  // a form that is otherwise three clicks into a running app.
  const wanted = new URLSearchParams(window.location.search).get('settings')
  if (wanted) {
    await config.load()
    config.openSettings(wanted as never)
  }
  git.store.repo = {
    path: '/repo',
    name: 'support-desk',
    head: 'tickets',
    detached: false,
    author
  } as never
  git.store.rows = ROWS
  git.store.hasMore = false
  git.store.refs = {
    locals: [
      { name: 'tickets', oid: ROWS[0]!.oid, is_head: true, upstream: 'origin/tickets', ahead: 2, behind: 0 },
      { name: 'staging', oid: ROWS[3]!.oid, is_head: false, upstream: 'origin/staging', ahead: 0, behind: 0 }
    ],
    remotes: [],
    tags: [],
    stashes: []
  }
  git.store.status = {
    staged: [{ path: MOVED_TO, from: MOVED_FROM, kind: 'renamed' }],
    unstaged: [
      { path: MOVED_TO, from: null, kind: 'modified' },
      { path: 'app/Rules/DutchPhoneNumber.php', from: null, kind: 'untracked' },
      { path: 'tests/Feature/Rules/DutchPhoneNumberTest.php', from: 'tests/DutchPhoneNumberTest.php', kind: 'renamed' }
    ],
    conflicted: []
  }
})
</script>

<template>
  <div class="lab">
    <SideBar class="side" />
    <!-- Opening a file takes over the middle, the same way the shell does it. -->
    <DiffViewer v-if="git.store.viewer" class="graph" />
    <GraphList v-else class="graph" />
    <WorkingChanges class="work" />
    <ActivityLog class="console" />
    <ContextMenu />
    <SettingsModal v-if="config.store.settingsOpen" />
  </div>
</template>

<style scoped>
/* The three panels side by side with the console under them, which is the
   shape the window has — the console is sized against what is left. */
.lab {
  display: grid;
  grid-template-columns: 260px minmax(0, 1fr) 380px;
  grid-template-rows: minmax(0, 1fr) auto;
  height: 100vh;
  min-height: 0;
}

.console {
  grid-column: 1 / -1;
}

.side,
.graph,
.work {
  min-width: 0;
  min-height: 0;
  border-left: 1px solid var(--line);
  overflow: hidden;
}
</style>
