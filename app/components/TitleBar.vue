<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  ArrowDown,
  ArrowDownToLine,
  ArrowUp,
  ArrowUpFromLine,
  Archive,
  GitBranchPlus,
  History,
  RefreshCw,
  Redo2,
  Settings,
  TriangleAlert,
  Undo2
} from 'lucide-vue-next'
import { useGit, type CommitSummary } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'

const git = useGit()
const store = git.store
const config = useConfig()

const showBranch = ref(false)
const showHistory = ref(false)

const head = computed(() => store.refs?.locals.find((b) => b.is_head) ?? null)

/**
 * A branch to offer as the way out of a detached HEAD.
 *
 * The branch that was checked out before is not recorded anywhere the frontend
 * can see, so this picks the most plausible destination instead: `main` or
 * `master` if either exists, otherwise the first local branch. Better a named
 * button than none.
 */
const lastBranch = computed(() => {
  const locals = store.refs?.locals ?? []
  const usual = locals.find((b) => b.name === 'main' || b.name === 'master')
  return (usual ?? locals[0])?.name ?? null
})
const conflicts = computed(() => store.status?.conflicted.length ?? 0)
const nextUndo = computed(() => store.history.undo[0] ?? null)
const nextRedo = computed(() => store.history.redo[0] ?? null)

/** Push is push: no dialog. If the remote refuses, the strip below offers the
    way out, which is the only moment a choice is actually needed. */
function push() {
  if (!head.value) return
  return git.pushBranch(head.value.name, !head.value.upstream)
}

// --- a refused push
const blocked = computed(() => store.pushBlocked)
const confirming = ref(false)
const checkingForce = ref(false)
const orphans = ref<CommitSummary[]>([])

/** Pull, and if that leaves the branch pushable, push straight away. */
async function pullThen(rebase: boolean) {
  // Read the target first: dismissing clears it.
  const target = blockedTarget()
  git.dismissPushBlock()
  const ok = await git.pull(rebase)
  if (!ok || !target) return
  if (store.status?.conflicted.length) {
    store.resolving = store.status.conflicted[0] ?? null
    return
  }
  await git.push(target.remote, target.branch, false, false)
}

/**
 * Asks before rewriting, and names what would go: the count comes from a fresh
 * preview rather than from whatever the last dialog happened to read.
 */
async function askForce() {
  confirming.value = true
  checkingForce.value = true
  orphans.value = []
  const preview = await git.pushPreview(store.pushBlocked?.branch, true)
  orphans.value = preview?.will_orphan ?? []
  checkingForce.value = false
}

async function forcePush() {
  const target = blockedTarget()
  if (!target) return
  confirming.value = false
  await git.push(target.remote, target.branch, true, false)
}

/** The push that was refused, captured before dismissing clears it. */
function blockedTarget() {
  const block = store.pushBlocked
  return block ? { remote: block.remote, branch: block.branch } : null
}

// A new rejection starts the offer over, never mid-confirmation.
watch(
  () => store.pushBlocked,
  () => {
    confirming.value = false
    orphans.value = []
  }
)
</script>

<template>
  <header class="bar">
    <div class="repo">
      <strong class="name">{{ store.repo?.name }}</strong>
      <span class="faint">/</span>
      <span class="branch mono">
        {{ store.repo?.head }}
        <span v-if="store.repo?.detached" class="pill">detached</span>
      </span>
      <!-- Icons rather than ↑ and ↓: at this size the text arrow crowds the
           digit next to it and "↑1" reads as "11". -->
      <span v-if="head?.ahead" class="pill up" :title="`${head.ahead} to push`">
        <ArrowUp :size="11" :stroke-width="2.5" />{{ head.ahead }}
      </span>
      <span v-if="head?.behind" class="pill down" :title="`${head.behind} to pull`">
        <ArrowDown :size="11" :stroke-width="2.5" />{{ head.behind }}
      </span>
    </div>

    <div class="actions">
      <button class="btn" :disabled="store.busy" title="Fetch all remotes" @click="git.fetch()">
        <RefreshCw :size="14" /> Fetch
      </button>
      <button class="btn" :disabled="store.busy" title="Pull, stashing local work first" @click="git.pull()">
        <ArrowDownToLine :size="14" /> Pull
      </button>
      <button
        class="btn"
        :disabled="store.busy || !head"
        :title="head?.upstream ? `Push to ${head.upstream}` : 'Push and set the upstream'"
        @click="push"
      >
        <ArrowUpFromLine :size="14" /> Push
      </button>

      <span class="sep" />

      <button class="btn" :disabled="store.busy" @click="showBranch = true">
        <GitBranchPlus :size="14" /> Branch
      </button>
      <button class="btn" :disabled="store.busy" title="Stash everything" @click="git.stashPush()">
        <Archive :size="14" /> Stash
      </button>
    </div>

    <div class="tools">
      <button
        class="btn icon-only"
        :disabled="store.busy || !nextUndo"
        :title="nextUndo ? `Undo ${nextUndo.label}` : 'Nothing to undo'"
        @click="git.undo()"
      >
        <Undo2 :size="15" />
      </button>
      <button
        class="btn icon-only"
        :disabled="store.busy || !nextRedo"
        :title="nextRedo ? `Redo ${nextRedo.label}` : 'Nothing to redo'"
        @click="git.redo()"
      >
        <Redo2 :size="15" />
      </button>
      <button class="btn icon-only" title="History" @click="showHistory = !showHistory">
        <History :size="15" />
      </button>

      <span class="sep" />

      <button class="btn icon-only" title="Settings" @click="config.openSettings('profiles')">
        <Settings :size="15" />
      </button>
      <ProfileMenu />
    </div>

    <!-- Strips below the toolbar, stacked so a conflict and a refused push can
         both be shown. -->
    <div class="banners">
    <!-- A rejected push turns the strip below the toolbar into the next step,
         rather than putting a dialog in the way. Force push gets its own
         second press: nothing rewrites a remote on one click. -->
    <div v-if="blocked" class="banner push" :class="{ danger: confirming }">
      <template v-if="!confirming">
        <TriangleAlert :size="14" />
        <span>
          <span class="mono">{{ blocked.upstream ?? `${blocked.remote}/${blocked.branch}` }}</span>
          has commits yours does not. Push was refused.
        </span>
        <button class="btn tiny" :disabled="store.busy" @click="pullThen(true)">
          Pull with rebase
        </button>
        <button class="btn tiny ghost" :disabled="store.busy" @click="pullThen(false)">
          Pull and merge
        </button>
        <button class="btn tiny ghost" :disabled="store.busy" @click="askForce">
          Force push…
        </button>
        <button class="btn tiny ghost close" @click="git.dismissPushBlock()">Dismiss</button>
      </template>

      <template v-else>
        <TriangleAlert :size="14" />
        <span v-if="checkingForce">Reading what a rewrite would drop…</span>
        <span v-else>
          Force pushing replaces
          <span class="mono">{{ blocked.upstream ?? `${blocked.remote}/${blocked.branch}` }}</span>
          with your branch.
          <template v-if="orphans.length">
            {{ orphans.length }} {{ orphans.length === 1 ? 'commit' : 'commits' }} there
            ({{ orphans.map((c) => c.short).join(', ') }})
            {{ orphans.length === 1 ? 'stops' : 'stop' }} being reachable. Are you sure?
          </template>
          <template v-else>Are you sure?</template>
        </span>
        <button
          class="btn tiny danger-btn"
          :disabled="store.busy || checkingForce"
          @click="forcePush"
        >
          Yes, force push with lease
        </button>
        <button class="btn tiny ghost close" @click="confirming = false">Cancel</button>
      </template>
    </div>

    <!-- Detached HEAD is easy to reach and, until now, had no way back from
         inside the app. Anything committed here belongs to no branch and is
         lost the moment you check something else out, so the strip says so and
         offers both ways out. -->
    <div v-if="store.repo?.detached" class="banner">
      <TriangleAlert :size="14" />
      <span>
        Not on a branch. Commits made here belong to nothing and are lost when
        you check out something else.
      </span>
      <button
        v-if="lastBranch"
        class="btn tiny"
        :disabled="store.busy"
        @click="git.checkout(lastBranch)"
      >
        Back to {{ lastBranch }}
      </button>
      <button class="btn tiny ghost" :disabled="store.busy" @click="showBranch = true">
        Branch from here
      </button>
    </div>

    <div v-if="conflicts" class="banner">
      <TriangleAlert :size="14" />
      <span>
        {{ conflicts }} conflicted {{ conflicts === 1 ? 'file' : 'files' }} to resolve
      </span>
      <button class="btn tiny" @click="store.resolving = store.status?.conflicted[0] ?? ''">
        Resolve
      </button>
      <button class="btn tiny ghost" :disabled="store.busy" @click="git.abortMerge()">
        Abort merge
      </button>
    </div>
    </div>

    <HistoryMenu v-if="showHistory" @close="showHistory = false" />
    <BranchDialog v-if="showBranch" @close="showBranch = false" />
  </header>
</template>

<style scoped>
.bar {
  position: relative;
  /* Three tracks so the action group sits in the true centre of the window,
     whatever the repository name happens to be. */
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: 16px;
  padding: 7px 10px;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
}

.repo {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}

.name {
  white-space: nowrap;
}

.branch {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--accent);
}

.actions {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 1px;
}

.tools {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 1px;
}

.icon-only {
  padding: 5px 7px;
}

.sep {
  width: 1px;
  height: 18px;
  background: var(--line);
  margin: 0 5px;
}

/* A filled pill, so the arrow and the digit have to sit square inside it:
   even padding, one line box, and no nudging the glyph off centre. */
.up,
.down {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px 7px;
  border-radius: 999px;
  font-size: 11px;
  line-height: 1;
  font-variant-numeric: tabular-nums;
}

/* Lucide draws on a padded 24-unit grid, so the arrow carries a sliver of its
   own space on the left; claw it back so the pill looks evenly filled. */
.up svg,
.down svg {
  margin-left: -2px;
}

.up {
  background: rgba(87, 193, 132, 0.16);
  color: var(--green);
}

.down {
  background: rgba(79, 156, 249, 0.16);
  color: var(--accent);
}

.banners {
  position: absolute;
  left: 0;
  right: 0;
  top: 100%;
  z-index: 6;
}

.banner {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 5px 12px;
  font-size: 12px;
  color: #f2bd6e;
  background: #2a2114;
  border-bottom: 1px solid rgba(240, 168, 60, 0.35);
}

.tiny {
  font-size: 11px;
  padding: 2px 8px;
  background: var(--amber);
  color: #1a1206;
  font-weight: 600;
}

.tiny.ghost {
  background: none;
  color: #f2bd6e;
  border: 1px solid rgba(240, 168, 60, 0.4);
}

/* The rejected-push strip. Two states in one place: the offer, then the
   confirmation, which turns red so the second press cannot be mistaken for the
   first. */
.push span:first-of-type {
  min-width: 0;
}

.push .close {
  margin-left: auto;
}

.banner.danger {
  color: #f3a1ad;
  background: #2c1519;
  border-bottom-color: rgba(224, 87, 109, 0.4);
}

.banner.danger .tiny.ghost {
  color: #f3a1ad;
  border-color: rgba(224, 87, 109, 0.45);
}

.danger-btn {
  background: var(--red);
  color: #fff;
  font-weight: 600;
}
</style>
