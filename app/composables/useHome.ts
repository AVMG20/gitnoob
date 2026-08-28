import { invoke } from './useInvoke'
import { computed, reactive } from 'vue'

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
}

export interface HomeStats {
  /** Commits a day for the last 53 weeks, oldest first; the last is today. */
  days: number[]
  week: number
  previous_week: number
  streak: number
  best_streak: number
  /** 1–7, Monday first. Zero when there is nothing to say. */
  busy_weekday: number
  busy_hour: number
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
  /** Whose commits were counted, when the profile names an address. */
  author: string | null
}

const store = reactive({
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
  }

  /** Forgets what was read, so the next open asks again. */
  function stale() {
    store.at = 0
  }

  return { store, repos, stats, load, stale }
}

const WEEKDAYS = ['', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

/** "Thu 16:00", or an empty string when no commit has been read. */
export function busiestLabel(stats: HomeStats | null) {
  if (!stats?.busy_weekday) return ''
  return `${WEEKDAYS[stats.busy_weekday] ?? ''} ${String(stats.busy_hour).padStart(2, '0')}:00`
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
