<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { open as pickFolder } from '@tauri-apps/plugin-dialog'
import { FolderOpen, GitBranch, Plus, RefreshCw, Search } from 'lucide-vue-next'
import { ago, busiestLabel, short, useHome, type RepoCard } from '~/composables/useHome'
import { useConfig } from '~/composables/useConfig'
import { SHORTCUTS, keyLabel } from '~/composables/useShortcuts'

/**
 * The home tab: every project, and a year of your own commits.
 *
 * The tab strip runs out of window long before anybody runs out of
 * repositories, so this is where all of them live — and, since it is reading
 * them anyway, where the things that are true across all of them can be said:
 * what is uncommitted, what has not been pushed, and how the year has gone.
 */
const emit = defineEmits<{ open: [string]; clone: []; init: [] }>()

const home = useHome()
const config = useConfig()

const filter = ref('')
const stats = computed(() => home.stats.value)

const shown = computed(() => {
  const words = filter.value.toLowerCase().split(/\s+/).filter(Boolean)
  return home.repos.value.filter((one) =>
    words.every((word) => `${one.name} ${one.path}`.toLowerCase().includes(word))
  )
})

/** Which tabs are open, so the list can say so instead of a date. */
const open = computed(() => new Set(config.projects.value.map((one) => one.path)))

const tiles = computed(() => {
  const found = stats.value
  if (!found) return []
  const change = found.week - found.previous_week
  return [
    {
      value: String(found.week),
      label: 'commits this week',
      hint: change === 0 ? 'same as last week' : `${change > 0 ? '+' : ''}${change} on last week`
    },
    {
      value: found.streak === 1 ? '1 day' : `${found.streak} days`,
      label: 'commit streak',
      hint: found.best_streak ? `best: ${found.best_streak}` : ''
    },
    {
      value: busiestLabel(found) || '—',
      label: 'when you work',
      hint: found.read ? `${found.read} commits read` : ''
    },
    {
      value: `+${short(found.added)} / −${short(found.removed)}`,
      label: 'lines this week',
      hint: found.repos_this_week
        ? `${found.repos_this_week} ${found.repos_this_week === 1 ? 'repository' : 'repositories'}`
        : ''
    }
  ]
})

/** The last 53 weeks as columns of seven, which is how the grid is drawn. */
const weeks = computed(() => {
  const days = stats.value?.days ?? []
  const out: number[][] = []
  for (let at = 0; at < days.length; at += 7) out.push(days.slice(at, at + 7))
  return out
})

/** Five steps, so a busy day and a quiet one are told apart at a glance. */
function level(count: number) {
  if (!count) return 0
  if (count <= 1) return 1
  if (count <= 3) return 2
  if (count <= 6) return 3
  return 4
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
/** A label under the column each month starts in, read off today's date. */
const months = computed(() => {
  const total = weeks.value.length
  const out: { at: number; name: string }[] = []
  const now = new Date()
  for (let back = 0; back < total; back += 1) {
    const when = new Date(now)
    when.setDate(now.getDate() - back * 7)
    const at = total - 1 - back
    const name = MONTHS[when.getMonth()] ?? ''
    if (!out.some((one) => one.name === name)) out.push({ at, name })
  }
  return out.reverse()
})

/** What is worth doing something about, in the order it is worth doing it. */
const attention = computed(() => {
  const rows: { text: string; where: string; path: string; action: string }[] = []
  for (const repo of home.repos.value) {
    if (!repo.exists) {
      rows.push({
        text: 'Folder is not there any more',
        where: repo.name,
        path: repo.path,
        action: 'Forget'
      })
      continue
    }
    if (repo.dirty) {
      rows.push({
        text: `${repo.dirty} uncommitted ${repo.dirty === 1 ? 'change' : 'changes'}`,
        where: repo.name,
        path: repo.path,
        action: 'Open'
      })
    }
    if (repo.ahead) {
      rows.push({
        text: `${repo.ahead} ${repo.ahead === 1 ? 'commit' : 'commits'} not on origin`,
        where: `${repo.name} · ${repo.branch}`,
        path: repo.path,
        action: 'Open'
      })
    }
  }
  return rows.slice(0, 6)
})

/** Three shortcuts worth knowing, picked from the list settings draws. */
const tips = ['project.switch', 'diff.mode', 'history.undo']
  .map((id) => SHORTCUTS.find((one) => one.id === id))
  .filter((one): one is (typeof SHORTCUTS)[number] => !!one)

const hour = new Date().getHours()
const greeting = hour < 6 ? 'Still up' : hour < 12 ? 'Good morning' : hour < 18 ? 'Good afternoon' : 'Good evening'
const who = computed(() => config.profile.value?.git_name?.split(' ')[0] ?? '')

const fun = computed(() => {
  const found = stats.value
  if (!found) return ''
  const bits: string[] = []
  if (found.favourite_count > 2) {
    bits.push(`“${found.favourite_word}” starts ${found.favourite_count} of your subjects`)
  }
  if (found.best_streak) bits.push(`longest streak ${found.best_streak} days`)
  if (found.read) bits.push(`${found.read} commits in the last year`)
  return bits.join(' · ')
})

/** The same folder picker the welcome pane and the tab strip use. */
async function pick() {
  const path = await pickFolder({ directory: true, multiple: false, title: 'Open a repository' })
  if (typeof path === 'string') emit('open', path)
}

function choose(repo: RepoCard) {
  if (!repo.exists) return
  emit('open', repo.path)
}

onMounted(() => home.load())
</script>

<template>
  <div class="home">
    <div class="sheet">
      <header class="top">
        <div class="titles">
          <h1>{{ greeting }}{{ who ? `, ${who}` : '' }}</h1>
          <p class="sub">
            {{ home.repos.value.length }}
            {{ home.repos.value.length === 1 ? 'repository' : 'repositories' }}
            <template v-if="open.size"> · {{ open.size }} open</template>
            <template v-if="stats"> · {{ stats.week }} commits this week</template>
          </p>
        </div>
        <div class="actions">
          <button class="btn" title="Read everything again" @click="home.load(true)">
            <RefreshCw :size="13" :class="{ spin: home.store.loading }" />
          </button>
          <button class="btn" @click="pick"><FolderOpen :size="13" /> Open</button>
          <button class="btn" @click="emit('clone')"><Plus :size="13" /> Clone</button>
        </div>
      </header>

      <p v-if="home.store.error" class="oops">{{ home.store.error }}</p>

      <section v-if="tiles.length" class="strip">
        <div v-for="tile in tiles" :key="tile.label" class="stat">
          <span class="stat-value">{{ tile.value }}</span>
          <span class="stat-label">{{ tile.label }}</span>
          <span v-if="tile.hint" class="stat-hint">{{ tile.hint }}</span>
        </div>
      </section>

      <section v-if="weeks.length" class="card year">
        <div class="head">
          <span class="head-title">A year of commits, across everything</span>
          <span v-if="home.store.summary?.author" class="head-note mono">
            {{ home.store.summary.author }}
          </span>
        </div>
        <div class="grid">
          <div v-for="(week, at) in weeks" :key="at" class="week">
            <span
              v-for="(day, index) in week"
              :key="index"
              class="cell"
              :class="`l${level(day)}`"
              :title="`${day} ${day === 1 ? 'commit' : 'commits'}`"
            />
          </div>
        </div>
        <div class="months">
          <span
            v-for="month in months"
            :key="month.name"
            class="month"
            :style="{ left: `${(month.at / weeks.length) * 100}%` }"
          >
            {{ month.name }}
          </span>
        </div>
      </section>

      <div class="columns">
        <section class="card">
          <div class="head">
            <span class="head-title">Projects</span>
            <label class="find">
              <Search :size="12" />
              <input v-model="filter" type="text" placeholder="Filter" />
            </label>
          </div>

          <p v-if="home.store.loading && !shown.length" class="empty">Reading your repositories…</p>
          <p v-else-if="!shown.length" class="empty">
            {{ filter ? 'Nothing matches.' : 'Nothing opened yet — Open or Clone one.' }}
          </p>

          <ul class="rows">
            <li
              v-for="repo in shown"
              :key="repo.path"
              class="row"
              :class="{ gone: !repo.exists }"
              @click="choose(repo)"
            >
              <span class="name">{{ repo.name }}</span>
              <MidTruncate class="path" :text="repo.path" :tail="12" />
              <span class="branch">
                <GitBranch :size="11" />
                {{ repo.exists ? repo.branch : 'not on disk' }}
              </span>
              <span class="meta">
                <span v-if="repo.ahead" :title="`${repo.ahead} to push`">↑{{ repo.ahead }}</span>
                <span v-if="repo.behind" :title="`${repo.behind} to pull`">↓{{ repo.behind }}</span>
                <span v-if="repo.dirty" class="on" :title="`${repo.dirty} changed`">
                  ●{{ repo.dirty }}
                </span>
              </span>
              <span class="when">
                {{ open.has(repo.path) ? 'open' : ago(repo.last_commit) }}
              </span>
            </li>
          </ul>
        </section>

        <div class="side">
          <section class="card">
            <div class="head"><span class="head-title">Needs a look</span></div>
            <ul class="rows">
              <li v-for="one in attention" :key="`${one.path}${one.text}`" class="line">
                <span class="line-main">{{ one.text }}</span>
                <span class="line-where">{{ one.where }}</span>
                <button class="link" @click="emit('open', one.path)">{{ one.action }}</button>
              </li>
              <li v-if="!attention.length" class="line quiet">
                Nothing waiting — everything is committed and pushed.
              </li>
            </ul>
          </section>

          <section class="card">
            <div class="head"><span class="head-title">Did you know</span></div>
            <ul class="rows">
              <li v-for="tip in tips" :key="tip.id" class="line tip">
                <kbd class="cap">{{ keyLabel(tip.keys) }}</kbd>
                <span class="line-main">{{ tip.label }}.</span>
              </li>
            </ul>
          </section>
        </div>
      </div>

      <footer v-if="fun" class="footnote">{{ fun }}</footer>
    </div>
  </div>
</template>

<style scoped>
.home {
  flex: 1;
  min-height: 0;
  overflow: auto;
  background: var(--bg);
  color: var(--text);
}

/* One column down the middle, so the page has an edge to line up against
   however wide the window is. */
.sheet {
  max-width: 1280px;
  margin: 0 auto;
  padding: 30px 30px 44px;
}

.top {
  display: flex;
  align-items: flex-end;
  gap: 16px;
  margin-bottom: 24px;
}

.titles {
  flex: 1;
  min-width: 0;
}

h1 {
  margin: 0;
  font-size: 20px;
  font-weight: 600;
  letter-spacing: -0.01em;
}

.sub {
  margin: 4px 0 0;
  font-size: 12.5px;
  color: var(--text-faint);
}

.actions {
  display: flex;
  gap: 7px;
}

.actions .btn {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 11px;
  border: 1px solid var(--line);
  border-radius: 7px;
  color: var(--text-dim);
  font-size: 12.5px;
}

.actions .btn:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.spin {
  animation: turn 0.9s linear infinite;
}

@keyframes turn {
  to {
    transform: rotate(360deg);
  }
}

.oops {
  margin: 0 0 16px;
  padding: 9px 11px;
  border: 1px solid var(--danger-line);
  border-radius: 7px;
  background: var(--danger-bg);
  color: var(--red-soft);
  font-size: 12px;
}

/* The numbers sit on the page rather than in four boxes: a border round every
   figure is what makes a page like this look busier than it is. */
.strip {
  display: flex;
  gap: 30px;
  padding: 0 2px 20px;
  margin-bottom: 20px;
  border-bottom: 1px solid var(--line-soft);
}

.stat {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.stat-value {
  font-size: 18px;
  font-weight: 600;
  letter-spacing: -0.01em;
}

.stat-label {
  font-size: 11.5px;
  color: var(--text-dim);
}

.stat-hint {
  font-size: 11px;
  color: var(--text-faint);
}

.card {
  padding: 14px 16px 16px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--bg-panel);
}

.head {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 13px;
}

.head-title {
  flex: 1;
  font-size: 11px;
  letter-spacing: 0.07em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.head-note {
  font-size: 11px;
  color: var(--text-faint);
}

.year {
  margin-bottom: 12px;
}

/* Edge to edge: the whole point of the year is the shape of it, and a chart
   with a margin down one side reads as one that failed to load. */
.grid {
  display: flex;
  gap: 3px;
}

.week {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.cell {
  width: 100%;
  aspect-ratio: 1;
  border-radius: 2px;
  background: var(--bg-raised);
}

.cell.l1 {
  background: color-mix(in srgb, var(--primary) 22%, var(--bg-raised));
}
.cell.l2 {
  background: color-mix(in srgb, var(--primary) 45%, var(--bg-raised));
}
.cell.l3 {
  background: color-mix(in srgb, var(--primary) 70%, var(--bg-raised));
}
.cell.l4 {
  background: var(--primary);
}

.months {
  position: relative;
  height: 14px;
  margin-top: 6px;
}

.month {
  position: absolute;
  font-size: 10.5px;
  color: var(--text-faint);
}

.columns {
  display: grid;
  grid-template-columns: minmax(0, 1.5fr) minmax(280px, 1fr);
  gap: 12px;
  align-items: start;
}

.rows {
  list-style: none;
  margin: 0;
  padding: 0;
}

/* Every row is the same set of columns, so names, branches and counts line up
   down the list instead of drifting with the text beside them. */
.row {
  display: grid;
  grid-template-columns: minmax(90px, auto) minmax(0, 1fr) 148px 58px 78px;
  align-items: center;
  gap: 10px;
  padding: 7px 6px;
  margin: 0 -6px;
  border-radius: 6px;
  font-size: 12.5px;
  cursor: pointer;
}

.row:hover {
  background: var(--bg-hover);
}

.row.gone {
  cursor: default;
  opacity: 0.55;
}

.name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.path,
.branch,
.when {
  color: var(--text-faint);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11.5px;
}

.path {
  font-family: var(--mono);
}

.branch {
  display: flex;
  align-items: center;
  gap: 5px;
  font-family: var(--mono);
  color: var(--text-dim);
}

.meta {
  display: flex;
  gap: 7px;
  justify-content: flex-end;
  font-size: 11px;
  color: var(--text-faint);
}

.meta .on {
  color: var(--warning-soft);
}

.when {
  text-align: right;
}

.find {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 8px;
  border: 1px solid var(--line);
  border-radius: 6px;
  color: var(--text-faint);
}

.find input {
  border: none;
  background: transparent;
  font-size: 12px;
  width: 120px;
  color: var(--text);
}

.empty {
  margin: 0;
  padding: 6px 2px 2px;
  font-size: 12.5px;
  color: var(--text-faint);
}

.line {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 0;
  font-size: 12.5px;
  color: var(--text-dim);
  border-top: 1px solid var(--line-soft);
}

.line:first-child {
  border-top: none;
}

.line.quiet {
  color: var(--text-faint);
}

.line-main {
  flex: 1;
  min-width: 0;
}

.line-where {
  font-size: 11px;
  color: var(--text-faint);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 45%;
}

.link {
  font-size: 11.5px;
  color: var(--primary-soft);
  padding: 2px 7px;
  border-radius: 5px;
}

.link:hover {
  background: var(--bg-active);
}

.tip {
  align-items: flex-start;
  line-height: 1.5;
}

.cap {
  flex: none;
  min-width: 34px;
  text-align: center;
  padding: 1px 5px;
  border: 1px solid var(--line);
  border-bottom-width: 2px;
  border-radius: 5px;
  background: var(--bg-raised);
  color: var(--text);
  font-size: 11px;
  font-family: inherit;
}

.side {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.footnote {
  margin-top: 20px;
  padding-top: 15px;
  border-top: 1px solid var(--line-soft);
  font-size: 11.5px;
  line-height: 1.6;
  color: var(--text-faint);
}
</style>
