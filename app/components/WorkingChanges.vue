<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  Archive,
  ArrowUpFromLine,
  Ban,
  Check,
  Copy,
  EyeOff,
  FolderOpen,
  Minus,
  Plus,
  RotateCcw,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Trash2,
  TriangleAlert,
  Undo2
} from 'lucide-vue-next'
import { copyText, isRunning, useGit, type GuardFinding } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useDragDrop } from '~/composables/useDragDrop'
import { useAi } from '~/composables/useAi'
import { CONFLICTED, useFileView } from '~/composables/useFileView'
import { COMMIT_FLOW_DEFAULTS, useConfig } from '~/composables/useConfig'
import { useCommitDraft } from '~/composables/useCommitDraft'
import { composeMessage, onTrunk, onlyPrefix, ticketPrefix } from '~/composables/useCommitFlow'
import { keyLabel, keysFor, matchesShortcut, useShortcuts } from '~/composables/useShortcuts'

const git = useGit()
const store = git.store
const menu = useContextMenu()
const drag = useDragDrop()
const ai = useAi()
const view = useFileView()
const config = useConfig()

/**
 * The key, short enough to sit on one line. An ssh key is configured as a
 * path and the tail of it is the part that identifies it; a gpg key id is
 * already short.
 */
const signingKey = computed(() => {
  const key = store.signing?.key
  if (!key) return ''
  const tail = key.split(/[/\\]/).pop() ?? key
  return tail.length > 28 ? `…${tail.slice(-27)}` : tail
})

const selected = computed(() =>
  store.viewer && !store.viewer.commit
    ? { path: store.viewer.path, side: store.viewer.side ?? 'unstaged' }
    : null
)

/**
 * What is being written, kept per repository and branch outside this
 * component — see `useCommitDraft`. The panel is taken down whenever a commit
 * is looked at instead, and the message and the amend tick used to go with it.
 */
const draft = useCommitDraft(() => ({
  repo: store.repo?.path ?? null,
  branch: store.repo?.head ?? null
}))
const message = draft.message
const amend = draft.amend
/** What the user had typed before ticking amend, so unticking restores it. */
const stashedMessage = draft.stashed
const amendPushed = draft.pushed

const flow = computed(() => ({ ...COMMIT_FLOW_DEFAULTS, ...(config.settings.value?.commit ?? {}) }))

/** `ABC-123: ` when the branch is named after a ticket and the setting is on. */
const prefix = computed(() =>
  flow.value.ticket_prefix && !store.repo?.detached ? ticketPrefix(store.repo?.head) : null
)

/** Nothing typed but the prefix that was put there: as good as empty. */
const blankMessage = computed(() => onlyPrefix(message.value, prefix.value))

/** Starts an empty box with the ticket, the first time it is seen empty. */
function prefill() {
  if (prefix.value && !message.value.trim() && !amend.value) message.value = prefix.value
}
watch(() => draft.key.value, prefill, { immediate: true })

/** The first line is the commit subject, the way git reads it. */
const subject = computed(() => message.value.split('\n')[0]?.trim() ?? '')
/** Git's own soft limit; past it `git log --oneline` starts truncating. */
const SUBJECT_LIMIT = 72

const staged = computed(() => store.status?.staged ?? [])
const unstaged = computed(() => store.status?.unstaged ?? [])
const conflicted = computed(() => store.status?.conflicted ?? [])

/**
 * What the unstaged list shows: the changes, and the conflicts among them.
 *
 * A conflicted file is a change to the working tree like any other — it is
 * only one git will not let you stage yet. Kept in a list of its own it sat
 * above the panel in a strip with no heading, in a tree the rest of the panel
 * was not using. Here it is a row where the file actually lives, marked.
 */
const unstagedRows = computed(() => [
  ...conflicted.value.map((path) => ({ path, kind: CONFLICTED })),
  ...unstaged.value.filter((entry) => !conflicted.value.includes(entry.path))
])

const hasMessage = computed(() => subject.value.length > 0 && !blankMessage.value)

/**
 * Nothing staged, but changes to commit: the button offers to stage the lot
 * first rather than sitting there disabled. Said in full on the button, new
 * files counted apart, since they are the ones nobody remembers making.
 */
const stageAllFirst = computed(
  () => !staged.value.length && !amend.value && unstaged.value.length > 0
)
const sweep = computed(() => {
  const fresh = unstaged.value.filter((entry) => entry.kind === 'untracked').length
  return { changed: unstaged.value.length - fresh, fresh }
})

const canCommit = computed(
  () =>
    (staged.value.length > 0 || amend.value || stageAllFirst.value) &&
    hasMessage.value &&
    !conflicted.value.length
)

/**
 * Ticking amend loads HEAD's message; unticking gives back what was typed.
 *
 * The message is read from the repository the box was showing when amend was
 * ticked, and it goes into that draft — written by key, not through the box,
 * since the tab in front may be another one by the time it arrives.
 */
async function toggleAmend(on: boolean) {
  if (on === amend.value) return
  amend.value = on
  if (on) {
    const at = draft.key.value
    stashedMessage.value = message.value
    const read = await git.amendDraft()
    if (read) {
      draft.updateAt(at, {
        message: read.body ? `${read.summary}\n\n${read.body}` : read.summary,
        pushed: read.is_pushed
      })
    } else {
      // No commit to amend: put the box back the way it was.
      draft.updateAt(at, { amend: false, message: draft.read(at).stashed ?? '', stashed: null })
    }
  } else {
    message.value = stashedMessage.value ?? ''
    stashedMessage.value = null
    amendPushed.value = false
    prefill()
  }
}
/**
 * The commit box is pinned to the bottom and the two file lists share what is
 * left equally. The diff only claims space once a file is actually open.
 */
const rows = 'minmax(46px, 1fr) minmax(46px, 1fr) auto'

/**
 * Fills the box with the message git already wrote, while it is still empty.
 *
 * Finishing a merge is not writing a commit message: git named the merge when
 * it started it, that name is what every tool expects to see, and the box was
 * refusing to commit without being told again. Only into an empty box, and only
 * once — whatever is typed over it stays typed, and clearing the box back to
 * nothing hands the merge's own words back.
 */
watch(
  () => store.progress?.prepared ?? null,
  (ready) => {
    if (ready && blankMessage.value && !amend.value) message.value = ready
  },
  { immediate: true }
)

/**
 * Starts the box from the name of a stash just put back, while it is empty.
 *
 * Whatever was typed is kept: the name is a suggestion, and the box already
 * holding a message means somebody is writing a different one. Taken either
 * way, so a name that was not used does not turn up after the next commit.
 */
watch(
  () => store.carried,
  (carried) => {
    if (carried === null) return
    if (blankMessage.value && !amend.value) message.value = carried
    store.carried = null
  },
  { immediate: true }
)

/**
 * Whether throwing a conflict away is on offer at all.
 *
 * See `isRunning`. Mid-rebase "what the branch had" is the wrong side of the
 * argument, and a merge is undone by aborting it rather than file by file.
 */
const canThrowAway = computed(() => !isRunning(store.progress))

/**
 * Committing straight to the branch everything is organised around is worth a
 * word of warning. Which branch that is, is the repository's own answer — the
 * one the sidebar and the graph use — not a list of usual names.
 */
const onProtected = computed(() =>
  onTrunk(
    store.repo?.detached ? null : store.repo?.head,
    store.trunk.name,
    [...new Set((store.refs?.remotes ?? []).map((one) => one.remote))]
  )
)

/** Opens the file across the graph area, the way GitKraken does. */
function show(path: string, side: 'staged' | 'unstaged') {
  // A conflicted file has no one version to show side by side: what it needs
  // is the resolver, which is what a click on it opens.
  if (side === 'unstaged' && conflicted.value.includes(path)) {
    store.resolving = path
    return
  }
  store.viewer =
    store.viewer?.path === path && store.viewer?.side === side ? null : { path, side }
}

// --- committing
//
// One press is one commit. ⌘↵ held down, or pressed twice while git is still
// working, used to start a second commit — or a second amend, which rewrites
// the commit the first one just made.
const committing = ref(false)

/**
 * What the look over the staged diff found, while it is being shown. `at` is
 * the draft it was found for: repository and branch.
 */
const findings = ref<{
  list: GuardFinding[]
  push: boolean
  at: string
  /** What the button staged on its way to the warning, to unstage on "Go back". */
  swept: string[]
} | null>(null)

/** A commit a hook stopped, with what it printed and what it rewrote. */
const stopped = ref<{
  output: string
  hooks: string[]
  changed: string[]
  mixed: string[]
  push: boolean
  at: string
} | null>(null)

/**
 * The strips under the box are about one repository and one branch.
 *
 * Left up across a tab switch, "Stage the hook's changes" staged those paths
 * in the other repository and "Commit without hooks" committed it with
 * `--no-verify`. They go when the draft does, and every button on them checks
 * again that the draft in front is the one they were raised for.
 */
watch(
  () => draft.key.value,
  () => {
    stopped.value = null
    findings.value = null
    generated.value = null
  }
)

/** Whether the box still shows the draft `at` names. */
const still = (at: string) => draft.key.value === at

/**
 * Commits — staging everything first when that is what the button says —
 * and pushes after when asked.
 */
async function commit(push = false) {
  if (committing.value || store.busy || !canCommit.value) return
  committing.value = true
  const at = draft.key.value
  let swept: string[] = []
  try {
    if (stageAllFirst.value) {
      // Conflicts disable the button, so `--all` cannot sweep one in. The
      // list said nothing was staged; if something is by now — a `git add -p`
      // in a terminal — the backend refuses rather than sweep it in unseen.
      swept = unstaged.value.map((one) => one.path)
      if ((await git.stageAll(false)) === null || !still(at)) return
    }
    if (flow.value.guard) {
      const found = await git.commitGuard(flow.value.guard_debug)
      if (!still(at)) return
      if (found?.length) {
        findings.value = { list: found, push, at, swept }
        return
      }
    }
    await finish(push, false)
  } finally {
    committing.value = false
  }
}

/**
 * Backs out of the guard's warning, leaving the index as it was before the
 * button staged everything: only the paths it staged are taken back out.
 */
async function goBack() {
  const asked = findings.value
  findings.value = null
  if (!asked?.swept.length || !still(asked.at)) return
  await git.unstage(asked.swept)
}

/** Commits what the guard warned about, because the user said to. */
async function commitAnyway() {
  const asked = findings.value
  findings.value = null
  if (!asked || committing.value || !still(asked.at)) return
  committing.value = true
  try {
    await finish(asked.push, false)
  } finally {
    committing.value = false
  }
}

/**
 * The commit itself, and everything after one that went through.
 *
 * The commit goes to the repository the box was showing when it was pressed.
 * A hook can take long enough for another tab to come to the front, and what
 * follows — clearing the box, pushing — belongs to the first one: the draft is
 * cleared by its key, and nothing is pushed from a tab that is not it.
 */
async function finish(push: boolean, noVerify: boolean) {
  const at = draft.key.value
  const branch = store.repo?.detached ? null : (store.repo?.head ?? null)
  const amending = amend.value
  const rewrote = amending && amendPushed.value
  const attempt = await git.commitAttempt(composeMessage(message.value), amending, noVerify)
  if (!attempt) return false
  if (!still(at)) {
    if (attempt.ok) {
      draft.clear(at)
      git.note(
        push
          ? 'Committed. Another repository or branch is in front now, so it was not pushed.'
          : 'Committed.'
      )
    } else {
      git.note(`Commit: ${attempt.output}`, 'error')
    }
    return attempt.ok
  }
  if (!attempt.ok) {
    if (attempt.hook) {
      stopped.value = { output: attempt.output, ...attempt.hook, push, at }
      git.note(`Commit stopped by the ${attempt.hook.hooks.join(', ')} hook`)
    } else {
      git.note(`Commit: ${attempt.output}`, 'error')
    }
    return false
  }
  stopped.value = null
  findings.value = null
  generated.value = null
  draft.clear()
  prefill()
  // Back to the graph, whether or not anything is still uncommitted. The
  // commit that was being written is now in the history, which is what
  // there is to look at; what is left over is a fresh read, started from
  // the list rather than from whatever page the last one ended on.
  store.viewer = null
  if (push) await pushAfter(rewrote, at, branch)
  return true
}

/**
 * Pushes the branch the way the toolbar does, setting its upstream when it
 * has none.
 *
 * An amend of a commit that is already on the remote cannot go up as a plain
 * push. It is tried as one anyway, and the remote's refusal opens the strip
 * under the toolbar, which is where a force push is offered — with what it
 * would drop, leased on what was read. Nothing here forces on its own.
 */
async function pushAfter(rewrote: boolean, at: string, branch: string | null) {
  const head = store.refs?.locals.find((one) => one.is_head)
  if (!head) {
    git.note('Committed. There is no branch checked out to push.')
    return
  }
  // Only the branch that was committed on, in the repository it was made in.
  if (!still(at) || head.name !== branch) {
    git.note('Committed. The branch in front is not the one committed on, so nothing was pushed.')
    return
  }
  await git.requestPush(head.name, !head.upstream)
  if (rewrote && store.pushBlocked) {
    git.note(
      'The amended commit replaces one already on the remote, so it needs a force push — offered under the toolbar.'
    )
  }
}

/** Tries the stopped commit again, as it was. */
async function retry() {
  const was = stopped.value
  if (!was || committing.value || !still(was.at)) return
  committing.value = true
  try {
    await finish(was.push, false)
  } finally {
    committing.value = false
  }
}

/** Commits without the hooks: only ever from the button that says so. */
async function skipHooks() {
  const was = stopped.value
  if (!was || committing.value || !still(was.at)) return
  committing.value = true
  try {
    await finish(was.push, true)
  } finally {
    committing.value = false
  }
}

/**
 * Stages what the hook rewrote and commits again.
 *
 * Only files that were staged and had nothing else unstaged in them, so what
 * goes in is exactly what the hook wrote — not an edit of the user's that was
 * being kept out of this commit on purpose.
 */
async function stageHookChanges() {
  const was = stopped.value
  if (!was?.changed.length || committing.value || !still(was.at)) return
  committing.value = true
  try {
    if ((await git.stage(was.changed)) === null || !still(was.at)) return
    await finish(was.push, false)
  } finally {
    committing.value = false
  }
}

/**
 * Sends the change to the stash instead of committing it, using whatever is in
 * the summary box as the stash's name — otherwise stashes are impossible to
 * tell apart later.
 */
async function stashInstead() {
  const name = blankMessage.value ? undefined : subject.value || undefined
  if (await git.stashPush(name)) {
    draft.clear()
    prefill()
  }
}

// --- the model

/**
 * What the box held before the model wrote in it, and what the model wrote,
 * for as long as the text is still the model's. One step back is then one
 * click, and typing over it is taken as keeping it.
 */
const generated = ref<{ before: string; after: string; amend: boolean } | null>(null)
const canRevert = computed(
  () => !!generated.value && message.value === generated.value.after
)

/** Writing a message needs something to describe: staged work, or an amend. */
const canGenerate = computed(() => (staged.value.length > 0 || amend.value) && !ai.store.busy)

async function generate() {
  if (!ai.configured.value || !canGenerate.value) return
  const at = draft.key.value
  const before = message.value
  // The prefix alone is not a draft to finish, but it is still handed over:
  // it is the one thing the model cannot know.
  const written = await ai.commitMessage({ amend: amend.value, draft: before })
  if (!written) return
  // Written about the changes of the tab it was asked in; another one in
  // front now gets none of it.
  if (!still(at)) {
    git.note('The repository or branch changed while the model was writing, so its answer was not used')
    return
  }
  // Typed over while the model was thinking: what was typed wins.
  if (message.value !== before) {
    git.note('The message changed while the model was writing, so its answer was not used')
    return
  }
  const after = written.body ? `${written.summary}\n\n${written.body}` : written.summary
  message.value = after
  generated.value = { before, after, amend: amend.value }
  git.note('Commit message written by the model — read it before committing')
}

function revertGenerated() {
  if (!canRevert.value || !generated.value) return
  message.value = generated.value.before
  generated.value = null
}

// --- keys

const box = ref<HTMLTextAreaElement | null>(null)

/** ⌘↵ commits, ⌘⇧↵ commits and pushes — wherever settings put them. */
function onBoxKey(event: KeyboardEvent) {
  if (matchesShortcut(event, 'commit.push')) {
    event.preventDefault()
    void commit(true)
  } else if (matchesShortcut(event, 'commit.write')) {
    event.preventDefault()
    void commit(false)
  }
}

/** The working file open in the viewer, when there is one. */
const openFile = computed(() => {
  const open = store.viewer
  if (!open || open.commit) return null
  return { path: open.path, side: open.side ?? 'unstaged' }
})

/** Unstaged changes to one file waiting on the question before they go. */
const discarding = ref<string | null>(null)

function askDiscard() {
  const open = openFile.value
  if (!open || open.side !== 'unstaged') return
  const entry = unstaged.value.find((one) => one.path === open.path)
  // An untracked file has its own question, and a conflict its own dialog.
  if (!entry || entry.kind === 'untracked' || conflicted.value.includes(open.path)) {
    if (entry?.kind === 'untracked') deleting.value = [open.path]
    return
  }
  discarding.value = open.path
}

async function discardConfirmed() {
  const path = discarding.value
  discarding.value = null
  if (path) await git.discard([path])
}

/** Opens the changed file `by` steps along both lists, or the first one. */
function stepFile(by: number) {
  const order = [
    ...unstaged.value
      .filter((one) => !conflicted.value.includes(one.path))
      .map((one) => ({ path: one.path, side: 'unstaged' as const })),
    ...staged.value.map((one) => ({ path: one.path, side: 'staged' as const }))
  ]
  if (!order.length) return
  const open = openFile.value
  const at = open ? order.findIndex((one) => one.path === open.path && one.side === open.side) : -1
  const next = at < 0 ? (by > 0 ? 0 : order.length - 1) : at + by
  const target = order[next]
  if (target) store.viewer = target
}

useShortcuts({
  'commit.push': () => void commit(true),
  'commit.amend': () => !store.busy && void toggleAmend(!amend.value),
  'commit.generate': () => void generate(),
  'work.toggle': () => {
    const open = openFile.value
    if (!open || store.busy) return
    if (open.side === 'staged') void git.unstage([open.path])
    else if (!conflicted.value.includes(open.path)) void git.stage([open.path])
  },
  'work.stageall': () => {
    if (store.busy || !unstaged.value.length) return
    void (conflicted.value.length
      ? git.stage(unstaged.value.map((one) => one.path).filter((p) => !conflicted.value.includes(p)))
      : git.stageAll())
  },
  'work.unstageall': () =>
    !store.busy && staged.value.length && void git.unstage(staged.value.map((one) => one.path)),
  'work.discard': () => askDiscard(),
  'work.next': () => stepFile(1),
  'work.previous': () => stepFile(-1)
})

/** The commit button's title: what it does, and the key for it. */
const commitTitle = computed(() => `${keyLabel(keysFor('commit.write'))}`)
const pushTitle = computed(
  () => `Commit, then push the branch (${keyLabel(keysFor('commit.push'))})`
)

/**
 * The untracked files a menu has offered to delete, while it is being asked.
 *
 * Discarding a tracked file takes its content back from the index or from HEAD,
 * where it still is. An untracked file has no copy anywhere — deleting it is
 * the only thing "discard" could mean, and there is nothing to undo it with, so
 * it is the one thing here worth a question first.
 */
const deleting = ref<string[] | null>(null)

/**
 * The conflicted files a menu has offered to throw away, while it is being
 * asked. What the branch already had comes back; what the other side was
 * bringing in is gone, and nothing on the list remembers it.
 */
const clearing = ref<string[] | null>(null)

async function deleteUntracked() {
  const paths = deleting.value
  deleting.value = null
  if (paths?.length) await git.deleteUntracked(paths)
}

/**
 * The same moves as a file's menu, applied to everything under a folder.
 *
 * The paths are taken from the pane the folder was clicked in rather than
 * handed to git as a directory: what the menu offers is then exactly what the
 * rows underneath it show, with nothing swept in from the other pane.
 */
function dirMenu(event: MouseEvent, dir: string, side: 'staged' | 'unstaged') {
  const inside = (side === 'staged' ? staged.value : unstagedRows.value).filter((entry) =>
    entry.path.startsWith(`${dir}/`)
  )
  if (!inside.length) return
  const paths = inside.map((entry) => entry.path)
  const files = `${paths.length} ${paths.length === 1 ? 'file' : 'files'}`
  // Git will not discard a file it has never seen, and deleting one is not
  // something to do as a side effect of "discard changes".
  const tracked = inside
    .filter((entry) => entry.kind !== 'untracked' && entry.kind !== CONFLICTED)
    .map((entry) => entry.path)
  /** The conflicts under it, which are thrown away rather than discarded. */
  const clashes = inside.filter((entry) => entry.kind === CONFLICTED).map((entry) => entry.path)
  // The ones git has never seen, which discard cannot touch and delete can.
  const fresh = inside.filter((entry) => entry.kind === 'untracked').map((entry) => entry.path)

  menu.show(
    event,
    [
      side === 'staged'
        ? { label: `Unstage folder — ${files}`, icon: Minus, action: () => git.unstage(paths) }
        : {
            label: `Stage folder — ${files}`,
            icon: Plus,
            disabled: !tracked.length && !fresh.length,
            action: () => git.stage(paths.filter((path) => !clashes.includes(path)))
          },
      // Only on the unstaged side. What "discard" means for a staged file is
      // two questions at once — the copy on disk, and the one in the index —
      // and a folder of them is that question several times over. Unstage
      // first, and it is the one question this answers.
      ...(side === 'unstaged'
        ? [
            {
              label: `Discard changes in this folder — ${tracked.length} ${
                tracked.length === 1 ? 'file' : 'files'
              }`,
              icon: Undo2,
              danger: true,
              disabled: !tracked.length,
              action: () => git.discard(tracked)
            }
          ]
        : []),
      ...(clashes.length && canThrowAway.value
        ? [
            {
              label: `Throw away the ${clashes.length} ${
                clashes.length === 1 ? 'conflict' : 'conflicts'
              } in it`,
              hint: 'keeps what the branch had',
              icon: Undo2,
              danger: true,
              action: () => {
                clearing.value = clashes
              }
            }
          ]
        : []),
      ...(fresh.length
        ? [
            {
              label: `Delete the ${fresh.length} new ${
                fresh.length === 1 ? 'file' : 'files'
              } in it`,
              icon: Trash2,
              danger: true,
              action: () => {
                deleting.value = fresh
              }
            }
          ]
        : []),
      { separator: true, label: '' },
      { label: `Ignore everything in ${dir}/`, icon: EyeOff, action: () => git.addToGitignore(`${dir}/`) },
      { label: git.revealLabel, icon: FolderOpen, action: () => git.reveal(dir) },
      { label: 'Copy path', icon: Copy, action: () => copyText(dir, 'Path') }
    ],
    `${dir}/`
  )
}

function fileMenu(event: MouseEvent, path: string, side: 'staged' | 'unstaged', kind: string) {
  // A conflicted file is not stageable and has no changes to "discard" in the
  // ordinary sense: both sides of it are in the index. It gets its own menu —
  // decide it, or take the file back to what the branch already had.
  if (kind === CONFLICTED) {
    menu.show(
      event,
      [
        { label: 'Resolve…', icon: TriangleAlert, action: () => (store.resolving = path) },
        ...(canThrowAway.value
          ? [
              {
                label: 'Throw the conflict away',
                hint: 'keeps what the branch had',
                icon: Undo2,
                danger: true,
                action: () => {
                  clearing.value = [path]
                }
              }
            ]
          : []),
        { separator: true, label: '' },
        { label: git.revealLabel, icon: FolderOpen, action: () => git.reveal(path) },
        { label: 'Copy path', icon: Copy, action: () => copyText(path, 'Path') }
      ],
      path
    )
    return
  }

  menu.show(
    event,
    [
      side === 'staged'
        ? { label: 'Unstage', icon: Minus, action: () => git.unstage([path]) }
        : { label: 'Stage', icon: Plus, action: () => git.stage([path]) },
      // Only on the unstaged side, where it means the one thing it says.
      // From the staged side it threw away the unstaged edits — the copy on
      // disk — while the user was looking at the staged one, and did nothing
      // at all to a file with no unstaged edits. Unstage is the move there,
      // the same as the folder menu.
      ...(side === 'staged'
        ? []
        : [
            // A file git has never seen has no changes to take back — the
            // whole file is the change — so the same place in the menu
            // offers the only thing that could have meant.
            kind === 'untracked'
              ? {
                  label: 'Delete this file',
                  icon: Trash2,
                  danger: true,
                  action: () => {
                    deleting.value = [path]
                  }
                }
              : {
                  label: 'Discard changes to this file',
                  icon: Undo2,
                  danger: true,
                  action: () => git.discard([path])
                }
          ]),
      { separator: true, label: '' },
      {
        label: 'Add to .gitignore',
        icon: EyeOff,
        disabled: kind !== 'untracked',
        action: () => git.addToGitignore(path)
      },
      { label: git.revealLabel, icon: FolderOpen, action: () => git.reveal(path) },
      { label: 'Copy path', icon: Copy, action: () => copyText(path, 'Path') }
    ],
    path
  )
}

</script>

<template>
  <div class="working" :style="{ gridTemplateRows: rows }">
    <!-- Unstaged -->
    <div
      class="group"
      :class="{ drop: drag.state.over === 'zone:unstaged' }"
      @dragover="drag.hover($event, 'zone:unstaged', ['file'])"
      @dragleave="drag.leave($event, 'zone:unstaged')"
      @drop.prevent="
        (() => {
          const payload = drag.take(['file'])
          if (payload?.kind === 'file' && payload.staged) git.unstage([payload.path])
        })()
      "
    >
      <div class="group-head">
        <span class="section-title">
          Unstaged <span class="num">{{ unstagedRows.length }}</span>
          <button
            v-if="conflicted.length"
            class="clashes"
            :title="`${conflicted.length} conflicted — click to resolve the first`"
            @click="store.resolving = conflicted[0] ?? null"
          >
            <TriangleAlert :size="11" />{{ conflicted.length }}
          </button>
        </span>
        <span class="head-tools">
          <span class="toggle">
            <button
              class="seg"
              :class="{ on: view.state.mode === 'path' }"
              title="Flat list of full paths"
              @click="view.state.mode = 'path'"
            >
              Path
            </button>
            <button
              class="seg"
              :class="{ on: view.state.mode === 'tree' }"
              title="Grouped by folder"
              @click="view.state.mode = 'tree'"
            >
              Tree
            </button>
          </span>
          <!-- `git add --all` would stage a conflicted file exactly as it
               stands, markers and all, and call the conflict settled. While
               any are open only the files git is willing to take go on. -->
          <button
            class="btn tiny"
            :disabled="store.busy || !unstaged.length"
            @click="
              conflicted.length ? git.stage(unstaged.map((e) => e.path)) : git.stageAll()
            "
          >
            Stage all
          </button>
        </span>
      </div>
      <FileList
        :files="unstagedRows"
        :selected="selected?.side === 'unstaged' ? selected.path : null"
        empty="Nothing changed."
        draggable
        action="Stage file"
        @select="(path) => show(path, 'unstaged')"
        @act="
          (entry) =>
            entry.kind === CONFLICTED ? (store.resolving = entry.path) : git.stage([entry.path])
        "
        @menu="(event, entry) => fileMenu(event, entry.path, 'unstaged', entry.kind)"
        @dirmenu="(event, dir) => dirMenu(event, dir, 'unstaged')"
        @dragstart="
          (event, entry) => drag.begin(event, { kind: 'file', path: entry.path, staged: false })
        "
        @dragend="drag.end()"
      />
    </div>

    <!-- Staged -->
    <div
      class="group"
      :class="{ drop: drag.state.over === 'zone:staged' }"
      @dragover="drag.hover($event, 'zone:staged', ['file'])"
      @dragleave="drag.leave($event, 'zone:staged')"
      @drop.prevent="
        (() => {
          const payload = drag.take(['file'])
          if (payload?.kind === 'file' && !payload.staged) git.stage([payload.path])
        })()
      "
    >
      <div class="group-head">
        <span class="section-title">Staged <span class="num">{{ staged.length }}</span></span>
        <button
          class="btn tiny"
          :disabled="store.busy || !staged.length"
          @click="git.unstage(staged.map((e) => e.path))"
        >
          Unstage all
        </button>
      </div>
      <FileList
        :files="staged"
        :selected="selected?.side === 'staged' ? selected.path : null"
        empty="Drag files here, or stage them below."
        draggable
        action="Unstage file"
        @select="(path) => show(path, 'staged')"
        @act="(entry) => git.unstage([entry.path])"
        @menu="(event, entry) => fileMenu(event, entry.path, 'staged', entry.kind)"
        @dirmenu="(event, dir) => dirMenu(event, dir, 'staged')"
        @dragstart="
          (event, entry) => drag.begin(event, { kind: 'file', path: entry.path, staged: true })
        "
        @dragend="drag.end()"
      />
    </div>

    <!-- Commit box -->
    <div class="commit">
      <div class="commit-head">
        <label class="amend">
          <input
            type="checkbox"
            :checked="amend"
            :disabled="store.busy"
            @change="toggleAmend(($event.target as HTMLInputElement).checked)"
          />
          Amend previous commit
        </label>
        <span v-if="ai.configured.value" class="ai-tools">
          <!-- One step back from what the model wrote, for as long as the
               text is still the model's. -->
          <button
            v-if="canRevert"
            class="btn tiny ghost revert"
            title="Put back what was in the box before the model wrote in it"
            @click="revertGenerated"
          >
            <RotateCcw :size="12" /> Revert
          </button>
          <button
            class="btn tiny ai"
            :disabled="!canGenerate"
            :title="
              amend
                ? `Write a message for the amended commit, finishing what is typed (${keyLabel(keysFor('commit.generate'))})`
                : `Write a message from the staged diff, finishing what is typed (${keyLabel(keysFor('commit.generate'))})`
            "
            @click="generate"
          >
            <Spinner v-if="ai.store.busy" :size="12" />
            <Sparkles v-else :size="12" />
            {{ ai.store.busy ? 'Writing…' : 'Generate' }}
          </button>
        </span>
      </div>
      <div class="field">
        <textarea
          ref="box"
          v-model="message"
          rows="4"
          placeholder="Summary on the first line, why it changed below"
          @keydown="onBoxKey"
        />
        <span
          v-if="subject.length"
          class="counter"
          :class="{ over: subject.length > SUBJECT_LIMIT }"
          :title="`Subject length — git truncates past ${SUBJECT_LIMIT}`"
        >
          {{ subject.length }}
        </span>
      </div>
      <p v-if="amend && amendPushed" class="warn-line">
        <TriangleAlert :size="12" /> That commit is already on a remote — amending it will need a
        force push.
      </p>
      <p v-else-if="onProtected && (staged.length || amend || stageAllFirst)" class="warn-line">
        <TriangleAlert :size="12" /> You are committing straight to
        <span class="mono">{{ store.repo?.head }}</span>.
      </p>
      <!-- What is about to happen, where the button that does it is. An
           unset key is one click from being set rather than a web search. -->
      <button
        v-if="store.signing?.signs"
        class="signhint"
        title="Set by this repository’s commit.gpgsign"
        @click="config.openSettings('profiles')"
      >
        <ShieldCheck :size="12" />
        Will be signed<template v-if="signingKey"> · <span class="mono">{{ signingKey }}</span></template>
      </button>

      <!-- What the look over the staged diff found. A warning, never a wall:
           the person committing knows whether the key is a fixture. -->
      <div v-if="findings" class="stop guard">
        <p class="stop-head">
          <ShieldAlert :size="13" />
          {{ findings.list.length === 1 ? 'One thing' : `${findings.list.length} things` }} in
          what is staged worth a second look
        </p>
        <ul class="findings">
          <li v-for="(one, at) in findings.list" :key="at">
            <span class="what">{{ one.what }}</span>
            <span class="mono where">{{ one.path }}<template v-if="one.line">:{{ one.line }}</template></span>
            <span v-if="one.excerpt" class="mono excerpt truncate">{{ one.excerpt }}</span>
          </li>
        </ul>
        <div class="stop-buttons">
          <button class="btn tiny" :disabled="store.busy" @click="goBack">Go back</button>
          <button class="btn tiny warn" :disabled="store.busy" @click="commitAnyway">
            Commit anyway
          </button>
        </div>
      </div>

      <!-- A hook said no. What it said, where the button was, and the ways
           on — skipping it only on its own explicit button. -->
      <div v-else-if="stopped" class="stop hook">
        <p class="stop-head">
          <TriangleAlert :size="13" /> The {{ stopped.hooks.join(', ') }}
          {{ stopped.hooks.length === 1 ? 'hook' : 'hooks' }} stopped the commit
        </p>
        <pre class="hook-output">{{ stopped.output.trim() || 'It printed nothing.' }}</pre>
        <p v-if="stopped.changed.length" class="faint small">
          It rewrote {{ stopped.changed.join(', ') }}.
        </p>
        <p v-if="stopped.mixed.length" class="faint small">
          It also rewrote {{ stopped.mixed.join(', ') }}, which
          {{ stopped.mixed.length === 1 ? 'has' : 'have' }} unstaged edits of your own — stage
          {{ stopped.mixed.length === 1 ? 'that one' : 'those' }} by hand.
        </p>
        <div class="stop-buttons">
          <button
            v-if="stopped.changed.length"
            class="btn tiny"
            :disabled="store.busy"
            @click="stageHookChanges"
          >
            Stage the hook’s changes and retry
          </button>
          <button class="btn tiny" :disabled="store.busy" @click="retry">Retry</button>
          <button class="btn tiny ghost" :disabled="store.busy" @click="skipHooks">
            Commit without hooks
          </button>
          <button class="btn tiny ghost" @click="stopped = null">Dismiss</button>
        </div>
      </div>

      <div class="buttons">
        <button
          class="btn btn-primary wide"
          :disabled="store.busy || committing || !canCommit"
          :title="commitTitle"
          @click="commit(false)"
        >
          <Spinner v-if="store.busy || committing" :size="13" />
          <Check v-else :size="14" />
          <template v-if="amend">
            Amend commit{{ staged.length ? ` with ${staged.length} more` : '' }}
          </template>
          <template v-else-if="stageAllFirst">
            Stage all {{ unstaged.length }} and commit
          </template>
          <template v-else>
            Commit {{ staged.length }} {{ staged.length === 1 ? 'file' : 'files' }}
          </template>
        </button>
        <button
          class="btn btn-ghost push-btn"
          :disabled="store.busy || committing || !canCommit"
          :title="pushTitle"
          aria-label="Commit and push"
          @click="commit(true)"
        >
          <ArrowUpFromLine :size="14" />
        </button>
        <button
          class="btn btn-ghost stash-btn"
          :disabled="store.busy || (!staged.length && !unstaged.length)"
          :title="
            !blankMessage && subject
              ? `Stash everything as “${subject}” instead of committing`
              : 'Stash everything instead of committing'
          "
          @click="stashInstead"
        >
          <Archive :size="14" />
          Stash
        </button>
      </div>
      <p v-if="stageAllFirst && !conflicted.length" class="sweep faint">
        Nothing is staged, so this stages
        <template v-if="sweep.changed">{{ sweep.changed }} changed</template>
        <template v-if="sweep.changed && sweep.fresh"> and </template>
        <template v-if="sweep.fresh">{{ sweep.fresh }} new</template>
        {{ unstaged.length === 1 ? 'file' : 'files' }} first.
      </p>
      <p v-if="conflicted.length" class="blocked faint">
        <Ban :size="12" /> Resolve the conflicts first.
      </p>
    </div>
  </div>

  <!-- Asked every time, from the key. The app saves a copy first, so undo
       brings the edits back, but nothing in git itself has them. What is
       staged is left as it is. -->
  <AppModal
    v-if="discarding"
    title="Discard these changes?"
    :width="420"
    @close="discarding = null"
  >
    <p class="gone">
      The unstaged changes to <span class="mono">{{ discarding }}</span> go back to what is
      staged or committed. Anything staged stays staged.
    </p>
    <template #footer>
      <button class="btn btn-ghost" @click="discarding = null">Cancel</button>
      <button class="btn btn-danger" @click="discardConfirmed">Discard</button>
    </template>
  </AppModal>

  <!-- Asked because an untracked file is in no index and no commit. The app
       saves a copy before deleting, so undo brings it back, but that copy is
       one only this app knows about. -->
  <AppModal
    v-if="deleting"
    :title="deleting.length === 1 ? 'Delete this file?' : `Delete ${deleting.length} files?`"
    :width="420"
    @close="deleting = null"
  >
    <p class="gone">
      Git is not tracking
      {{ deleting.length === 1 ? 'it' : 'them' }}, so
      {{ deleting.length === 1 ? 'it is' : 'they are' }} only on disk. Undo in the history menu brings
      {{ deleting.length === 1 ? 'it' : 'them' }} back.
    </p>
    <ul class="gone-list mono">
      <li v-for="path in deleting.slice(0, 8)" :key="path" class="truncate">{{ path }}</li>
      <li v-if="deleting.length > 8" class="faint">…and {{ deleting.length - 8 }} more</li>
    </ul>

    <template #footer>
      <button class="btn btn-ghost" @click="deleting = null">Cancel</button>
      <button class="btn btn-danger" @click="deleteUntracked">
        {{ deleting.length === 1 ? 'Delete the file' : `Delete ${deleting.length} files` }}
      </button>
    </template>
  </AppModal>

  <DiscardConflictsDialog v-if="clearing" :paths="clearing" @close="clearing = null" />
</template>

<style scoped>
.gone {
  margin: 0 0 10px;
  font-size: 12.5px;
  line-height: 1.55;
}

.gone-list {
  margin: 0;
  padding: 6px 10px;
  list-style: none;
  max-height: 160px;
  overflow: auto;
  font-size: 11.5px;
  color: var(--text-dim);
  background: var(--surface);
  border: 1px solid var(--line-soft);
  border-radius: var(--radius-sm);
}

.working {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  min-height: 0;
  overflow: hidden;
}

/* How many of the rows below are conflicts, next to how many rows there are.
   Amber rather than red: nothing has been lost, something is undecided. */
.clashes {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  margin-left: 4px;
  padding: 0 6px;
  border-radius: var(--radius-pill);
  background: var(--warning-bg);
  color: var(--amber-soft);
  font-size: 10.5px;
  font-weight: 600;
  line-height: 16px;
  font-variant-numeric: tabular-nums;
}

.clashes:hover {
  box-shadow: inset 0 0 0 1px var(--warning-line);
}

/* The two lists, one above the other, with a hairline where one ends. */
.group {
  display: flex;
  flex-direction: column;
  min-height: 0;
  /* Clip inside the group when the panel is too short, rather than letting one
     group's rows bleed over the next one's header. */
  overflow: hidden;
  transition: background 0.1s;
}

.group + .group {
  border-top: 1px solid var(--line);
}

/* Somewhere a dragged file can land: tinted, with an accent edge. */
.group.drop {
  background: var(--primary-bg);
  box-shadow: inset 0 0 0 1px var(--primary-line);
}

.group-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-height: 30px;
  padding: 2px 6px 2px 0;
  flex: none;
}

.group-head .section-title {
  padding: 4px 12px;
  font-size: 12px;
  color: var(--text);
}

/* The count beside a heading. */
.num {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  padding: 0 5px;
  border-radius: var(--radius-pill);
  background: var(--bg-raised);
  color: var(--text-dim);
  font-size: 10.5px;
  font-weight: 600;
  line-height: 16px;
  font-variant-numeric: tabular-nums;
}

/* Stage all, unstage all: small quiet buttons, there when wanted. */
.tiny {
  min-height: 24px;
  font-size: 12px;
  padding: 1px 8px;
  border-radius: var(--radius-sm);
}

.tiny.ai {
  gap: 5px;
}

.tiny.ai svg {
  color: var(--info);
}

.tiny.warn {
  background: var(--amber);
  color: var(--bg);
  font-weight: 600;
}

.head-tools {
  display: flex;
  align-items: center;
  gap: 4px;
}

/* Path or tree: a small bordered segmented switch. */
.toggle {
  display: flex;
  height: 22px;
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  background: var(--bg);
  overflow: hidden;
}

.seg {
  padding: 0 8px;
  font-size: 11px;
  font-weight: 500;
  line-height: 20px;
  color: var(--text-dim);
  transition:
    background 0.1s,
    color 0.1s;
}

.seg + .seg {
  border-left: 1px solid var(--line);
}

.seg:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.seg.on {
  background: var(--bg-active);
  color: var(--text);
  font-weight: 600;
}

/* The composer closes the panel: the message, and under it the one button this
   whole panel exists for. */
.commit {
  padding: 10px;
  border-top: 1px solid var(--line);
}

.commit-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 6px;
}

.amend {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-dim);
  cursor: pointer;
}

.amend:hover {
  color: var(--text);
}

.field {
  position: relative;
  margin-bottom: 8px;
}

/* An editable well on the page colour, so it reads as the place to type. */
textarea {
  width: 100%;
  display: block;
  min-height: 80px;
  padding: 7px 40px 7px 9px;
  border-radius: var(--radius);
  background: var(--bg);
  line-height: 1.5;
}

.counter {
  position: absolute;
  top: 8px;
  right: 9px;
  font-size: 10.5px;
  line-height: 16px;
  font-variant-numeric: tabular-nums;
  color: var(--text-faint);
  pointer-events: none;
}

.counter.over {
  color: var(--amber-soft);
  font-weight: 600;
}

.buttons {
  display: flex;
  gap: 6px;
}

.wide {
  flex: 1;
  min-height: 32px;
  justify-content: center;
}

.stash-btn {
  flex: none;
  min-height: 32px;
}

.warn-line,
.blocked {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 0 8px;
  padding: 5px 8px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  background: var(--warning-bg);
  color: var(--amber-soft);
  border: 1px solid var(--warning-line);
}

.blocked {
  margin: 8px 0 0;
  padding: 0;
  background: none;
  border: none;
  color: var(--text-faint);
}

/* Says what the button below will do, in the colour of it being fine. */
.signhint {
  display: flex;
  align-items: center;
  gap: 5px;
  margin: 0 0 8px;
  padding: 0;
  font-size: 11.5px;
  color: var(--success-soft);
}

.signhint:hover {
  text-decoration: underline;
}

.signhint .mono {
  font-family: var(--mono);
  font-size: 10.5px;
  color: var(--text-faint);
}

.ai-tools {
  display: flex;
  align-items: center;
  gap: 4px;
}

.revert {
  gap: 4px;
}

/* Commit and push: the same press with one more step, so it sits right beside
   the button it extends and says what it adds with an arrow. */
.push-btn {
  flex: none;
  min-height: 32px;
  padding: 0 9px;
}

.sweep {
  margin: 6px 0 0;
  font-size: 11.5px;
}

/* A commit that did not happen yet, and why: in place, above the buttons
   that will try again. */
.stop {
  margin: 0 0 8px;
  padding: 7px 8px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  background: var(--warning-bg);
  border: 1px solid var(--warning-line);
}

.stop-head {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 0 6px;
  color: var(--amber-soft);
  font-weight: 600;
}

.stop .small {
  margin: 4px 0 0;
  font-size: 11.5px;
}

.findings {
  margin: 0;
  padding: 0;
  list-style: none;
  max-height: 140px;
  overflow: auto;
}

.findings li {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  padding: 3px 0;
}

.findings li + li {
  border-top: 1px solid var(--line-soft);
}

.findings .where,
.findings .excerpt {
  font-family: var(--mono);
  font-size: 10.5px;
  color: var(--text-dim);
}

.hook-output {
  margin: 0;
  padding: 6px 8px;
  max-height: 160px;
  overflow: auto;
  font-family: var(--mono);
  font-size: 11px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
  background: var(--bg);
  border: 1px solid var(--line-soft);
  border-radius: var(--radius-sm);
  color: var(--text);
}

.stop-buttons {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 8px;
}
</style>
