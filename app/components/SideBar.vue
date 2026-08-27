<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Archive,
  ArrowDown,
  ArrowUp,
  ChevronRight,
  Cloud,
  Copy,
  Download,
  Eye,
  EyeOff,
  ExternalLink,
  Folder,
  FolderOpen,
  GitBranch,
  GitMerge,
  GitPullRequest,
  HardDrive,
  Hash,
  Milestone,
  Pencil,
  Plus,
  Search,
  Tag,
  Trash2,
  Upload
} from 'lucide-vue-next'
import { copyText, fullTime, relativeTime, useGit, type Tag as TagRef } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useDragDrop } from '~/composables/useDragDrop'
import { useForge, type Review } from '~/composables/useForge'
import { useReview } from '~/composables/useReview'
import { useConfig } from '~/composables/useConfig'

const git = useGit()
const store = git.store
const menu = useContextMenu()
const drag = useDragDrop()
const forge = useForge()
const reviewPage = useReview()
const config = useConfig()

const SECTIONS_KEY = 'gitnoob:sidebar-sections'

/** Which sections stand open, remembered across runs of the app. */
const open = reactive(readSections())

function readSections() {
  try {
    const saved = JSON.parse(localStorage.getItem(SECTIONS_KEY) ?? '{}')
    return {
      locals: saved.locals !== false,
      remotes: saved.remotes !== false,
      tags: saved.tags === true,
      stashes: saved.stashes !== false,
      reviews: saved.reviews !== false
    }
  } catch {
    return { locals: true, remotes: true, tags: false, stashes: true, reviews: true }
  }
}

watch(open, () => {
  try {
    localStorage.setItem(SECTIONS_KEY, JSON.stringify(open))
  } catch {
    // A window that cannot remember still shows the sections this session.
  }
})

const filter = ref('')
/** The branch whose deletion is being confirmed. */
const deleting = ref<string | null>(null)
/** Set while the new pull request dialog is open. */
const creatingReview = ref(false)
/** The remote form: `'add'`, the name being edited, or closed. */
const remoteForm = ref<'add' | string | null>(null)
const prompt = ref<{
  title: string
  label: string
  initial?: string
  hint?: string
  confirm: string
  danger?: boolean
  run: (value: string) => void
} | null>(null)

const match = (name: string) =>
  !filter.value.trim() || name.toLowerCase().includes(filter.value.trim().toLowerCase())

const FOLDERS_KEY = 'gitnoob:sidebar-folders'

/**
 * Folders the user has shut, remembered across runs. Everything starts open,
 * so a window that has never been touched shows branches rather than a wall of
 * closed folders.
 */
const shut = reactive(new Set<string>(readShut()))

function toggleFolder(path: string) {
  if (shut.has(path)) shut.delete(path)
  else shut.add(path)
  saveShut()
}

function readShut(): string[] {
  try {
    const saved = JSON.parse(localStorage.getItem(FOLDERS_KEY) ?? '[]')
    return Array.isArray(saved) ? saved.filter((one) => typeof one === 'string') : []
  } catch {
    return []
  }
}

function saveShut() {
  try {
    localStorage.setItem(FOLDERS_KEY, JSON.stringify([...shut]))
  } catch {
    // A window that cannot remember still opens and shuts folders this session.
  }
}

/** A folder heading or a branch, at a depth, ready for one flat `v-for`. */
type Shelf<T> =
  | { kind: 'folder'; key: string; path: string; label: string; depth: number; shut: boolean }
  | { kind: 'branch'; key: string; item: T; label: string; depth: number }

/**
 * Sorts branch names into the folders their slashes already describe.
 *
 * `feature/login` and `feature/signup` are two branches in one place, and a
 * project that names them that way ends up with a column of near-identical
 * prefixes, each one pushing the part that differs off the right-hand edge. The
 * folder carries the shared half, and a branch is listed under it by the part
 * that tells it apart.
 *
 * The list is flat, with a depth on each row, rather than a component nesting
 * itself: the rows are drag targets and menu targets, and keeping them siblings
 * keeps that wiring in one place.
 *
 * While a filter is typed every folder is open — someone searching wants what
 * they searched for, not the folder it happens to live in.
 */
/**
 * Turns a flat list of refs into folder rows.
 *
 * `pin` is the one name that goes to the top whatever the alphabet says: the
 * branch everything else is measured against belongs where the eye lands, not
 * wherever `main` happens to fall between `fix-login` and `release`.
 */
function shelve<T extends { name: string }>(
  items: T[],
  scope: string,
  pin: string | null = null
): Shelf<T>[] {
  const rows: Shelf<T>[] = []
  const searching = !!filter.value.trim()
  const sorted = [...items].sort((a, b) => {
    if (pin && a.name === pin) return -1
    if (pin && b.name === pin) return 1
    return a.name.localeCompare(b.name)
  })
  let trail: string[] = []
  // The depth of the shut folder currently hiding rows, if any.
  let hidden: number | null = null

  for (const item of sorted) {
    const parts = item.name.split('/')
    const dirs = parts.slice(0, -1)

    let same = 0
    while (same < dirs.length && same < trail.length && dirs[same] === trail[same]) same++
    if (hidden !== null && hidden >= same) hidden = null
    trail = trail.slice(0, same)

    for (let i = same; i < dirs.length; i++) {
      trail.push(dirs[i]!)
      if (hidden !== null) continue
      const path = `${scope}:${trail.join('/')}`
      const closed = !searching && shut.has(path)
      rows.push({ kind: 'folder', key: `folder:${path}`, path, label: dirs[i]!, depth: i, shut: closed })
      if (closed) hidden = i
    }

    if (hidden === null) {
      rows.push({
        kind: 'branch',
        key: `branch:${scope}:${item.name}`,
        item,
        label: parts[parts.length - 1]!,
        depth: dirs.length
      })
    }
  }
  return rows
}

// --- section heights

/** The sections that can be dragged. Stashes is not one: it is last, and takes
    whatever the others leave. */
type Section = 'locals' | 'remotes' | 'reviews' | 'tags'

const SIZES_KEY = 'gitnoob:sidebar-sizes'

/** A section dragged below this is not worth the scrollbar it would need. */
const MIN_SECTION = 40

/**
 * Heights the user has dragged sections to, in pixels.
 *
 * Undragged, a section is as tall as what is in it and no taller than the cap
 * in the stylesheet — three merge requests take three rows, forty remote
 * branches take the cap and scroll inside it. Nothing is squeezed to make the
 * column fit: the sidebar scrolls when the sections together are taller than
 * it, which is the one thing a person can predict.
 *
 * One set for every profile: the shape of the sidebar belongs to the window,
 * not to the account signed into it.
 */
const sizes = reactive<Partial<Record<Section, number>>>(readSizes())

function readSizes(): Partial<Record<Section, number>> {
  try {
    const saved = JSON.parse(localStorage.getItem(SIZES_KEY) ?? '{}')
    // Heights used to be kept per profile, each set under the profile's id.
    // One of those sets is adopted as the shared one rather than thrown away —
    // the active profile's where it exists.
    if (saved && Object.values(saved).some((value) => value && typeof value === 'object')) {
      const byProfile = saved as Record<string, Partial<Record<Section, number>>>
      const one = byProfile[config.profile.value?.id ?? ''] ?? byProfile.default ?? {}
      return withoutStashes(one)
    }
    return withoutStashes(saved)
  } catch {
    return {}
  }
}

/** Stashes was resizable once. A height left over from then would fight the
    section that now fills the bottom, so it is dropped on the way in. */
function withoutStashes(saved: Record<string, unknown>): Partial<Record<Section, number>> {
  const { stashes: _stashes, ...rest } = saved
  return rest as Partial<Record<Section, number>>
}

function saveSizes() {
  try {
    localStorage.setItem(SIZES_KEY, JSON.stringify(sizes))
  } catch {
    // A window that cannot remember the layout still lays it out.
  }
}

function sizeOf(section: Section) {
  const height = sizes[section]
  // `max-height` is what caps an undragged section; a dragged one has said what
  // it wants, so the cap comes off and the height stands.
  return height ? { height: `${height}px`, maxHeight: 'none' } : undefined
}

/**
 * Drags the divider under a section to set its height.
 *
 * The grip resizes the section above it, which is the one the pointer just
 * left — the same rule every split view uses. Nothing else moves: the sections
 * below keep their heights and the sidebar scrolls if that no longer fits.
 */
function grab(event: PointerEvent, section: Section) {
  const grip = event.currentTarget as HTMLElement
  const group = grip.previousElementSibling as HTMLElement | null
  if (!group) return

  const startY = event.clientY
  const startHeight = group.getBoundingClientRect().height
  grip.setPointerCapture(event.pointerId)

  const move = (moved: PointerEvent) => {
    sizes[section] = Math.max(MIN_SECTION, startHeight + moved.clientY - startY)
  }
  const done = () => {
    grip.releasePointerCapture(event.pointerId)
    grip.removeEventListener('pointermove', move)
    grip.removeEventListener('pointerup', done)
    saveSizes()
  }
  grip.addEventListener('pointermove', move)
  grip.addEventListener('pointerup', done)
  event.preventDefault()
}

/** Double-clicking a divider gives a section its ordinary height back. */
function resetSize(section: Section) {
  delete sizes[section]
  saveSizes()
}

// --- branches put out of the way

const HIDDEN_KEY = 'gitnoob:hidden-branches'

/**
 * Branches the user has dimmed, by repository path.
 *
 * Kept rather than removed from the list: a branch you have stopped working on
 * is still one you check out again, and a list that hides things outright
 * leaves you wondering where they went. Dimmed, it stays where it was and stops
 * competing for the eye with the branches actually in play.
 *
 * By repository because branch names only mean anything inside one.
 */
const hidden = reactive<Record<string, string[]>>(readHidden())

function readHidden(): Record<string, string[]> {
  try {
    return JSON.parse(localStorage.getItem(HIDDEN_KEY) ?? '{}')
  } catch {
    return {}
  }
}

const repoKey = computed(() => store.repo?.path ?? '')

function isHidden(name: string) {
  return (hidden[repoKey.value] ?? []).includes(name)
}

function toggleHidden(name: string) {
  const key = repoKey.value
  if (!key) return
  const list = hidden[key] ?? []
  hidden[key] = list.includes(name) ? list.filter((one) => one !== name) : [...list, name]
  if (!hidden[key].length) delete hidden[key]
  try {
    localStorage.setItem(HIDDEN_KEY, JSON.stringify(hidden))
  } catch {
    // A window that cannot remember it still dims it for this session.
  }
}

const head = computed(() => store.repo?.head ?? '')

/** The branch this repository is organised around, pinned to the top of both lists. */
const trunk = computed(() => store.trunk.name)

/**
 * The trunk as one remote's list writes it.
 *
 * A remote branch is listed under its remote by the bare name, so `origin/main`
 * is the row called `main` inside `origin`. A trunk that is already a bare name
 * pins in every group, which is what somebody with two remotes would expect.
 */
function remotePin(remote: string) {
  const name = trunk.value
  if (!name) return null
  return name.startsWith(`${remote}/`) ? name.slice(remote.length + 1) : name
}

const locals = computed(() => (store.refs?.locals ?? []).filter((b) => match(b.name)))
const localShelf = computed(() => shelve(locals.value, 'local', trunk.value))
const tags = computed(() => (store.refs?.tags ?? []).filter((t) => match(t.name)))

/**
 * The hover text for a tag row: an annotated tag has a message and a date of
 * its own worth reading, a lightweight one is only a name on a commit.
 */
function describeTag(tag: TagRef) {
  const when = fullTime(tag.when)
  if (!tag.annotated) return `${tag.name} — lightweight, on ${tag.oid.slice(0, 7)}`
  const head = `${tag.name} — annotated, ${when}`
  return tag.message ? `${head}\n${tag.message}` : head
}

const stashes = computed(() =>
  store.stashes.filter((stash) => match(`${stash.message} ${stash.branch ?? ''}`))
)

/**
 * Reviews matching the filter.
 *
 * Searched by everything shown on the row and by the branch behind it: people
 * look for a merge request by its number as often as by its title, and by the
 * branch more often than either.
 */
const reviews = computed(() =>
  forge.store.reviews.filter((review) =>
    match(`${forge.sigil.value}${review.number} ${review.title} ${review.source_branch} ${review.author}`)
  )
)

/**
 * How a review's branch reads when it belongs to somebody else: `them:fix-typo`
 * is how the forges write it, and the bare name would be a lie — there is no
 * `fix-typo` here, and there may well be a different one.
 */
function reviewBranch(review: Review) {
  const from = review.source
  return from?.is_fork && from.owner ? `${from.owner}:${review.source_branch}` : review.source_branch
}

function reviewTitle(review: Review) {
  const where = review.source
    ? review.source.is_fork
      ? `from ${review.source.full_name}`
      : ''
    : 'the branch it came from has been deleted'
  const lines = [review.title, `${reviewBranch(review)} → ${review.target_branch}`]
  if (where) lines.push(where)
  lines.push('Click to read the review · double-click to check it out')
  return lines.join('\n')
}

/** The review whose page is open, so its row stays lit while it is read. */
const openReview = computed(() => reviewPage.store.current)

/**
 * A single click on a review opens it here, in place of the graph — the
 * conversation, the files across the whole review, and the remarks standing on
 * their lines.
 *
 * But a double click is two singles, and the page opening on the first would
 * take the column away before the second arrived. So the open waits a beat,
 * and the double click cancels it and checks the branch out instead. Anything
 * slower than that beat was one click.
 */
const DBLCLICK_WAIT = 220
let openTimer: number | undefined

function openReviewPage(review: Review) {
  window.clearTimeout(openTimer)
  openTimer = window.setTimeout(() => reviewPage.show(review), DBLCLICK_WAIT)
}

/** A double click: stand on it, fetching and adding the fork if that is what
    having those commits takes. */
function checkoutReview(review: Review) {
  window.clearTimeout(openTimer)
  return git.checkoutReview({
    number: review.number,
    branch: review.source_branch,
    head_sha: review.head_sha,
    source: review.source
  })
}

/** One remote branch, as the rows need it. */
interface RemoteRef {
  name: string
  oid: string
}

/**
 * How many remote branches are handed over at a time.
 *
 * A project of any age has hundreds of them and a busy one has thousands —
 * fourteen hundred is not unusual — and every one of those rows is a piece of
 * document to build, lay out and then rebuild the next time anything in the
 * sidebar changes. Put on screen all at once they cost more than everything
 * else the window does put together: a tenth of a second on every tab switch,
 * on every refresh, and on every letter typed into the filter.
 *
 * So the list hands over a hundred and asks for the next hundred when the
 * scroll approaches the end. Only the drawing is rationed: the filter still
 * runs over every branch the repository has, because it searches the branches
 * and not the rows.
 */
const REMOTE_PAGE = 100

const remoteShown = ref(REMOTE_PAGE)

// A different repository is a different list, and it starts at the top. A
// refresh is not: it would drag someone who had scrolled back to the first
// hundred while they were reading.
watch(repoKey, () => {
  remoteShown.value = REMOTE_PAGE
})

// Neither is typing, but there the list really is new.
watch(filter, () => {
  remoteShown.value = REMOTE_PAGE
})

/** Every remote branch the filter left, grouped by the remote it is on. */
const remoteGroups = computed(() => {
  const groups = new Map<string, RemoteRef[]>()
  for (const branch of store.refs?.remotes ?? []) {
    if (!match(branch.name)) continue
    const list = groups.get(branch.remote) ?? []
    list.push({ name: branch.name, oid: branch.oid })
    groups.set(branch.remote, list)
  }
  // Sorted here rather than only in `shelve`, because the list below takes the
  // first so many branches and a trunk left in alphabetical order would be cut
  // off the end of a repository with more remote branches than the list draws.
  return [...groups.entries()].map(([remote, branches]) => {
    const pin = remotePin(remote)
    branches.sort((a, b) => {
      if (pin && a.name === pin) return -1
      if (pin && b.name === pin) return 1
      return a.name.localeCompare(b.name)
    })
    return { remote, branches }
  })
})

/** How many remote branches the filter left, across every remote. */
const remoteCount = computed(() =>
  remoteGroups.value.reduce((sum, group) => sum + group.branches.length, 0)
)

/**
 * The rows to draw: the first `remoteShown` branches, foldered.
 *
 * Worked out here rather than in the template, where it sat inside the `v-for`
 * that drew it — so every render of the sidebar, whatever had changed, re-sorted
 * and re-walked every branch in the repository before drawing a single row.
 */
const remoteShelves = computed(() => {
  let budget = remoteShown.value
  const shelves: { remote: string; rows: Shelf<RemoteRef>[] }[] = []
  for (const group of remoteGroups.value) {
    if (budget <= 0) break
    const branches = group.branches.slice(0, budget)
    budget -= branches.length
    shelves.push({
      remote: group.remote,
      rows: shelve(branches, `remote:${group.remote}`, remotePin(group.remote))
    })
  }
  return shelves
})

/** How many the filter found but the list is not drawing yet. */
const remoteMore = computed(() => Math.max(0, remoteCount.value - remoteShown.value))

/** Asks for the next hundred once the end of the list is nearly in view. */
function onRemoteScroll(event: Event) {
  if (!remoteMore.value) return
  const list = event.currentTarget as HTMLElement
  if (list.scrollTop + list.clientHeight < list.scrollHeight - 200) return
  remoteShown.value += REMOTE_PAGE
}

// --- drag and drop between branches

/**
 * Dropping one branch on another asks what to do with it.
 *
 * Merge and rebase are the same gesture with different consequences, so the
 * menu names both in terms of the two branches rather than in git's vocabulary.
 */
async function onDropOnBranch(event: MouseEvent, target: string, targetIsRemote: boolean) {
  const payload = drag.take(['branch', 'commit', 'stash'])
  if (!payload) return

  if (payload.kind === 'commit') {
    menu.show(
      event,
      [
        {
          label: `Cherry-pick ${payload.short} onto ${target}`,
          icon: GitBranch,
          hint: target === head.value ? '' : 'checks out first',
          action: async () => {
            if (target !== head.value) await git.checkout(target)
            await git.cherryPick([payload.oid])
          }
        }
      ],
      payload.summary.slice(0, 48)
    )
    return
  }

  if (payload.kind === 'stash') {
    menu.show(
      event,
      [
        {
          label: `Apply this stash on ${target}`,
          icon: Archive,
          action: async () => {
            if (target !== head.value) await git.checkout(target)
            await git.stashApply(payload.index)
          }
        }
      ],
      payload.message.slice(0, 48)
    )
    return
  }

  const source = payload.name
  if (source === target) return
  if (targetIsRemote) {
    git.note('Drop onto a local branch to merge or rebase', 'error')
    return
  }

  // What is actually possible between these two. Offering a move that would do
  // nothing is worse than not offering it: it gets picked, nothing visible
  // happens, and the user is left guessing whether it worked.
  const relation = await git.branchRelation(source, target)
  const ahead = relation?.ahead ?? 0
  const behind = relation?.behind ?? 0
  const merged = ahead === 0
  const fastForward = ahead > 0 && behind === 0

  if (merged) {
    menu.show(
      event,
      [
        {
          label: `${target} already has everything from ${source}`,
          icon: GitMerge,
          disabled: true
        }
      ],
      `${source} → ${target}`
    )
    return
  }

  menu.show(
    event,
    [
      // Merge first: it is what the gesture means nine times in ten, and a
      // menu that puts a special case above the ordinary one makes the reader
      // check both every time.
      {
        label: `Merge ${source} into ${target}`,
        icon: GitMerge,
        hint: fastForward ? 'forces a merge commit' : '',
        action: async () => {
          const outcome = await git.mergeInto(source, target, fastForward)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      // Offered only when it is on the table: the target has nothing of its
      // own, so it can simply be moved forward.
      ...(fastForward
        ? [
            {
              label: `Fast-forward ${target} to ${source}`,
              icon: GitMerge,
              hint: 'no merge commit',
              action: () => git.mergeInto(source, target, false)
            }
          ]
        : []),
      // Rebasing a branch that has nothing of its own is a fast-forward with
      // extra steps, so it is only worth offering once the two have diverged.
      ...(behind > 0
        ? [
            {
              label: `Rebase ${target} onto ${source}`,
              icon: GitBranch,
              hint: `${behind} rewritten`,
              danger: true,
              action: async () => {
                const outcome = await git.rebaseBranch(target, source)
                if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
              }
            }
          ]
        : [])
    ],
    `${ahead} ahead · ${behind} behind`
  )
}

// --- context menus

function localMenu(event: MouseEvent, name: string, upstream: string | null) {
  const isHead = name === head.value
  const isTrunk = name === trunk.value
  menu.show(
    event,
    [
      // Ordered by how often it is wanted: the two that move commits between
      // here and the remote, then checking out, then the ones that rewrite or
      // combine history, then housekeeping, then the one that destroys
      // something.
      {
        // Works on a branch you are not standing on, open changes and all:
        // the backend moves the ref directly when it can, and visits the
        // branch and comes back when it cannot.
        label: upstream ? `Pull from ${upstream}` : 'Pull',
        icon: Download,
        disabled: !upstream,
        hint: upstream ? '' : 'no upstream',
        action: () => git.pullBranch(name)
      },
      {
        label: upstream ? `Push to ${upstream}` : 'Push and set upstream',
        icon: Upload,
        action: () => git.pushBranch(name, !upstream)
      },
      { label: 'Check out', icon: GitBranch, disabled: isHead, action: () => git.checkout(name) },
      { separator: true, label: '' },
      // All three move history between two branches, in different directions,
      // and "merge" alone does not say which way. Name both branches and say
      // which one ends up changed. The second direction is the one git makes
      // awkward — it merges into where you stand and nowhere else — so it is
      // offered here and the switching is done out of sight.
      {
        label: `Merge ${name} into ${head.value}`,
        icon: GitMerge,
        hint: '',
        disabled: isHead,
        action: async () => {
          const outcome = await git.merge(name, false)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      {
        label: `Merge ${head.value} into ${name}`,
        icon: GitMerge,
        hint: '',
        disabled: isHead,
        action: async () => {
          const outcome = await git.mergeInto(head.value, name, false)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      {
        label: `Rebase ${head.value} onto ${name}`,
        icon: GitBranch,
        hint: 'rewrites history',
        disabled: isHead,
        action: async () => {
          const outcome = await git.rebase(name)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      { separator: true, label: '' },
      {
        // What "is this safe to delete?" gets measured against. Guessed as main
        // or master until somebody says otherwise, which is wrong in every
        // repository that calls its trunk something else — and quietly wrong,
        // because a branch that has only reached `staging` reads as landed.
        label: isTrunk ? 'Not the main branch' : 'Use as the main branch',
        icon: Milestone,
        hint: isTrunk ? 'back to guessing' : 'what "safe to delete" is measured against',
        action: async () => {
          await git.setTrunk(isTrunk ? null : name)
        }
      },
      {
        label: 'Copy branch name',
        icon: Copy,
        action: () => copyText(name, 'Branch')
      },
      {
        // Nothing happens to the branch itself; this is about the list.
        label: isHidden(name) ? 'Undim in the list' : 'Dim in the list',
        icon: isHidden(name) ? Eye : EyeOff,
        hint: isHidden(name) ? '' : 'still there, just quiet',
        action: () => toggleHidden(name)
      },
      {
        label: 'Rename…',
        icon: Pencil,
        action: () =>
          (prompt.value = {
            title: 'Rename branch',
            label: 'New name',
            initial: name,
            confirm: 'Rename',
            run: (value) => git.renameBranch(name, value)
          })
      },
      { separator: true, label: '' },
      {
        label: 'Delete branch…',
        icon: Trash2,
        danger: true,
        disabled: isHead,
        action: () => (deleting.value = name)
      }
    ],
    name
  )
}

function remoteMenu(event: MouseEvent, remote: string, name: string) {
  const full = `${remote}/${name}`
  menu.show(
    event,
    [
      {
        label: 'Check out as a local branch',
        icon: GitBranch,
        action: () => git.checkout(full)
      },
      {
        label: `Merge into ${head.value}`,
        icon: GitMerge,
        action: async () => {
          const outcome = await git.merge(full, false)
          if (outcome?.conflicts.length) store.resolving = outcome.conflicts[0]
        }
      },
      {
        label: 'Set as upstream of current branch',
        icon: Cloud,
        action: () => git.setUpstream(head.value, full)
      },
      { separator: true, label: '' },
      { label: 'Copy name', icon: Copy, action: () => copyText(full, 'Branch') },
      {
        label: `Delete ${name} on ${remote}`,
        icon: Trash2,
        danger: true,
        action: () =>
          (prompt.value = {
            title: `Delete ${name} on ${remote}?`,
            label: 'Type the branch name to confirm',
            confirm: 'Delete on remote',
            danger: true,
            hint: 'This removes the branch for everyone, not just here.',
            run: (value) => {
              if (value === name) git.deleteRemoteBranch(remote, name)
              else git.note('Name did not match; nothing was deleted', 'error')
            }
          })
      }
    ],
    full
  )
}

/** The remote itself, rather than one of its branches. */
function remoteHeaderMenu(event: MouseEvent, remote: string) {
  menu.show(
    event,
    [
      {
        label: 'Fetch from this remote',
        icon: Download,
        action: () => {
          void git.fetch(remote)
        }
      },
      { separator: true, label: '' },
      {
        label: 'Change address…',
        icon: Pencil,
        action: () => {
          remoteForm.value = remote
        }
      },
      {
        label: 'Rename…',
        icon: Pencil,
        action: () => {
          prompt.value = {
            title: `Rename remote ${remote}`,
            label: 'New name',
            initial: remote,
            confirm: 'Rename',
            hint: 'The remote-tracking branches move with the name; local branches keep tracking them.',
            run: (value) => {
              if (value !== remote) void git.remoteRename(remote, value)
            }
          }
        }
      },
      { label: 'Copy name', icon: Copy, action: () => { void copyText(remote, 'Remote') } },
      {
        label: `Remove ${remote}`,
        icon: Trash2,
        danger: true,
        action: () => {
          prompt.value = {
            title: `Remove remote ${remote}?`,
            label: 'Type the remote name to confirm',
            confirm: 'Remove remote',
            danger: true,
            hint: 'Removes the remote and its remote-tracking branches. Local branches and their commits stay; adding the remote back restores the tracking branches.',
            run: (value) => {
              if (value === remote) void git.remoteRemove(remote)
              else git.note('Name did not match; nothing was removed', 'error')
            }
          }
        }
      }
    ],
    remote
  )
}

function tagMenu(event: MouseEvent, name: string, oid: string) {
  menu.show(
    event,
    [
      { label: 'Check out this tag', icon: GitBranch, action: () => git.checkout(name) },
      { label: 'Push tag to origin', icon: Upload, action: () => git.pushTag('origin', name) },
      { label: 'Copy tag name', icon: Copy, action: () => copyText(name, 'Tag') },
      { label: 'Copy commit hash', icon: Hash, action: () => copyText(oid, 'Hash') },
      { separator: true, label: '' },
      {
        label: 'Delete tag locally',
        icon: Trash2,
        danger: true,
        action: () => git.deleteTag(name)
      }
    ],
    name
  )
}

function stashMenu(event: MouseEvent, index: number, message: string) {
  menu.show(
    event,
    [
      { label: 'Apply and keep', icon: Archive, action: () => git.stashApply(index) },
      { label: 'Pop (apply and remove)', icon: Archive, action: () => git.stashPop(index) },
      {
        label: 'Rename…',
        icon: Pencil,
        action: () => {
          prompt.value = {
            title: 'Rename stash',
            label: 'What is in it',
            initial: message,
            confirm: 'Rename',
            hint: 'The branch it was made on stays as it is.',
            run: (value) => {
              git.stashRename(index, value)
            }
          }
        }
      },
      {
        label: 'Turn into a branch…',
        icon: GitBranch,
        hint: 'safest',
        action: () =>
          (prompt.value = {
            title: 'Branch from stash',
            label: 'Branch name',
            confirm: 'Create branch',
            hint: 'Applies the stash onto a new branch and removes the entry.',
            run: (value) => git.stashBranch(index, value)
          })
      },
      { separator: true, label: '' },
      {
        label: 'Drop this stash',
        icon: Trash2,
        danger: true,
        action: () => git.stashDrop(index)
      }
    ],
    message
  )
}
</script>

<template>
  <aside class="side">
    <div class="filter">
      <Search :size="13" class="faint" />
      <!-- Esc empties the box rather than leaving a filter applied over a list
           nobody is looking at any more. -->
      <input
        v-model="filter"
        type="search"
        placeholder="Filter branches, requests, stashes"
        @keydown.esc.prevent="filter = ''"
      />
    </div>

    <div class="scroll">
      <!-- Local -->
      <button class="section-title toggle" @click="open.locals = !open.locals">
        <ChevronRight :size="12" class="chev" :class="{ down: open.locals }" />
        <HardDrive :size="12" class="mark" />
        Local
        <span class="count">{{ locals.length }}</span>
      </button>
      <div v-if="open.locals" class="group" :style="sizeOf('locals')">
        <template v-for="row in localShelf" :key="row.key">
          <button
            v-if="row.kind === 'folder'"
            class="row folder"
            :style="{ paddingLeft: `calc(var(--indent) + ${row.depth * 14}px)` }"
            @click="toggleFolder(row.path)"
          >
            <ChevronRight :size="11" class="chev" :class="{ down: !row.shut }" />
            <component :is="row.shut ? Folder : FolderOpen" :size="13" class="glyph" />
            <span class="name truncate">{{ row.label }}</span>
          </button>
          <div
            v-else
            class="row"
            :class="{
              on: row.item.is_head,
              dim: isHidden(row.item.name),
              drop: drag.state.over === `branch:${row.item.name}`
            }"
            :style="{ paddingLeft: `calc(var(--indent) + ${row.depth * 14}px)` }"
            :title="`${row.item.name}${row.item.upstream ? ` → ${row.item.upstream}` : ' (no upstream)'}`"
            draggable="true"
            @click="git.revealCommit(row.item.oid)"
            @dblclick="git.checkout(row.item.name)"
            @contextmenu="localMenu($event, row.item.name, row.item.upstream)"
            @dragstart="drag.begin($event, { kind: 'branch', name: row.item.name, remote: false })"
            @dragend="drag.end()"
            @dragover="drag.hover($event, `branch:${row.item.name}`, ['branch', 'commit', 'stash'])"
            @dragleave="drag.leave($event, `branch:${row.item.name}`)"
            @drop.prevent="onDropOnBranch($event, row.item.name, false)"
          >
            <GitBranch :size="13" class="glyph" :class="{ current: row.item.is_head }" />
            <!-- Cut in the middle: these names share a prefix far more often
                 than they share an ending, so the end is what tells them apart. -->
            <MidTruncate class="name" :text="row.label" />
            <!-- A text arrow at this size sits so close to the digit that "↑1"
                 reads as "11", so the arrow is a glyph with a gap of its own. -->
            <span
              v-if="row.item.ahead"
              class="tick up"
              :title="`${row.item.ahead} ahead of the upstream`"
            >
              <ArrowUp :size="11" :stroke-width="2.5" />{{ row.item.ahead }}
            </span>
            <span
              v-if="row.item.behind"
              class="tick down"
              :title="`${row.item.behind} behind the upstream`"
            >
              <ArrowDown :size="11" :stroke-width="2.5" />{{ row.item.behind }}
            </span>
            <Milestone
              v-if="row.item.name === trunk"
              :size="11"
              class="faint trunk-mark"
              :title="
                store.trunk.chosen
                  ? 'The main branch, as set here'
                  : 'Taken as the main branch — right-click another to change it'
              "
            />
            <Cloud v-if="!row.item.upstream" :size="11" class="faint no-upstream" />
          </div>
        </template>
      </div>
      <div
        v-if="open.locals"
        class="grip"
        title="Drag to resize · double-click to reset"
        @pointerdown="grab($event, 'locals')"
        @dblclick="resetSize('locals')"
      />

      <!-- Remote -->
      <div class="head-row">
        <button class="section-title toggle" @click="open.remotes = !open.remotes">
          <ChevronRight :size="12" class="chev" :class="{ down: open.remotes }" />
          <Cloud :size="12" class="mark" />
          Remote
          <span class="count">{{ remoteCount }}</span>
        </button>
        <button
          class="head-action"
          title="Add a remote"
          @click="remoteForm = 'add'"
        >
          <Plus :size="13" />
        </button>
      </div>
      <div
        v-if="open.remotes"
        class="group"
        :style="sizeOf('remotes')"
        @scroll.passive="onRemoteScroll"
      >
        <div v-for="group in remoteShelves" :key="group.remote">
          <div
            class="remote-name"
            :title="group.remote"
            @contextmenu="remoteHeaderMenu($event, group.remote)"
          >
            <Cloud :size="11" /> {{ group.remote }}
          </div>
          <template v-for="row in group.rows" :key="row.key">
            <button
              v-if="row.kind === 'folder'"
              class="row folder indent"
              :style="{ paddingLeft: `calc(var(--indent-2) + ${row.depth * 14}px)` }"
              @click="toggleFolder(row.path)"
            >
              <ChevronRight :size="11" class="chev" :class="{ down: !row.shut }" />
              <component :is="row.shut ? Folder : FolderOpen" :size="13" class="glyph" />
              <span class="name truncate">{{ row.label }}</span>
            </button>
            <div
              v-else
              class="row indent"
              :style="{ paddingLeft: `calc(var(--indent-2) + ${row.depth * 14}px)` }"
              :title="`${group.remote}/${row.item.name}`"
              draggable="true"
              @click="git.revealCommit(row.item.oid)"
              @dblclick="git.checkout(`${group.remote}/${row.item.name}`)"
              @contextmenu="remoteMenu($event, group.remote, row.item.name)"
              @dragstart="
                drag.begin($event, {
                  kind: 'branch',
                  name: `${group.remote}/${row.item.name}`,
                  remote: true
                })
              "
              @dragend="drag.end()"
            >
              <GitBranch :size="13" class="glyph remote" />
              <MidTruncate class="name" :text="row.label" />
            </div>
          </template>
        </div>
        <!-- The scroll asks for the next hundred on its own; this is for a
             section too short to scroll, and for saying how many are left. -->
        <button v-if="remoteMore" class="row more" @click="remoteShown += REMOTE_PAGE">
          <span class="name faint">{{ remoteMore }} more…</span>
        </button>
        <p v-if="!remoteGroups.length" class="none faint">No remote branches.</p>
      </div>
      <div
        v-if="open.remotes"
        class="grip"
        title="Drag to resize · double-click to reset"
        @pointerdown="grab($event, 'remotes')"
        @dblclick="resetSize('remotes')"
      />

      <!-- Pull requests -->
      <template v-if="forge.store.status && forge.store.status.kind !== 'none'">
        <div class="head-row">
          <button class="section-title toggle" @click="open.reviews = !open.reviews">
            <ChevronRight :size="12" class="chev" :class="{ down: open.reviews }" />
            <GitPullRequest :size="12" class="mark" />
            {{ forge.label.value }}
            <span class="count">{{ reviews.length }}</span>
          </button>
          <button
            class="head-action"
            :disabled="!forge.usable.value"
            :title="`Open a new ${forge.label.value.toLowerCase().replace(/s$/, '')} from this branch`"
            @click="creatingReview = true"
          >
            <Plus :size="13" />
          </button>
        </div>
        <div v-if="open.reviews" class="group" :style="sizeOf('reviews')">
          <div
            v-for="review in reviews"
            :key="review.number"
            class="row"
            :class="{ on: review.is_current || openReview?.number === review.number }"
            :title="reviewTitle(review)"
            @click="openReviewPage(review)"
            @dblclick="checkoutReview(review)"
            @contextmenu="
              menu.show(
                $event,
                [
                  {
                    label: `Read ${forge.sigil.value}${review.number} here`,
                    icon: GitPullRequest,
                    action: () => openReviewPage(review)
                  },
                  { label: 'Open in browser', icon: ExternalLink, action: () => forge.open(review.url) },
                  {
                    label: `Check out ${reviewBranch(review)}`,
                    icon: GitBranch,
                    action: () => checkoutReview(review)
                  },
                  { label: 'Copy link', icon: Copy, action: () => copyText(review.url, 'Link') }
                ],
                `${forge.sigil.value}${review.number} ${review.title.slice(0, 40)}`
              )
            "
          >
            <GitPullRequest :size="13" class="glyph pr" />
            <span class="name truncate">
              <span class="faint">{{ forge.sigil.value }}{{ review.number }}</span>
              {{ review.title }}
            </span>
            <span v-if="review.draft" class="tick faint">draft</span>
            <button
              class="row-action"
              title="Open in the browser"
              @click.stop="forge.open(review.url)"
            >
              <ExternalLink :size="12" />
            </button>
          </div>
          <p v-if="!forge.usable.value" class="none faint">
            {{
              forge.store.status?.has_token
                ? 'No remote on this forge to read.'
                : 'Add an access token to this profile in Settings to see them.'
            }}
          </p>
          <p v-else-if="forge.store.error" class="err">{{ forge.store.error }}</p>
          <p v-else-if="!reviews.length" class="none faint">
            {{ forge.store.loading ? 'Loading…' : 'Nothing open.' }}
          </p>
        </div>
        <div
          v-if="open.reviews"
          class="grip"
          title="Drag to resize · double-click to reset"
          @pointerdown="grab($event, 'reviews')"
          @dblclick="resetSize('reviews')"
        />
      </template>

      <!-- Tags -->
      <button class="section-title toggle" @click="open.tags = !open.tags">
        <ChevronRight :size="12" class="chev" :class="{ down: open.tags }" />
        <Tag :size="12" class="mark" />
        Tags
        <span class="count">{{ tags.length }}</span>
      </button>
      <div v-if="open.tags" class="group" :style="sizeOf('tags')">
        <div
          v-for="tag in tags"
          :key="tag.name"
          class="row"
          :title="describeTag(tag)"
          @click="git.revealCommit(tag.oid)"
          @contextmenu="tagMenu($event, tag.name, tag.oid)"
        >
          <Tag :size="12" class="glyph tag" />
          <MidTruncate class="name" :text="tag.name" />
          <span class="when faint">{{ relativeTime(tag.when) }}</span>
        </div>
        <p v-if="!tags.length" class="none faint">
          No tags. Right-click a commit in the graph to tag it.
        </p>
      </div>
      <div
        v-if="open.tags"
        class="grip"
        title="Drag to resize · double-click to reset"
        @pointerdown="grab($event, 'tags')"
        @dblclick="resetSize('tags')"
      />

      <!-- Stashes -->
      <button class="section-title toggle" @click="open.stashes = !open.stashes">
        <ChevronRight :size="12" class="chev" :class="{ down: open.stashes }" />
        <Archive :size="12" class="mark" />
        Stashes
        <span class="count">{{ stashes.length }}</span>
      </button>
      <!-- The last section takes whatever is left rather than a height of its
           own: there is nothing under it to divide from, so a cap on it would
           only leave the bottom of the sidebar empty. -->
      <div v-if="open.stashes" class="group last">
        <div
          v-for="stash in stashes"
          :key="stash.index"
          class="row stash"
          :title="`${stash.files} ${stash.files === 1 ? 'file' : 'files'} · ${stash.branch ?? ''}`"
          draggable="true"
          @click="git.selectStash(stash.index)"
          @dblclick="git.stashPop(stash.index)"
          @contextmenu="stashMenu($event, stash.index, stash.message)"
          @dragstart="
            drag.begin($event, { kind: 'stash', index: stash.index, message: stash.message })
          "
          @dragend="drag.end()"
        >
          <Archive :size="12" class="glyph" />
          <span class="names">
            <span class="name truncate">{{ stash.message }}</span>
            <span class="faint meta">
              {{ stash.branch }} · {{ stash.files }}
              {{ stash.files === 1 ? 'file' : 'files' }} · {{ relativeTime(stash.time) }}
            </span>
          </span>
        </div>
        <p v-if="!stashes.length" class="none faint">Nothing stashed.</p>
      </div>
    </div>

    <PromptDialog
      v-if="prompt"
      :title="prompt.title"
      :label="prompt.label"
      :initial="prompt.initial"
      :hint="prompt.hint"
      :confirm="prompt.confirm"
      :danger="prompt.danger"
      @close="prompt = null"
      @submit="
        (value) => {
          prompt?.run(value)
          prompt = null
        }
      "
    />

    <DeleteBranchDialog v-if="deleting" :name="deleting" @close="deleting = null" />
    <RemoteDialog
      v-if="remoteForm !== null"
      :name="remoteForm === 'add' ? undefined : remoteForm"
      @close="remoteForm = null"
    />
    <ReviewDialog v-if="creatingReview" @close="creatingReview = false" />
  </aside>
</template>

<style scoped>
.side {
  display: grid;
  /* Stated, so a long branch name scrolls inside the sidebar rather than
     widening it past the window. */
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: auto minmax(0, 1fr);
  background: var(--bg-panel);
  border-right: 1px solid var(--line);
}

.filter {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 8px;
  padding: 0 8px;
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: 5px;
}

.filter input {
  flex: 1;
  min-width: 0;
  border: none;
  background: none;
  padding: 5px 0;
  font-size: 12px;
}

.filter input:focus {
  outline: none;
}

/* The sections are stacked at the height they ask for and the sidebar scrolls
   when they do not all fit. Sharing the height out between them instead meant
   every section changed size whenever the window did, or whenever a section was
   folded open — five lists all shrinking at once to avoid one scrollbar, and
   none of them left tall enough to read. */
.scroll {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: 14px;
}

.scroll > .toggle,
.scroll > .head-row {
  flex: none;
}

/* As tall as what is in it, up to a cap: a section with three merge requests
   takes three rows, and one with forty branches takes the cap and scrolls
   inside itself rather than burying every heading below it. The cap is about
   nine rows — enough to work in, short enough that the usual five sections fit
   an ordinary window without the sidebar scrolling at all. Dragging a divider
   replaces both the height and the cap. */
.group {
  padding-top: 2px;
  flex: none;
  max-height: 250px;
  overflow-y: auto;
}

/* Fills the space the sections above leave, and keeps its content height when
   they leave none — at which point the sidebar itself scrolls, which is what it
   does whenever the sections together outgrow it. */
.group.last {
  flex: 1 1 auto;
  max-height: none;
}

/* The divider between two sections is also the handle for the one above it, so
   the grip draws that line itself rather than hovering in the space above it:
   it stands in for the heading's own rule, and takes the same height as the
   margin and padding it replaces. */
.grip {
  flex: none;
  height: 18px;
  cursor: row-resize;
  position: relative;
}

.grip::before {
  content: '';
  position: absolute;
  left: 0;
  right: 0;
  top: 8px;
  height: 1px;
  background: var(--line-soft);
}

.grip:hover::before,
.grip:active::before {
  height: 2px;
  background: var(--accent);
}

/* The last one has no section under it to divide from. */
.grip:last-child::before {
  display: none;
}


/* One indent scale for the whole tree. A row's glyph sits in the same column as
   its section's icon, so a name lines up under the name of the thing it belongs
   to, and a remote's branches step in once more under the remote. */
.side {
  --pad: 8px;
  --indent: 26px;
  --indent-2: 40px;
}

/* A line above each heading rather than below it, so the rule always sits
   between two sections. Under the heading it would cut a section off from its
   own rows, and a collapsed section would leave a line under nothing. */
.toggle {
  width: 100%;
  padding-left: var(--pad);
  text-align: left;
}

.scroll > .toggle:not(:first-child) {
  margin-top: 8px;
  padding-top: 10px;
  border-top: 1px solid var(--line-soft);
}

.toggle:hover {
  color: var(--text-dim);
}

.mark {
  flex: none;
  opacity: 0.75;
}

.chev {
  transition: transform 0.12s;
  flex: none;
}

.chev.down {
  transform: rotate(90deg);
}

.count {
  margin-left: auto;
  font-weight: 400;
}

/* The heading keeps the rule above it, so the button beside it has to sit
   inside that same box rather than after it. */
.head-row {
  display: flex;
  align-items: center;
}

.scroll > .head-row:not(:first-child) {
  margin-top: 8px;
  padding-top: 10px;
  border-top: 1px solid var(--line-soft);
}

.head-row > .toggle {
  flex: 1;
  min-width: 0;
}

.head-action {
  flex: none;
  margin-right: 6px;
  padding: 2px 4px;
  border-radius: 4px;
  color: var(--text-faint);
}

.head-action:hover:not(:disabled) {
  color: var(--text);
  background: var(--bg-hover);
}

.head-action:disabled {
  opacity: 0.35;
}

/* Only on the row the pointer is over: a link icon on every review would
   compete with the titles, which are what the list is for. */
.row-action {
  flex: none;
  margin-left: 2px;
  color: var(--text-faint);
  visibility: hidden;
}

.row:hover .row-action {
  visibility: visible;
}

.row-action:hover {
  color: var(--text);
}

.row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px 3px var(--indent);
  cursor: default;
  user-select: none;
}

.row:hover {
  background: var(--bg-hover);
}

.row.on {
  background: var(--bg-active);
}

.row.on .name {
  color: var(--text);
  font-weight: 600;
}

/* Sits where the next branch would, and reads as a note rather than as one
   more branch: the list carries on below it once it is asked to. */
.row.more {
  width: 100%;
  padding-left: var(--indent-2);
  cursor: pointer;
  font-style: italic;
}

/* Dimmed on purpose: the branch is still listed, still right-clickable, and
   still checks out — it has simply stopped asking to be read. Enough contrast
   left to scan, little enough that the eye passes over it. */
.row.dim .name,
.row.dim .glyph,
.row.dim .tick,
.row.dim .no-upstream {
  opacity: 0.42;
}

.row.dim:hover .name,
.row.dim:hover .glyph {
  opacity: 0.75;
}

.row.drop {
  outline: 1px solid var(--accent);
  outline-offset: -1px;
  background: color-mix(in srgb, var(--accent) 16%, transparent);
}

/* A folder is a button, so it has to be talked out of looking like one. */
.row.folder {
  width: 100%;
  background: none;
  border: 0;
  font: inherit;
  color: inherit;
  text-align: left;
}

.row.folder:hover {
  background: var(--bg-hover);
}

.row.folder .glyph {
  color: var(--text-dim);
}

.row.stash {
  align-items: flex-start;
  padding-top: 4px;
  padding-bottom: 4px;
}

.indent {
  padding-left: var(--indent-2);
}

.glyph {
  flex: none;
  color: var(--text-faint);
}

.glyph.current {
  color: var(--accent);
}

.glyph.remote {
  color: var(--purple-soft);
}

.glyph.tag {
  color: var(--amber);
}

.glyph.pr {
  color: var(--green);
}

.name {
  flex: 1;
  min-width: 0;
}

.names {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.meta {
  font-size: 10px;
}

.when {
  margin-left: auto;
  padding-left: 8px;
  font-size: 10px;
  flex: none;
}

.tick {
  display: inline-flex;
  align-items: center;
  gap: 1px;
  margin-left: 3px;
  font-size: 11px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  flex: none;
}

/* Lucide draws on a padded 24-unit grid, so centring the icon box against the
   digit leaves the stroke sitting high and crowding it. Nudge the glyph rather
   than the box. */
.tick svg {
  transform: translate(-1px, 1px);
}

.up {
  color: var(--green);
}

.down {
  color: var(--accent);
}

.no-upstream {
  flex: none;
  opacity: 0.5;
}

/* Quieter than the ahead/behind ticks beside it: this says what a branch is,
   not that something needs doing about it. */
.trunk-mark {
  flex: none;
  opacity: 0.55;
}

.remote-name {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px 2px var(--indent);
  font-size: 11px;
  color: var(--text-faint);
}

/* Right-clickable, so it says so on the way past. */
.remote-name:hover {
  color: var(--text-dim);
}

.none,
.err {
  padding: 3px 12px 5px var(--indent);
  font-size: 11.5px;
  margin: 0;
}

.err {
  color: var(--red);
}
/* A heading that follows a grip has had its rule drawn for it, so it drops the
   one it would otherwise carry. Last in the sheet, to outrank the rule above
   that gives every heading but the first a line of its own. */
.scroll > .grip + .toggle,
.scroll > .grip + .head-row {
  margin-top: 0;
  padding-top: 0;
  border-top: none;
}
</style>
