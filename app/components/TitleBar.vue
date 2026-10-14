<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  Archive,
  ArrowDownToLine,
  ArrowUpFromLine,
  ChevronRight,
  Download,
  GitBranch,
  GitBranchPlus,
  History,
  Package,
  Redo2,
  RefreshCw,
  Settings,
  Sprout,
  GitPullRequestArrow,
  Trash2,
  TriangleAlert,
  Undo2,
  X
} from 'lucide-vue-next'
import { isRunning, useGit, type CommitSummary } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'
import { useShortcuts } from '~/composables/useShortcuts'
import { useUpdates } from '~/composables/useUpdates'
import { useRebase } from '~/composables/useRebase'
import { usePanes } from '~/composables/usePanes'
import { useBranchNaming } from '~/composables/useBranchNaming'
import { leadWithForce, trunkLocalName } from '~/composables/useSync'
import type { BranchDeletion } from '~/composables/useGit'
import { useRewrite } from '~/composables/useRewrite'
import { useForge } from '~/composables/useForge'

const emit = defineEmits<{ leave: [depth: number] }>()
const rewrite = useRewrite()

/** A cherry-pick or revert that stopped, named the way its buttons say it. */
const picking = computed(() => {
  const now = store.progress
  if (!now || now.merging || now.rebasing) return null
  if (now.cherry_picking) return 'cherry-pick'
  if (now.reverting) return 'revert'
  return null
})

const git = useGit()
const store = git.store

/** The open review for the branch checked out, for the badge beside it. */
const forgeCurrent = computed(() => useForge().currentReview.value)

/** The project the trail started from — the tab you are still in. */
const rootName = computed(() => store.inside[0]?.fromName ?? '')
const config = useConfig()
const updates = useUpdates()
const rebase = useRebase()
const { layout } = usePanes()

/**
 * How far the strips reach across the window.
 *
 * They hang under the toolbar over whatever is below, and the right panel's
 * own header — what is staged, what is not — was being covered by them. The
 * strips stop at the panel's edge instead: they are about the repository and
 * the history, which is what they now sit over.
 */
// Over the graph and clear of the inspector, whose own head says what is staged.
const bannerRight = computed(() => `${layout.panel + 1}px`)

/**
 * A release on offer, or on its way in. The button stays through the download
 * so the progress in settings is one click away, and goes with "not now".
 */
const updateOffered = computed(() =>
  ['available', 'downloading', 'ready'].includes(updates.store.stage)
)

const branchNaming = useBranchNaming()
/**
 * The dialog, for when the graph is not there to type into. The graph takes
 * the name inline on the row the branch starts from whenever it is on screen,
 * which is nearly always; a file or a review open in its place leaves the
 * dialog as the way to ask.
 */
const showBranch = ref(false)

function newBranch() {
  if (store.busy) return
  if (!branchNaming.begin()) showBranch.value = true
}
const showHistory = ref(false)

/** The start-work dialog: fetch, then branch from the trunk as the remote has it. */
const starting = ref(false)

function startWork() {
  if (store.busy || !store.repo) return
  starting.value = true
}

/** The trunk's own local name, so the branch you are on can be told apart from it. */
const trunkName = computed(() => trunkLocalName(store.trunk, store.refs?.locals ?? []))

/** Updating from main means something only on a branch that is not main. */
const canUpdate = computed(() => !!head.value && !!trunkName.value && head.value.name !== trunkName.value)

function updateFromMain() {
  if (store.busy || !canUpdate.value || !head.value) return
  return git.updateFromTrunk(head.value.name)
}

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

/** The stash waiting to be confirmed for dropping; null when none is. */
const dropping = ref<number | null>(null)

/** Set while "throw the conflicts away" is being confirmed. */
const clearingConflicts = ref(false)

async function stashAction(what: 'apply' | 'pop') {
  const one = stash.value
  if (!one) return
  const said = what === 'apply' ? await git.stashApply(one.index) : await git.stashPop(one.index)
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

/** See `isRunning`: a merge or a rebase is aborted, not thrown away. */
const running = computed(() => isRunning(store.progress))
const nextUndo = computed(() => store.history.undo[0] ?? null)
const nextRedo = computed(() => store.history.redo[0] ?? null)

/** Push is push: no dialog. If the remote refuses, the strip below offers the
    way out, which is the only moment a choice is actually needed.

    The exceptions are asked in `requestPush`, which every push in the window
    goes through: a branch whose upstream was deleted on the remote — most
    often because its pull request was merged, and pushing it would quietly
    put the deleted branch back — and, when settings say so, a push straight
    to the main branch. */
function push() {
  if (!head.value) return
  return git.requestPush(head.value.name, !head.value.upstream)
}

// --- an upstream that was deleted

/** What deleting the branch would cost, read when the strip opens. */
const goneReading = ref<BranchDeletion | null>(null)

/** The strip stands only while the branch it asks about still has a gone upstream. */
const gone = computed(() => {
  const asked = store.pushGone
  if (!asked) return null
  const local = store.refs?.locals.find((one) => one.name === asked.branch)
  return local?.gone ? local : null
})

watch(
  () => store.pushGone?.branch ?? null,
  async (name) => {
    goneReading.value = null
    if (!name) return
    const reading = await git.deleteBranchPreview(name)
    if (store.pushGone?.branch === name) goneReading.value = reading
  },
  { immediate: true }
)

/** Its work is on the trunk, by history or by patch, so it can go. */
const goneLanded = computed(() => !!goneReading.value?.trunk_holds)

/** Nothing open to carry across: switching away is a plain switch. */
const clean = computed(
  () =>
    !store.status ||
    (!store.status.staged.length && !store.status.unstaged.length && !store.status.conflicted.length)
)

/** Whether finishing the branch means stepping off it first. */
const goneIsHead = computed(() => !!gone.value && gone.value.name === store.repo?.head)

/** Steps onto the trunk if need be and deletes the finished branch, with Undo on the notice. */
async function switchAndDelete() {
  const branch = gone.value?.name
  // Where it pointed when the strip asked: if it moves on, it is not deleted.
  const oid = gone.value?.oid
  const trunk = trunkName.value
  store.pushGone = null
  if (!branch || !oid || !trunk) return
  if (store.repo?.head === branch) {
    await git.checkout(trunk)
    if (store.repo?.head !== trunk) return
  }
  await git.deleteBranches([{ name: branch, oid }])
}

/** Pushes on purpose, putting the remote branch back under its old name. */
async function recreate() {
  const branch = gone.value?.name
  store.pushGone = null
  if (branch) await git.requestPush(branch, false, true)
}

// The toolbar's own actions, bound to the keyboard. It is mounted for as long
// as a repository is open, which is exactly as long as these should fire.
useShortcuts({
  'repo.fetch': () => !store.busy && void git.fetch(),
  'repo.pull': () => !store.busy && void git.pull(),
  'repo.push': () => !store.busy && void push(),
  'repo.refresh': () => !store.busy && void git.refresh(),
  'repo.settings': () => config.openSettings('profiles'),
  'branch.create': () => newBranch(),
  'branch.start': () => startWork(),
  'branch.update': () => !store.busy && void updateFromMain(),
  'stash.push': () => !store.busy && void git.stashPush(),
  'history.undo': () => !store.busy && nextUndo.value && void git.undo(),
  'history.redo': () => !store.busy && nextRedo.value && void git.redo()
})

// --- a refused push
const blocked = computed(() => store.pushBlocked)
const confirming = ref(false)
const checkingForce = ref(false)
const orphans = ref<CommitSummary[]>([])
/** The upstream commit the force-push preview showed; the push leases on it. */
const lease = ref<string | null>(null)

/**
 * Pull the refused branch, and if that leaves it pushable, push straight away.
 *
 * The branch the push was for, not whichever one is checked out: a push from
 * the sidebar can be refused for a branch you are not standing on, and the
 * backend brings any branch up to date.
 */
async function pullThen(rebase: boolean) {
  // Read the target first: dismissing clears it.
  const target = blockedTarget()
  git.dismissPushBlock()
  if (!target) return
  const outcome = await git.pullBranch(target.branch, rebase)
  if (!outcome?.ok) {
    if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0] ?? null
    return
  }
  await git.push(target.remote, target.branch, false, false)
}

/**
 * Asks before rewriting, and names what would go: the count comes from a fresh
 * preview rather than from whatever the last dialog happened to read. The
 * upstream commit that preview saw is what the push is then leased on, so
 * what the user confirmed is exactly what the push may replace.
 */
async function askForce() {
  confirming.value = true
  checkingForce.value = true
  orphans.value = []
  lease.value = null
  const preview = await git.pushPreview(store.pushBlocked?.branch, true)
  orphans.value = preview?.will_orphan ?? []
  lease.value = preview?.upstream_oid ?? null
  checkingForce.value = false
}

/** Lead with forcing when the commits it would drop are older copies of ours. */
const forceFirst = computed(() => leadWithForce(store.pushBlocked))

async function forcePush() {
  const target = blockedTarget()
  // Never without a lease: with no upstream commit read, `--force-with-lease`
  // would lease on whatever the last fetch happened to leave, which is not
  // what anyone confirmed.
  if (!target || !lease.value) return
  confirming.value = false
  await git.push(target.remote, target.branch, true, false, lease.value)
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
    lease.value = null
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
      <div class="where">
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
      </div>
      <!-- The branch you are on, under the repository's name: the one fact
           about it that changes under you all day. -->
      <span class="branch" :class="{ detached: store.repo?.detached }" :title="store.repo?.head">
        <GitBranch :size="12" class="branch-icon" />
        <span class="branch-name">{{ store.repo?.head }}</span>
        <span v-if="store.repo?.detached" class="pill">detached</span>
        <ReviewBadge v-if="forgeCurrent" :review="forgeCurrent" />
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
        class="btn"
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
        <GitBranchPlus :size="14" /> Branch
      </button>

      <span class="sep" />

      <button
        class="btn"
        :disabled="store.busy"
        title="Throw the stash away"
        @click="dropping = stash.index"
      >
        <Trash2 :size="14" /> Drop
      </button>
    </div>

    <div v-else class="actions">
      <button class="btn" :disabled="store.busy" title="Fetch all remotes" @click="git.fetch()">
        <RefreshCw :size="14" /> Fetch
      </button>
      <button
        class="btn"
        :disabled="store.busy"
        :title="head?.behind ? `Pull ${head.behind} from upstream, carrying local work along` : 'Pull, carrying local work along'"
        @click="git.pull()"
      >
        <ArrowDownToLine :size="14" /> Pull
        <!-- The counts live on the buttons that act on them: what there is to
             pull sits on Pull, what there is to push on Push, as a badge on
             the button's icon. -->
        <span v-if="head?.behind" class="count down">{{ head.behind }}</span>
      </button>
      <button
        class="btn"
        :disabled="store.busy || !head"
        :title="head?.upstream ? `Push to ${head.push_to ?? head.upstream}` : head?.gone ? `${head.gone} was deleted on the remote` : 'Push and set the upstream'"
        @click="push"
      >
        <ArrowUpFromLine :size="14" /> Push
        <span v-if="head?.ahead" class="count up">{{ head.ahead }}</span>
      </button>

      <span class="sep" />

      <button class="btn" :disabled="store.busy" title="Start a branch here" @click="newBranch">
        <GitBranchPlus :size="14" /> Branch
      </button>
      <button
        class="btn"
        :disabled="store.busy"
        title="Fetch, then start a new branch from main as the remote has it"
        @click="startWork"
      >
        <Sprout :size="14" /> Start
      </button>
      <button
        v-if="canUpdate"
        class="btn"
        :disabled="store.busy"
        :title="`Fetch, then bring ${head?.name} up to date with main`"
        @click="updateFromMain"
      >
        <GitPullRequestArrow :size="14" /> Update
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
    <div class="banners" :style="{ right: bannerRight }">
    <!-- A rejected push turns the strip below the toolbar into the next step,
         rather than putting a dialog in the way. Force push gets its own
         second press: nothing rewrites a remote on one click. -->
    <div v-if="blocked" class="banner push" :class="{ danger: confirming }">
      <template v-if="!confirming && forceFirst">
        <TriangleAlert :size="14" />
        <span>
          <span class="mono">{{ blocked.upstream ?? `${blocked.remote}/${blocked.branch}` }}</span>
          still has the commits you rewrote here. Pulling would bring the old copies back beside
          the new ones.
        </span>
        <button class="btn tiny" :disabled="store.busy" @click="askForce">
          Force push with lease…
        </button>
        <button class="btn tiny ghost" :disabled="store.busy" @click="pullThen(true)">
          Pull with rebase
        </button>
        <button class="btn tiny ghost" :disabled="store.busy" @click="pullThen(false)">
          Pull and merge
        </button>
        <button class="btn tiny ghost close" @click="git.dismissPushBlock()">Dismiss</button>
      </template>

      <template v-else-if="!confirming">
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
        <span v-else-if="!lease">
          Could not read what
          <span class="mono">{{ blocked.upstream ?? `${blocked.remote}/${blocked.branch}` }}</span>
          holds, so there is nothing to lease the force push on. Fetch, then try again.
        </span>
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
          :disabled="store.busy || checkingForce || !lease"
          @click="forcePush"
        >
          Yes, force push with lease
        </button>
        <button class="btn tiny ghost close" @click="confirming = false">Cancel</button>
      </template>
    </div>

    <!-- The branch's remote copy was deleted — usually its pull request was
         merged. A plain push would put it back, so this asks what was meant. -->
    <div v-if="gone" class="banner push gone-strip">
      <TriangleAlert :size="14" />
      <span>
        <span class="mono">{{ gone.gone }}</span> was deleted on the remote.
        <template v-if="!goneReading">Checking whether its work landed…</template>
        <template v-else-if="goneLanded">
          Its work is on <span class="mono">{{ goneReading.against }}</span>, so
          <span class="mono">{{ gone.name }}</span> is finished.
        </template>
        <template v-else>Its work is not on the main branch.</template>
      </span>
      <button
        v-if="goneLanded && trunkName"
        class="btn tiny"
        :disabled="store.busy || (goneIsHead && !clean)"
        :title="goneIsHead && !clean ? 'Commit or stash your open changes first' : ''"
        @click="switchAndDelete"
      >
        <template v-if="goneIsHead">Switch to {{ trunkName }} and delete {{ gone.name }}</template>
        <template v-else>Delete {{ gone.name }}</template>
      </button>
      <button
        class="btn tiny"
        :class="{ ghost: goneLanded }"
        :disabled="store.busy"
        @click="recreate"
      >
        Push to recreate {{ gone.gone }}
      </button>
      <button class="btn tiny ghost close" @click="store.pushGone = null">Dismiss</button>
    </div>

    <!-- A push straight to the main branch, when settings say to ask first. -->
    <div v-if="store.pushConfirm" class="banner push confirm-strip">
      <TriangleAlert :size="14" />
      <span>{{ store.pushConfirm.reason }}</span>
      <button class="btn tiny" :disabled="store.busy" @click="git.confirmPush(true)">
        Push anyway
      </button>
      <button class="btn tiny ghost close" @click="git.confirmPush(false)">Cancel</button>
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
      <button class="btn tiny ghost" :disabled="store.busy" @click="newBranch">
        Branch from here
      </button>
    </div>

    <!-- A rebase that is part-way through, whether or not the plan is still
         on screen: closing the pane must not be a way to lose the thread. The
         conflict strip below covers what to do about the files themselves. -->
    <!-- A split stops with the commit's changes unstaged: the ordinary
         staging panel is where the pieces are made, and this is the way on. -->
    <div
      v-if="store.progress?.rebasing && rebase.store.progress?.splitting && !rebase.store.open"
      class="banner"
      data-test="split-banner"
    >
      <TriangleAlert :size="14" />
      <span v-if="rewrite.store.splitLeft.length">
        {{ rewrite.store.splitLeft.length }}
        {{ rewrite.store.splitLeft.length === 1 ? 'file' : 'files' }} of the commit
        {{ rewrite.store.splitLeft.length === 1 ? 'is' : 'are' }} not committed yet:
        <span class="mono">{{ rewrite.store.splitLeft.join(', ') }}</span>
      </span>
      <span v-else>
        Splitting <span class="mono">{{ rebase.store.progress.splitting }}</span>: commit pieces,
        then Continue
      </span>
      <template v-if="rewrite.store.splitLeft.length">
        <button
          class="btn tiny"
          :disabled="store.busy"
          data-test="split-commit-rest"
          @click="rewrite.splitContinue('commit')"
        >
          Commit the rest as one more piece
        </button>
        <button
          class="btn tiny ghost"
          :disabled="store.busy"
          title="For changes you discarded on purpose"
          @click="rewrite.splitContinue('leave')"
        >
          Go on without it
        </button>
      </template>
      <button
        v-else
        class="btn tiny"
        :disabled="store.busy || !!conflicts"
        data-test="split-continue"
        @click="rewrite.splitContinue('ask')"
      >
        Continue
      </button>
      <button
        class="btn tiny ghost close"
        :disabled="store.busy"
        title="Puts the branch back as it was. Anything you changed during the split goes to the stash first."
        data-test="split-abort"
        @click="rebase.abort()"
      >
        Abort the split
      </button>
    </div>

    <!-- A run of picks or reverts with nothing left to resolve: committing the
         stopped one is not the end of it, the rest of the list is waiting. -->
    <div
      v-if="store.progress?.sequencer && !conflicts"
      class="banner"
      data-test="sequence-banner"
    >
      <TriangleAlert :size="14" />
      <span>
        {{ store.progress.sequencer === 'revert' ? 'Reverting' : 'Cherry-picking' }} —
        {{ store.progress.sequence_left ?? 0 }}
        {{ (store.progress.sequence_left ?? 0) === 1 ? 'commit' : 'commits' }} still to apply
      </span>
      <button class="btn tiny" :disabled="store.busy" @click="rewrite.sequenceContinue()">
        Continue
      </button>
      <button class="btn tiny ghost" :disabled="store.busy" @click="rewrite.sequenceSkip()">
        Skip this commit
      </button>
      <button class="btn tiny ghost close" :disabled="store.busy" @click="rewrite.sequenceAbort()">
        Abort the {{ store.progress.sequencer }}
      </button>
    </div>

    <div
      v-if="store.progress?.rebasing && !rebase.store.progress?.splitting && !rebase.store.open"
      class="banner"
    >
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
        not fit back on. They are still in the stash.
      </span>
      <span v-else-if="store.progress?.applied_stash">
        {{ conflicts }} conflicted {{ conflicts === 1 ? 'file' : 'files' }}: the stash would not go
        on as it is. It is still on the list.
      </span>
      <span v-else>
        {{ conflicts }} conflicted {{ conflicts === 1 ? 'file' : 'files' }} to resolve
      </span>
      <button class="btn tiny" @click="store.resolving = store.status?.conflicted[0] ?? ''">
        Resolve
      </button>
      <!-- The way out when the answer is "none of this": the files go back to
           what the branch had. Only where there is nothing to abort — a merge
           or a rebase has its own way out, below, and aborting it puts the
           whole thing back rather than just the files. -->
      <button
        v-if="!running"
        class="btn tiny ghost"
        :disabled="store.busy"
        title="Take the files back to what the branch already had"
        @click="clearingConflicts = true"
      >
        Throw them away
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
        Put it back
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
      <button
        v-else-if="picking"
        class="btn tiny ghost"
        :disabled="store.busy"
        data-test="abort-pick"
        @click="rewrite.sequenceAbort()"
      >
        Abort {{ picking }}
      </button>
    </div>
    </div>

    <HistoryMenu v-if="showHistory" @close="showHistory = false" />
    <BranchDialog v-if="showBranch" @close="showBranch = false" />
    <BranchDialog v-if="starting" fresh @close="starting = false" />
    <DropStashDialog v-if="dropping !== null" :index="dropping" @close="dropping = null" />

    <DiscardConflictsDialog
      v-if="clearingConflicts"
      :paths="store.status?.conflicted ?? []"
      @close="clearingConflicts = false"
    />

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
/*
 * The toolbar is chrome, the tone of the tab strip's open tab above it. The
 * repository and branch on the left, the everyday actions in the middle as
 * icons with their names under them — the shape every git client has taught
 * people to look for — and the window's own tools on the right.
 */
.bar {
  position: relative;
  /* Three tracks so the action group sits in the true centre of the window,
     whatever the repository name happens to be. */
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: 16px;
  min-height: 52px;
  padding: 4px 10px 4px 14px;
  background: var(--canvas);
  border-bottom: 1px solid var(--line);
}

/* Name over branch, two lines that read as one label. */
.repo {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 1px;
  min-width: 0;
}

.where {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
}

.name {
  font-size: 13.5px;
  font-weight: 650;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* The trail. The last step is where you are and carries the way out; the ones
   before it are buttons back to themselves. */
.crumb {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 1px 5px;
  margin-left: -5px;
  border-radius: var(--radius-sm);
  font-size: 13px;
  color: var(--text-dim);
  white-space: nowrap;
}

.crumb + .sep + .crumb {
  margin-left: 0;
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
  font-weight: 650;
}

.sep {
  flex: none;
}

.out {
  display: flex;
  padding: 2px;
  border-radius: var(--radius-sm);
  color: var(--text-faint);
}

.out:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.branch {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  font-size: 12px;
  color: var(--text-dim);
}

.branch-icon {
  flex: none;
  color: var(--accent);
}

.branch-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.branch.detached .branch-icon {
  color: var(--amber);
}

/* The everyday actions: a larger icon with its name under it. */
.actions {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 2px;
}

.actions .btn {
  position: relative;
  flex-direction: column;
  gap: 2px;
  min-width: 54px;
  min-height: 0;
  padding: 5px 8px 4px;
  font-size: 11px;
  font-weight: 500;
  line-height: 1.2;
  color: var(--text-dim);
}

.actions .btn svg {
  width: 18px;
  height: 18px;
  color: var(--text);
  stroke-width: 1.75;
}

.actions .btn:hover:not(:disabled) {
  color: var(--text);
}

.tools {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 2px;
}

.icon-only {
  width: 30px;
  padding: 0;
}

/* A new version: tinted rather than filled, news and not an alarm. */
.update {
  margin-right: 4px;
  background: var(--primary-bg);
  color: var(--accent-soft);
  font-weight: 600;
}

.update:hover:not(:disabled) {
  background: var(--primary-line);
  color: var(--accent-soft);
}

.profile {
  margin-left: 4px;
}

.sep {
  width: 1px;
  height: 22px;
  background: var(--line);
  margin: 0 6px;
}

.where .sep {
  width: auto;
  height: auto;
  margin: 0;
  background: none;
}

/* How many there are to pull or push, as a badge on the button's icon. */
.count {
  position: absolute;
  top: 1px;
  left: calc(50% + 6px);
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--radius-pill);
  border: 2px solid var(--canvas);
  box-sizing: content-box;
  font-size: 9.5px;
  font-weight: 700;
  line-height: 16px;
  text-align: center;
  font-variant-numeric: tabular-nums;
  background: var(--accent);
  color: var(--on-accent);
}

/* Under the toolbar, over the graph — but never over the inspector, whose
   own head says what is staged. The right edge is set from its width. */
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
  flex-wrap: wrap;
  gap: 8px;
  padding: 6px 12px;
  font-size: 12px;
  color: var(--warning-soft);
  background: var(--warning-bg);
  border-bottom: 1px solid var(--warning-line);
}

.tiny {
  min-height: 22px;
  font-size: 11.5px;
  padding: 1px 8px;
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--warning-line);
  font-weight: 600;
}

.banner .tiny:hover:not(:disabled) {
  background: var(--bg);
  color: var(--text);
  border-color: currentColor;
}

.tiny.ghost {
  background: transparent;
  color: inherit;
  border-color: transparent;
  font-weight: 500;
}

.banner .tiny.ghost:hover:not(:disabled) {
  background: color-mix(in srgb, currentColor 12%, transparent);
  color: inherit;
  border-color: transparent;
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
  color: var(--danger-soft);
  background: var(--danger-bg);
  border-bottom-color: var(--danger-line);
}

.banner .danger-btn,
.banner .danger-btn:hover:not(:disabled) {
  background: var(--red);
  color: var(--on-danger);
  border-color: transparent;
}
</style>
