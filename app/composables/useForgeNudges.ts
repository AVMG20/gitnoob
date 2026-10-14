import { invoke } from './useInvoke'
import { useConfig } from './useConfig'
import { useForge, type Review } from './useForge'
import { useGit } from './useGit'
import { useReview } from './useReview'
import { useToasts } from './useToasts'

/**
 * The forge doing a little of the typing for you.
 *
 * Each of these is a thing everybody does by hand after the same moment: open a
 * pull request after the first push, tidy up after a merge, notice that this
 * repository belongs to the other account. They happen where they cannot lose
 * anything; everything else is offered as a notice whose button is the
 * decision.
 */

/** One commit a new review would carry. */
export interface DraftCommit {
  subject: string
  body: string
}

/** What a new review is started with. */
export interface ReviewStart {
  title: string
  body: string
}

/**
 * The issue a branch name points at, as the description should say it.
 *
 * `123-fix-login` and `fix/123-login` close issue 123 on both forges, so the
 * description says so in the words they act on. A tracker key like `ABC-123`
 * belongs to some other tracker: it is kept as text for that tracker's own
 * integration to find, and nothing is claimed about closing it.
 */
export function issueReference(branch: string): string | null {
  const leaf = branch.split('/').pop() ?? branch
  const key = branch.match(/(?:^|[/_-])([A-Z][A-Z0-9]+-\d+)(?=$|[/_-])/)
  if (key) return key[1]!
  const number = leaf.match(/^(\d+)(?:[-_]|$)/)
  if (number) return `Closes #${number[1]}`
  return null
}

/** `fix/login-crash` as "Fix login crash", for a title nobody has typed yet. */
export function titleFromBranch(branch: string): string {
  const leaf = (branch.split('/').pop() ?? branch)
    .replace(/^[A-Z][A-Z0-9]+-\d+[-_]?/, '')
    .replace(/^\d+[-_]/, '')
  const words = leaf.replace(/[-_]+/g, ' ').trim()
  if (!words) return branch
  return words.charAt(0).toUpperCase() + words.slice(1)
}

/**
 * The title and description a new review starts with.
 *
 * One commit is the whole argument, so its subject and body are the title and
 * the description. More than one, and the branch name is the title and the
 * description lists what each commit did. The issue the branch names goes at
 * the end either way.
 */
export function draftFromCommits(branch: string, commits: DraftCommit[]): ReviewStart {
  const reference = issueReference(branch)
  let title: string
  let body: string
  if (commits.length === 1) {
    title = commits[0]!.subject
    body = commits[0]!.body
  } else {
    title = titleFromBranch(branch)
    body = commits.map((one) => `- ${one.subject}`).join('\n')
  }
  if (reference && !body.includes(reference)) body = body ? `${body}\n\n${reference}` : reference
  return { title, body }
}

/** The one-word state a review's checks are in, for the notices. */
type Standing = Pick<Review, 'number' | 'title' | 'author' | 'checks' | 'review_state'>

/** Something worth telling somebody whose pull request it is. */
export interface ForgeChange {
  number: number
  title: string
  text: string
}

/**
 * What changed between two readings of the same project's reviews that the
 * author would want to hear about: checks finishing or failing, a review
 * approving or asking for changes. Only for `me`'s reviews, and never for the
 * first reading — a list that has just loaded is not news.
 */
export function changes(before: Standing[] | null, after: Standing[], me: string | null): ForgeChange[] {
  if (!before || !me) return []
  const out: ForgeChange[] = []
  for (const now of after) {
    if (now.author !== me) continue
    const was = before.find((one) => one.number === now.number)
    if (!was) continue
    if (was.checks === 'pending' && now.checks === 'success') {
      out.push({ number: now.number, title: now.title, text: 'Checks passed' })
    } else if (was.checks !== 'failure' && now.checks === 'failure') {
      out.push({ number: now.number, title: now.title, text: 'Checks failed' })
    }
    if (was.review_state !== now.review_state) {
      if (now.review_state === 'approved') {
        out.push({ number: now.number, title: now.title, text: 'Approved' })
      } else if (now.review_state === 'changes_requested') {
        out.push({ number: now.number, title: now.title, text: 'Changes requested' })
      }
    }
  }
  return out
}

/** How long to wait before asking again while checks are still running:
    doubling from twenty seconds to two minutes. */
export function nextDelay(previous: number | null): number {
  if (!previous) return 20_000
  return Math.min(previous * 2, 120_000)
}

/* ---------- after a merge ------------------------------------------------- */

/**
 * Catches the clone up with a merge just made from here, and offers to delete
 * the branch that carried it.
 *
 * Deleting is offered, not done, and the commit it pointed at is kept so the
 * notice that says it is gone can put it back.
 */
export async function offerBranchCleanup(source: string, head: string | null) {
  const git = useGit()
  const review = useReview()
  const toasts = useToasts()
  const done = await review.afterMerge(source, head)
  await git.refresh()
  if (!done) return
  if (done.message) git.note(done.message)
  const branch = done.branch
  const sha = done.sha
  if (!branch || !sha) return
  toasts.offer(
    `Delete the branch ${branch}?`,
    [
      {
        label: 'Delete it',
        run: async () => {
          try {
            const said = await invoke<string>('review_delete_merged', { name: branch, sha })
            await git.refresh()
            toasts.offer(said, [
              {
                label: 'Undo',
                run: async () => {
                  try {
                    git.note(await invoke<string>('review_restore_branch', { name: branch, sha }))
                  } catch (error) {
                    toasts.fail(String(error))
                  }
                  await git.refresh()
                }
              }
            ])
          } catch (error) {
            toasts.fail(String(error))
          }
        }
      },
      { label: 'Keep it', run: () => {} }
    ],
    'Its work is merged. Deleting only removes the name here; Undo puts it back.'
  )
}

/* ---------- after a first push -------------------------------------------- */

/**
 * After a branch's first push, offers to open a pull request for it.
 *
 * Not for the trunk, not when one is already open, not when the setting is
 * off. The dialog it opens is the ordinary one, already filled in.
 */
export async function offerReviewAfterPush(branch: string, setUpstream: boolean) {
  if (!setUpstream || !branch) return
  const config = useConfig()
  if (config.settings.value?.offer_review_after_push === false) return
  const forge = useForge()
  const git = useGit()
  if (!forge.usable.value) return
  const facts = await forge.loadFacts()
  const trunk = git.store.trunk.name?.replace(/^[^/]+\//, '') ?? null
  const target = facts?.default_branch || trunk || 'main'
  if (branch === target || branch === trunk) return
  await forge.loadReviews()
  if (forge.store.reviews.some((one) => one.source_branch === branch && one.is_current)) return
  if (forge.store.reviews.some((one) => one.source_branch === branch && !one.source?.is_fork)) return

  const toasts = useToasts()
  const label = forge.shortLabel.value
  toasts.offer(`Open a ${label} for ${branch}?`, [
    {
      label: `Open ${label}`,
      run: async () => {
        const commits = await invoke<DraftCommit[]>('review_branch_commits', {
          source: branch,
          target
        }).catch(() => [] as DraftCommit[])
        const start = draftFromCommits(branch, commits)
        forge.compose({ source: branch, target, title: start.title, body: start.body })
      }
    },
    { label: 'Not now', run: () => {} }
  ])
}

/**
 * The one call a push makes into all this: after a push that set an
 * upstream, the offer above. Never awaited by the push and never able to
 * fail it.
 */
export function afterPush(branch: string, setUpstream: boolean, ok: boolean) {
  if (!ok) return
  void offerReviewAfterPush(branch, setUpstream).catch(() => {})
}

/** A notification from the system, when it lets the app send one. */
export async function notify(title: string, body: string) {
  try {
    const system = await import('@tauri-apps/plugin-notification')
    let granted = await system.isPermissionGranted()
    if (!granted) granted = (await system.requestPermission()) === 'granted'
    if (granted) system.sendNotification({ title, body })
  } catch {
    // Outside the app window there is no system to tell.
  }
}

/* ---------- from the home tab --------------------------------------------- */

/** Something a row on the home tab asked for, in the project it is about. */
export interface HomeAct {
  path: string
  action: 'open' | 'push' | 'pull' | 'open-pr' | 'review'
  branch?: string
  review?: Review
  /** The forge project and host a review row is about: `owner/name`. */
  repo?: string
  host?: string
}

/**
 * Does what a home row asked, once its project is the one open.
 *
 * Every one of these is the project's own flow — the same push, pull, dialog
 * or review page its own buttons start — so it runs in the project's tab
 * where its result can be seen, never quietly from the home page.
 */
export async function runHomeAct(act: HomeAct) {
  const git = useGit()
  const forge = useForge()
  if (git.store.repo?.path !== act.path) return
  switch (act.action) {
    case 'push':
      if (act.branch) await git.requestPush(act.branch, true)
      return
    case 'pull':
      if (act.branch) await git.pullBranch(act.branch)
      return
    case 'open-pr': {
      if (!act.branch) return
      const facts = await forge.loadFacts()
      const target = facts?.default_branch || git.store.trunk.name?.replace(/^[^/]+\//, '') || 'main'
      const commits = await invoke<DraftCommit[]>('review_branch_commits', {
        source: act.branch,
        target
      }).catch(() => [] as DraftCommit[])
      const start = draftFromCommits(act.branch, commits)
      forge.compose({ source: act.branch, target, ...start })
      return
    }
    case 'review': {
      if (!act.review) return
      // A number names a review only within its project. The folder's reviews
      // have to come from the project the row is about, or #12 here is some
      // other #12.
      await forge.refreshStatus()
      if (!sameProject(act, forge.store.status)) {
        git.note(
          `${act.repo ?? 'That review'} is not the project this folder's reviews come from, so it was not opened here`,
          'error'
        )
        return
      }
      await forge.loadReviews()
      const listed = forge.store.reviews.find((one) => one.number === act.review!.number)
      useReview().show(listed ?? act.review)
      return
    }
    default:
  }
}

/** Whether a home row's review is in the forge project the folder answers to. */
export function sameProject(
  act: Pick<HomeAct, 'repo' | 'host'>,
  status: { host: string; slug: { owner: string; name: string } | null } | null
) {
  if (!act.repo || !status?.slug) return !act.repo
  const bare = (host: string) =>
    host
      .toLowerCase()
      .replace(/:\d+$/, '')
      .replace(/^(ssh|altssh|www)\.(?=.+\.)/, '')
  const here = `${status.slug.owner}/${status.slug.name}`.toLowerCase()
  return here === act.repo.toLowerCase() && (!act.host || bare(act.host) === bare(status.host))
}

/* ---------- which profile ------------------------------------------------- */

export interface ProfileChoice {
  id: string
  name: string
}

export interface ProfileMatch {
  host: string | null
  repo: string | null
  using: ProfileChoice | null
  bound: boolean
  fits: boolean
  best: ProfileChoice[]
}

export interface IdentityCheck {
  local_name: string | null
  local_email: string | null
  profile: string | null
  name: string | null
  email: string | null
  fits: boolean
  differs: boolean
}

/** Puts a repository under another profile and reads the forge again. */
async function useProfileFor(path: string, choice: ProfileChoice) {
  const config = useConfig()
  const forge = useForge()
  const git = useGit()
  await config.setRepoProfile(path, choice.id)
  await forge.refreshStatus()
  void forge.loadReviews()
  git.note(`This repository now uses the ${choice.name} profile`)
  await askAboutIdentity()
}

/**
 * When a repository opens: is it on the forge the profile is for?
 *
 * With exactly one profile that fits better, the setting decides — switch to
 * it for this repository, ask, or say nothing. The active profile stays what
 * it is everywhere else.
 */
export async function checkProfile() {
  const git = useGit()
  const path = git.store.repo?.path
  if (!path) return
  const config = useConfig()
  const setting = config.settings.value?.profile_match ?? 'ask'
  const found = await invoke<ProfileMatch>('profile_match').catch(() => null)
  if (!found || git.store.repo?.path !== path) return
  if (!found.fits && found.best.length === 1 && setting !== 'off') {
    const choice = found.best[0]!
    if (setting === 'switch') {
      await useProfileFor(path, choice)
      return
    }
    useToasts().offer(
      `Use the ${choice.name} profile here?`,
      [
        { label: `Use ${choice.name}`, run: () => useProfileFor(path, choice) },
        { label: 'Not now', run: () => {} }
      ],
      `This repository is on ${found.host}, which the ${found.using?.name ?? 'active'} profile is not for.`
    )
    return
  }
  await askAboutIdentity()
}

/**
 * Asks before a repository's own commit address is replaced by the profile's.
 * Opening a repository used to rewrite it without a word.
 */
export async function askAboutIdentity() {
  const git = useGit()
  const path = git.store.repo?.path
  const found = await invoke<IdentityCheck>('identity_check').catch(() => null)
  if (!found || !found.fits || !found.differs || !found.email || git.store.repo?.path !== path) return
  const config = useConfig()
  useToasts().offer(
    `Commit here as ${found.email}?`,
    [
      {
        label: 'Use the profile’s',
        run: async () => {
          const said = await config.applyIdentity(true).catch((error) => String(error))
          if (said) git.note(said)
          await git.refresh()
        }
      },
      { label: `Keep ${found.local_email}`, run: () => {} }
    ],
    `This repository commits as ${found.local_email}; the ${found.profile} profile says ${found.email}.`
  )
}
