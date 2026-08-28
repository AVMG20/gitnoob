<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  Archive,
  ArrowDown,
  ArrowDownToLine,
  ArrowUp,
  ArrowUpFromLine,
  ChevronRight,
  Download,
  GitBranchPlus,
  History,
  Package,
  Redo2,
  RefreshCw,
  Settings,
  Trash2,
  TriangleAlert,
  Undo2,
  X
} from 'lucide-vue-next'
import { useGit, type CommitSummary } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'
import { useShortcuts } from '~/composables/useShortcuts'
import { useUpdates } from '~/composables/useUpdates'
import { useRebase } from '~/composables/useRebase'

const emit = defineEmits<{ leave: [depth: number] }>()

const git = useGit()
const store = git.store

/** The project the trail started from — the tab you are still in. */
const rootName = computed(() => store.inside[0]?.fromName ?? '')
const config = useConfig()
const updates = useUpdates()
const rebase = useRebase()

/**
 * A release on offer, or on its way in. The button stays through the download
 * so the progress in settings is one click away, and goes with "not now".
 */
const updateOffered = computed(() =>
  ['available', 'downloading', 'ready'].includes(updates.store.stage)
)

const showBranch = ref(false)
const showHistory = ref(false)

/**
 * The stash the selected row is, when it is one.
 *
 * Read off the selection rather than the loaded detail, so the bar changes in
 * the same frame as the click rather than a round trip later.
 */
const stash = computed(
  () => store.stashes.find((one) => one.oid === store.selected) ?? null
)

/** Naming the branch a stash becomes; null when nothing is being named. */
const naming = ref(false)

async function stashAction(what: 'apply' | 'pop' | 'drop') {
  const one = stash.value
  if (!one) return
  const said =
    what === 'apply'
      ? await git.stashApply(one.index)
      : what === 'pop'
        ? await git.stashPop(one.index)
        : await git.stashDrop(one.index)
  if (said !== null) git.note(said)
}

function branchFromStash() {
  naming.value = true
}

async function makeStashBranch(name: string) {
  const one = stash.value
  naming.value = false
  if (!one || !name.trim()) return
  const said = await git.stashBranch(one.index, name.trim())
  if (said !== null) git.note(said)
}

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

// The toolbar's own actions, bound to the keyboard. It is mounted for as long
// as a repository is open, which is exactly as long as these should fire.
useShortcuts({
  'repo.fetch': () => !store.busy && void git.fetch(),
  'repo.pull': () => !store.busy && void git.pull(),
  'repo.push': () => !store.busy && void push(),
  'repo.refresh': () => !store.busy && void git.refresh(),
  'repo.settings': () => config.openSettings('profiles'),
  'branch.create': () => {
    if (!store.busy) showBranch.value = true
  },
  'stash.push': () => !store.busy && void git.stashPush(),
  'history.undo': () => !store.busy && nextUndo.value && void git.undo(),
  'history.redo': () => !store.busy && nextRedo.value && void git.redo()
})

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

// --- a diverged checkout

/**
 * The question a checkout left open, for as long as it is still a question.
 *
 * The strip only stands while the user is on that branch and the branch is
 * still both ahead and behind: a pull, a push or a switch done any other way
 * answers it without these buttons, and a strip that outlives its question
 * offers a rebase nobody asked for. The counts come from the live refs rather
 * than from the moment of the checkout, for the same reason.
 */
const diverged = computed(() => {
  const asked = store.divergedCheckout
  if (!asked) return null
  const local = store.refs?.locals.find((b) => b.name === asked.branch)
  if (!local?.is_head) return null
  if (!(local.ahead > 0 && local.behind > 0)) return null
  return { ...asked, ahead: local.ahead, behind: local.behind }
})

/** Joins the two histories the way the button says, and opens the resolver
    if that runs into conflicts. */
async function reconcile(rebase: boolean) {
  const target = store.divergedCheckout?.upstream
  git.dismissDiverged()
  if (!target) return
  const outcome = rebase ? await git.rebase(target) : await git.merge(target, false)
  if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0] ?? null
}
</script>

<template>
  <header class="bar">
    <div class="repo">
      <!-- The trail into a submodule. The project it belongs to is still what
           the tab says, so this is where being somewhere else has to be
           visible — and each step is its own way back out. -->
      <template v-if="store.inside.length">
        <button
          class="crumb root"
          :title="`Back to ${rootName}`"
          @click="emit('leave', 0)"
        >
          {{ rootName }}
        </button>
        <template v-for="(step, at) in store.inside" :key="step.path">
          <ChevronRight :size="12" class="faint sep" />
          <button
            v-if="at < store.inside.length - 1"
            class="crumb"
            :title="`Back to ${step.name}`"
            @click="emit('leave', at + 1)"
          >
            {{ step.name }}
          </button>
          <span v-else class="crumb here" :title="step.path">
            <Package :size="12" />
            {{ step.name }}
            <button class="out" title="Leave the submodule" @click="emit('leave', 0)">
              <X :size="12" />
            </button>
          </span>
        </template>
      </template>
      <strong v-else class="name">{{ store.repo?.name }}</strong>
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

    <!-- What the bar offers follows what is selected. A stash is the one kind
         of row whose actions are nothing like a commit's, so while one is
         picked the bar is about the stash. Fetch, pull and push keep their
         keys — this component is still mounted, only its buttons change. -->
    <div v-if="stash" class="actions stash-actions">
      <button
        class="btn"
        :disabled="store.busy"
        title="Put these changes back and keep the stash"
        @click="stashAction('apply')"
      >
        <ArrowDownToLine :size="14" /> Apply
      </button>
      <button
        class="btn strong"
        :disabled="store.busy"
        title="Put these changes back and take the stash off the list"
        @click="stashAction('pop')"
      >
        <ArrowDownToLine :size="14" /> Pop
      </button>
      <button
        class="btn"
        :disabled="store.busy"
        title="Start a branch from it, for a stash that will not go on here"
        @click="branchFromStash"
      >
        <GitBranchPlus :size="14" /> Branch from it
      </button>

      <span class="sep" />

      <button
        class="btn danger-text"
        :disabled="store.busy"
        title="Throw the stash away"
        @click="stashAction('drop')"
      >
        <Trash2 :size="14" /> Drop
      </button>
    </div>

    <div v-else class="actions">
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

      <button
        v-if="updateOffered"
        class="btn update"
        :title="`Version ${updates.store.version} is available`"
        @click="config.openSettings('updates')"
      >
        <Download :size="14" />
        {{ updates.store.stage === 'available' ? 'Update' : 'Updating…' }}
      </button>
      <button class="btn icon-only" title="Settings" @click="config.openSettings('profiles')">
        <Settings :size="15" />
      </button>
      <ProfileMenu class="profile" />
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

    <!-- Checking out a remote branch found the local one had gone its own way
         while the remote moved on too. The switch already happened; this is
         the question settings said to ask: what to do about the two
         histories. Same shape as a refused push — the strip is the next step,
         not a dialog in the way. -->
    <div v-if="diverged" class="banner push">
      <TriangleAlert :size="14" />
      <span>
        <span class="mono">{{ diverged.branch }}</span> and
        <span class="mono">{{ diverged.upstream }}</span> have diverged:
        {{ diverged.ahead }} {{ diverged.ahead === 1 ? 'commit' : 'commits' }} of yours,
        {{ diverged.behind }} of theirs.
      </span>
      <button
        class="btn tiny"
        :disabled="store.busy"
        title="Replays your commits on top of the remote's — the usual answer"
        @click="reconcile(true)"
      >
        Rebase mine on top
      </button>
      <button
        class="btn tiny ghost"
        :disabled="store.busy"
        title="Ties the two histories together with a merge commit"
        @click="reconcile(false)"
      >
        Merge theirs in
      </button>
      <button class="btn tiny ghost close" @click="git.dismissDiverged()">Leave it</button>
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

    <!-- A rebase that is part-way through, whether or not the plan is still
         on screen: closing the pane must not be a way to lose the thread. The
         conflict strip below covers what to do about the files themselves. -->
    <div v-if="store.progress?.rebasing && !rebase.store.open" class="banner">
      <TriangleAlert :size="14" />
      <span v-if="rebase.store.progress">
        Rebasing — {{ rebase.store.progress.at }} of {{ rebase.store.progress.total }}, stopped at
        <span class="mono">{{ rebase.store.progress.summary }}</span>
      </span>
      <span v-else>A rebase is part-way through.</span>
      <button class="btn tiny" @click="rebase.store.open = true">Show the plan</button>
      <button
        v-if="!conflicts"
        class="btn tiny ghost"
        :disabled="store.busy"
        @click="rebase.resume()"
      >
        Carry on
      </button>
      <button class="btn tiny ghost close" :disabled="store.busy" @click="rebase.abort()">
        Abort the rebase
      </button>
    </div>

    <div v-if="conflicts" class="banner">
      <TriangleAlert :size="14" />
      <span v-if="store.progress?.restoring">
        {{ conflicts }} conflicted {{ conflicts === 1 ? 'file' : 'files' }}: your own changes did
        not fit on this branch. They are still in the stash.
      </span>
      <span v-else>
        {{ conflicts }} conflicted {{ conflicts === 1 ? 'file' : 'files' }} to resolve
      </span>
      <button class="btn tiny" @click="store.resolving = store.status?.conflicted[0] ?? ''">
        Resolve
      </button>
      <!-- The way out is whatever git is actually part-way through. Offering
           "abort merge" for a stash that would not go back on is how you get
           told there is no merge to abort. -->
      <button
        v-if="store.progress?.restoring"
        class="btn tiny ghost"
        :disabled="store.busy"
        @click="git.undoRestore()"
      >
        Undo the switch
      </button>
      <button
        v-else-if="store.progress?.rebasing"
        class="btn tiny ghost"
        :disabled="store.busy"
        @click="git.abortRebase()"
      >
        Abort rebase
      </button>
      <button
        v-else-if="store.progress?.merging"
        class="btn tiny ghost"
        :disabled="store.busy"
        @click="git.abortMerge()"
      >
        Abort merge
      </button>
    </div>
    </div>

    <HistoryMenu v-if="showHistory" @close="showHistory = false" />
    <BranchDialog v-if="showBranch" @close="showBranch = false" />
    <PromptDialog
      v-if="naming"
      title="Branch from stash"
      label="Branch name"
      confirm="Create branch"
      hint="Applies the stash onto a new branch and takes it off the list."
      @close="naming = false"
      @submit="makeStashBranch"
    />
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

/* The trail. The last step is where you are and carries the way out; the ones
   before it are buttons back to themselves. */
.crumb {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 6px;
  border-radius: 5px;
  font-size: 12px;
  color: var(--text-dim);
  white-space: nowrap;
}

.crumb.root,
.crumb:not(.here) {
  font-weight: 600;
}

.crumb:not(.here):hover {
  background: var(--bg-hover);
  color: var(--text);
}

.crumb.here {
  color: var(--text);
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  font-family: var(--mono);
  font-size: 11px;
}

.sep {
  flex: none;
}

.out {
  display: flex;
  margin: 0 -3px 0 1px;
  padding: 1px;
  border-radius: 3px;
  color: var(--text-faint);
}

.out:hover {
  background: var(--bg-active);
  color: var(--text);
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

/* Tinted rather than filled: news, not an alarm, and the pull and push
   buttons beside it should still be the ones the eye lands on. */
.update {
  margin-right: 3px;
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
  font-weight: 600;
}

.update:hover:not(:disabled) {
  background: color-mix(in srgb, var(--accent) 26%, transparent);
  color: var(--accent);
}

/* The profile pill is bordered where the icons around it are not, so the 1px
   gap the group uses reads as the settings icon touching it. */
.profile {
  margin-left: 6px;
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
  background: var(--success-bg);
  color: var(--green);
}

.down {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
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
  border-bottom: 1px solid var(--warning-line);
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
  border: 1px solid var(--warning-line);
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
  border-bottom-color: var(--danger-line);
}

.banner.danger .tiny.ghost {
  color: #f3a1ad;
  border-color: var(--danger-line);
}

.danger-btn {
  background: var(--red);
  color: #fff;
  font-weight: 600;
}

/* --- the bar while a stash is picked */

/* The colour is the whole of it. Which stash was picked is not said here: you
   have just clicked the row, and the row is still marked. */
.stash-actions .btn {
  color: var(--amber-soft);
}

.stash-actions .btn:hover:not(:disabled) {
  background: color-mix(in srgb, var(--amber) 18%, transparent);
  color: var(--amber-soft);
}

.stash-actions .strong {
  font-weight: 600;
  color: var(--text);
}

.stash-actions .danger-text {
  color: var(--red-soft);
}

.stash-actions .danger-text:hover:not(:disabled) {
  background: var(--danger-bg);
  color: var(--red-soft);
}
</style>