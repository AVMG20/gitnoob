<script setup lang="ts">
import { onMounted, ref } from 'vue'
import HomePane from './HomePane.vue'
import ProjectTabs from './ProjectTabs.vue'
import ContextMenu from './ContextMenu.vue'
import { useConfig } from '~/composables/useConfig'

/**
 * The home tab on fixture data, at `?lab=home` on the dev server.
 *
 * The page itself is the real one: only the answers are made up, so what is
 * looked at here is what ships. A year of commits is the one thing that cannot
 * be sensibly faked by hand, so it is generated — quiet weekends, a fortnight
 * off in the middle, busier lately.
 */
const config = useConfig()
const ready = ref(false)

const PROJECTS = [
  ['gitui', '/Users/robin/tools/gitui'],
  ['storefront-returns-admin', '/Users/robin/sites/storefront-returns-admin'],
  ['tracker', '/Users/robin/tools/tracker'],
  ['marketwatch', '/Users/robin/tools/marketwatch'],
  ['fieldwork', '/Users/robin/sites/fieldwork'],
  ['harbour-lights', '/Users/robin/sites/harbour-lights'],
  ['storefront', '/Users/robin/sites/storefront'],
  ['northwind-api', '/Users/robin/sites/northwind-api'],
  ['catalog-toolbox', '/Users/robin/tools/catalog-toolbox'],
  ['dotfiles', '/Users/robin/dotfiles']
] as const

const CARDS = [
  { branch: 'main', ahead: 0, behind: 0, dirty: 12, days: 0 },
  { branch: 'tickets', ahead: 6, behind: 0, dirty: 0, days: 0 },
  { branch: 'main', ahead: 0, behind: 3, dirty: 0, days: 0 },
  { branch: 'feature/odds', ahead: 2, behind: 0, dirty: 3, days: 1 },
  { branch: 'main', ahead: 0, behind: 0, dirty: 0, days: 3 },
  // A folder that has been moved or deleted since it was last opened, so the
  // page can be looked at with something actually wrong on it.
  { branch: '', ahead: 0, behind: 0, dirty: 0, days: 8, gone: true },
  { branch: 'main', ahead: 0, behind: 1, dirty: 0, days: 9 },
  { branch: 'main', ahead: 0, behind: 0, dirty: 0, days: 15 },
  { branch: 'main', ahead: 1, behind: 0, dirty: 0, days: 24 },
  { branch: 'main', ahead: 0, behind: 0, dirty: 1, days: 40 }
] as { branch: string; ahead: number; behind: number; dirty: number; days: number; gone?: boolean }[]

/** A fixed seed, so the page looks the same every time it is opened. */
function random(seed: number) {
  let state = seed >>> 0
  return () => {
    state = (state + 0x6d2b79f5) >>> 0
    let value = Math.imul(state ^ (state >>> 15), 1 | state)
    value = (value + Math.imul(value ^ (value >>> 7), 61 | value)) ^ value
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296
  }
}

function year() {
  const next = random(20260828)
  return Array.from({ length: 371 }, (_, at) => {
    const quiet = at % 7 === 5 || at % 7 === 6
    const lately = 0.45 + (at / 371) * 0.9
    if (at > 150 && at < 172) return 0
    if (next() > (quiet ? 0.28 : 0.82) * lately) return 0
    return Math.max(1, Math.round(next() * 9 * lately * (quiet ? 0.4 : 1)))
  })
}

function summary() {
  const days = year()
  const now = Math.floor(Date.now() / 1000)
  return {
    repos: PROJECTS.map(([name, path], at) => ({
      path,
      name,
      branch: CARDS[at]!.branch,
      ahead: CARDS[at]!.ahead,
      behind: CARDS[at]!.behind,
      dirty: CARDS[at]!.dirty,
      last_commit: now - CARDS[at]!.days * 86_400 - 3600,
      exists: !CARDS[at]!.gone
    })),
    stats: {
      days,
      week: days.slice(-7).reduce((all, one) => all + one, 0),
      previous_week: days.slice(-14, -7).reduce((all, one) => all + one, 0),
      streak: 9,
      best_streak: 14,
      read: days.reduce((all, one) => all + one, 0),
      added: 4231,
      removed: 1804,
      repos_this_week: 4,
      favourite_word: 'fix',
      favourite_count: 214
    },
    author: 'robin@example.com'
  }
}

/**
 * How long the read is held back, from `&slow=1200` on the address.
 *
 * The outline the page draws while it waits is the hardest part of it to look
 * at, because on a real machine it is gone in under a second. Off by default:
 * everything else here wants the answers straight away.
 */
const HOLD = Number(new URLSearchParams(location.search).get('slow') ?? 0)

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

/** Answers the commands this page sends; everything else is empty. */
function install() {
  const internals = ((window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ ??=
    {}) as Record<string, unknown>
  let answer = summary()
  internals.invoke = async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'home_summary') {
      if (HOLD) await wait(HOLD)
      return answer
    }
    if (cmd === 'project_forget') {
      const path = String(args?.path ?? '')
      answer = { ...answer, repos: answer.repos.filter((one) => one.path !== path) }
      return null
    }
    if (cmd === 'config_get') {
      return {
        version: 1,
        active_profile: 'p1',
        global: { show_avatars: true, graph_page_size: 500 },
        profiles: [
          {
            id: 'p1',
            name: 'Work',
            forge: 'none',
            host: '',
            git_name: 'Robin Vale',
            git_email: 'robin@example.com',
            ssh_key: null,
            signing_key: null,
            signing_format: null,
            sign_commits: null,
            sign_tags: null,
            projects: PROJECTS.slice(0, 3).map(([name, path]) => ({ name, path })),
            recents: PROJECTS.map(([name, path]) => ({ name, path })),
            active_project: PROJECTS[0]![1]
          }
        ]
      }
    }
    return null
  }
}

install()
onMounted(async () => {
  await config.load()
  ready.value = true
})
</script>

<template>
  <div class="lab">
    <ProjectTabs home @open="() => {}" @clone="() => {}" @init="() => {}" @home="() => {}" />
    <HomePane v-if="ready" @open="() => {}" @clone="() => {}" @init="() => {}" />
    <ContextMenu />
  </div>
</template>

<style scoped>
.lab {
  display: flex;
  flex-direction: column;
  height: 100vh;
  min-height: 0;
}
</style>
