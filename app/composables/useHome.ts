import { invoke } from './useInvoke'
import { computed, reactive } from 'vue'
import type { Inbox } from './useForge'
import type { HomeAct } from './useForgeNudges'

/** One project on the home tab, as the backend reads it off disk. */
export interface RepoCard {
  path: string
  name: string
  branch: string
  ahead: number
  behind: number
  dirty: number
  /** Unix seconds of the last commit, or 0 when it could not be read. */
  last_commit: number
  /** False when the folder has been moved or deleted since it was opened. */
  exists: boolean
  /** Whether it has a remote at all. */
  has_remote?: boolean
  /** Branches that track nothing and hold commits no remote has. */
  unpushed?: { name: string; count: number }[]
  /** The trunk, when it is behind what it tracks. */
  trunk?: string | null
  trunk_behind?: number
  /** Stashes older than two weeks, and the oldest's age in days. */
  old_stashes?: number
  oldest_stash_days?: number
  /** Branches untouched for a month whose upstream has been deleted. */
  gone?: string[]
}

export interface HomeStats {
  /** Commits a day for the last 53 weeks, oldest first; the last is today. */
  days: number[]
  week: number
  previous_week: number
  streak: number
  best_streak: number
  read: number
  added: number
  removed: number
  repos_this_week: number
  favourite_word: string
  favourite_count: number
}

export interface HomeSummary {
  repos: RepoCard[]
  stats: HomeStats
  /** Whose commits were counted: the profile's address, or git's. */
  author: string | null
  /** Projects not counted, because there is no address to tell yours by. */
  uncounted?: string[]
}

const store = reactive({
  /** What the forges say is waiting on you, read after the projects. */
  inbox: null as Inbox | null,
  summary: null as HomeSummary | null,
  loading: false,
  error: null as string | null,
  /** When it was read, so opening the tab again does not re-read everything. */
  at: 0
})

/** Long enough that flipping between tabs is free; short enough to stay true. */
const FRESH_MS = 60_000

export function useHome() {
  const repos = computed(() => store.summary?.repos ?? [])
  const stats = computed(() => store.summary?.stats ?? null)

  /**
   * Reads every project the profile knows about.
   *
   * A handful of `git log` runs per repository, so it is asked for when the
   * page is opened rather than kept up to date in the background — and not
   * again for a minute, which is what makes leaving the tab and coming back
   * instant.
   */
  async function load(force = false) {
    if (store.loading) return
    if (!force && store.summary && Date.now() - store.at < FRESH_MS) return
    store.loading = true
    store.error = null
    try {
      store.summary = await invoke<HomeSummary>('home_summary')
      store.at = Date.now()
    } catch (error) {
      store.error = String(error)
    } finally {
      store.loading = false
    }
    // The forges are a network away and the projects are not: the page draws
    // first and the forge rows join it when they arrive.
    void invoke<Inbox>('forge_inbox')
      .then((inbox) => (store.inbox = inbox))
      .catch(() => {})
  }

  /** Forgets what was read, so the next open asks again. */
  function stale() {
    store.at = 0
  }

  return { store, repos, stats, load, stale }
}

/** What a row of the "needs a look" list can ask to be done. */
export interface WaitingAction {
  label: string
  /** Open the project and run its own flow there. */
  act?: HomeAct
  /** Take the project off the list. */
  forget?: boolean
  /** Open a page on the forge, for a review with no clone here. */
  url?: string
}

export type WaitingKind =
  | 'gone'
  | 'dirty'
  | 'ahead'
  | 'unpushed'
  | 'behind'
  | 'stash'
  | 'stale'
  | 'requested'
  | 'failing'
  | 'changes'

/** One thing worth doing something about, and what can be done. */
export interface Waiting {
  text: string
  where: string
  path: string
  kind: WaitingKind
  actions: WaitingAction[]
}

const plural = (count: number, one: string, many = `${one}s`) =>
  `${count} ${count === 1 ? one : many}`

/**
 * Everything across every project that needs you, most pressing first: what
 * the forges are waiting on you for, then what is only on this disk, then the
 * loose ends. Actions that write open the project and use its own flows.
 */
export function needsMe(repos: RepoCard[], inbox: Inbox | null): Waiting[] {
  const rows: Waiting[] = []
  const sigil = (host: string) => (host.includes('gitlab') ? '!' : '#')

  for (const row of inbox?.rows ?? []) {
    const name = `${row.repo} ${sigil(row.host)}${row.review.number}`
    const text =
      row.kind === 'requested'
        ? `Review requested: ${row.review.title}`
        : row.kind === 'failing'
          ? `Checks failing: ${row.review.title}`
          : `Changes requested: ${row.review.title}`
    const open: WaitingAction = row.path
      ? {
          label: 'Review',
          act: { path: row.path, action: 'review', review: row.review, repo: row.repo, host: row.host }
        }
      : { label: 'Open', url: row.review.url }
    rows.push({ text, where: name, path: row.path ?? '', kind: row.kind, actions: [open] })
  }

  for (const repo of repos) {
    if (!repo.exists) {
      rows.push({
        text: 'Folder is not there any more',
        where: repo.name,
        path: repo.path,
        kind: 'gone',
        actions: [{ label: 'Forget', forget: true }]
      })
      continue
    }
    const open: WaitingAction = { label: 'Open', act: { path: repo.path, action: 'open' } }
    if (repo.dirty) {
      rows.push({
        text: `${plural(repo.dirty, 'uncommitted change')}`,
        where: repo.name,
        path: repo.path,
        kind: 'dirty',
        actions: [open]
      })
    }
    if (repo.ahead) {
      rows.push({
        text: `${plural(repo.ahead, 'commit')} not pushed`,
        where: `${repo.name} · ${repo.branch}`,
        path: repo.path,
        kind: 'ahead',
        actions: [
          { label: 'Push', act: { path: repo.path, action: 'push', branch: repo.branch } }
        ]
      })
    }
    for (const branch of repo.unpushed ?? []) {
      rows.push({
        text: `${plural(branch.count, 'commit')} never pushed`,
        where: `${repo.name} · ${branch.name}`,
        path: repo.path,
        kind: 'unpushed',
        actions: [
          { label: 'Push', act: { path: repo.path, action: 'push', branch: branch.name } }
        ]
      })
    }
    if (repo.trunk && repo.trunk_behind) {
      rows.push({
        text: `${repo.trunk} is ${plural(repo.trunk_behind, 'commit')} behind`,
        where: repo.name,
        path: repo.path,
        kind: 'behind',
        actions: [
          { label: 'Pull', act: { path: repo.path, action: 'pull', branch: repo.trunk } }
        ]
      })
    }
    if (repo.old_stashes) {
      rows.push({
        text: `${plural(repo.old_stashes, 'stash', 'stashes')} older than two weeks`,
        where: `${repo.name} · oldest ${repo.oldest_stash_days} days`,
        path: repo.path,
        kind: 'stash',
        actions: [open]
      })
    }
    if (repo.gone?.length) {
      rows.push({
        text: `${plural(repo.gone.length, 'branch', 'branches')} idle for a month, gone from the remote`,
        where: `${repo.name} · ${repo.gone.slice(0, 3).join(', ')}`,
        path: repo.path,
        kind: 'stale',
        actions: [open]
      })
    }
  }
  return rows
}

/** 4200 as "4.2k", because the tiles are one line each. */
export function short(count: number) {
  if (count < 1000) return String(count)
  return `${(count / 1000).toFixed(1)}k`
}

/** How long ago, in the words the rest of the window uses. */
export function ago(seconds: number) {
  if (!seconds) return ''
  const mins = Math.floor((Date.now() / 1000 - seconds) / 60)
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins}m ago`
  const hours = Math.floor(mins / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  if (days === 1) return 'yesterday'
  if (days < 14) return `${days} days ago`
  const weeks = Math.floor(days / 7)
  if (weeks < 9) return `${weeks} weeks ago`
  const months = Math.floor(days / 30)
  return months < 24 ? `${months} months ago` : `${Math.floor(days / 365)} years ago`
}
