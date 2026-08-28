<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { open as pickFolder } from '@tauri-apps/plugin-dialog'
import {
  ArrowDown,
  ArrowUp,
  FolderOpen,
  FolderX,
  GitBranch,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Shuffle,
  X
} from 'lucide-vue-next'
import { ago, short, useHome, type RepoCard } from '~/composables/useHome'
import { useConfig } from '~/composables/useConfig'
import { keyLabel } from '~/composables/useShortcuts'
import { pickTips, type Tip } from '~/composables/useTips'

/**
 * The home tab: every project, and a year of your own commits.
 *
 * The tab strip runs out of window long before anybody runs out of
 * repositories, so this is where all of them live — and, since it is reading
 * them anyway, where the things that are true across all of them can be said:
 * what is uncommitted, what has not been pushed, and how the year has gone.
 *
 * Read top to bottom it goes from the week to the year to the work: the four
 * figures, the year they add up to, and then the two lists — the projects, and
 * what among them is waiting. The year sits high because it is the same
 * subject as the figures above it at a longer range, and because a band that
 * wide either leads the page or interrupts it.
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

/** A piece of a tile's figure, so one number can be two colours. */
interface Part {
  text: string
  tone?: 'up' | 'down'
}

const tiles = computed(() => {
  const found = stats.value
  if (!found) return []
  const change = found.week - found.previous_week
  return [
    {
      label: 'commits this week',
      parts: [{ text: String(found.week) }] as Part[],
      hint:
        change === 0
          ? 'level with last week'
          : `${change > 0 ? '↑' : '↓'} ${Math.abs(change)} on last week`,
      // Up or down on last week, in the two colours the diffs already use for
      // more and less: a number on its own says nothing about the week.
      tone: change === 0 ? undefined : change > 0 ? 'up' : ('down' as Part['tone'])
    },
    {
      label: 'day streak',
      parts: [{ text: String(found.streak) }] as Part[],
      hint: found.best_streak ? `best so far: ${found.best_streak}` : '',
      tone: undefined
    },
    {
      label: found.repos_this_week === 1 ? 'repository touched' : 'repositories touched',
      parts: [{ text: String(found.repos_this_week) }] as Part[],
      hint: `${found.read} commits in the year`,
      tone: undefined
    },
    {
      label: 'lines this week',
      // Gained and lost are not one figure with a slash in it: they are the
      // green number and the red one, the same pair the diffs colour.
      parts: [
        { text: `+${short(found.added)}`, tone: 'up' },
        { text: `−${short(found.removed)}`, tone: 'down' }
      ] as Part[],
      hint: 'added and removed',
      tone: undefined
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

/**
 * What the pointer is over in the year, and where to draw it.
 *
 * A `title` says the same thing, but a second late and in the platform's own
 * box, which on a 371-square grid is the difference between reading the year
 * and interrogating it. The bubble is placed from the square's own rectangle,
 * so it sits over the day it is about however wide the window is.
 */
const bubble = ref<{ text: string; x: number; y: number } | null>(null)
const yearCard = ref<HTMLElement | null>(null)

/** The date a square stands for: the last one is today. */
function dayOf(index: number) {
  const days = stats.value?.days.length ?? 0
  const when = new Date()
  when.setHours(12, 0, 0, 0)
  when.setDate(when.getDate() - (days - 1 - index))
  return when
}

const WHEN = new Intl.DateTimeFormat(undefined, {
  weekday: 'short',
  day: 'numeric',
  month: 'short'
})

function overDay(event: MouseEvent, index: number) {
  const card = yearCard.value
  const square = event.currentTarget as HTMLElement | null
  if (!card || !square) return
  const count = stats.value?.days[index] ?? 0
  const here = square.getBoundingClientRect()
  const around = card.getBoundingClientRect()
  bubble.value = {
    text: `${count || 'No'} ${count === 1 ? 'commit' : 'commits'} · ${WHEN.format(dayOf(index))}`,
    x: here.left - around.left + here.width / 2,
    y: here.top - around.top
  }
}

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
interface Waiting {
  text: string
  where: string
  path: string
  action: string
  kind: 'gone' | 'dirty' | 'ahead'
}

const attention = computed(() => {
  const rows: Waiting[] = []
  for (const repo of home.repos.value) {
    if (!repo.exists) {
      rows.push({
        text: 'Folder is not there any more',
        where: repo.name,
        path: repo.path,
        action: 'Forget',
        kind: 'gone'
      })
      continue
    }
    if (repo.dirty) {
      rows.push({
        text: `${repo.dirty} uncommitted ${repo.dirty === 1 ? 'change' : 'changes'}`,
        where: repo.name,
        path: repo.path,
        action: 'Open',
        kind: 'dirty'
      })
    }
    if (repo.ahead) {
      rows.push({
        text: `${repo.ahead} ${repo.ahead === 1 ? 'commit' : 'commits'} not on origin`,
        where: `${repo.name} · ${repo.branch}`,
        path: repo.path,
        action: 'Open',
        kind: 'ahead'
      })
    }
  }
  return rows
})

/** Six is what the card holds without becoming a second project list. */
const waiting = computed(() => attention.value.slice(0, 6))

const ICONS = { gone: FolderX, dirty: Pencil, ahead: ArrowUp }

/**
 * Three tips, swapped every so often.
 *
 * There are two dozen of them and only room for three, so which three is a
 * question the page answers again every time you look at it: on opening, on
 * the shuffle, and on a slow timer while you are reading. Slow, because a line
 * that changes while it is being read is worse than one that never changes.
 */
const tips = ref<Tip[]>(pickTips(3))
/** True while the three on screen are on their way out. */
const fading = ref(false)
const holding = ref(false)
let turn: number | undefined
let swap: number | undefined

const FADE_MS = 180

/**
 * Fades the three out, swaps them, fades the three in.
 *
 * The list itself is never unmounted. A keyed transition did the same fade and
 * left a gap between its two halves where the block had nothing in it at all,
 * which is what a screenshot taken at the wrong moment caught.
 */
function shuffle() {
  if (swap) window.clearTimeout(swap)
  fading.value = true
  swap = window.setTimeout(() => {
    tips.value = pickTips(3)
    fading.value = false
  }, FADE_MS)
}

const ROTATE_MS = 20_000

onMounted(() => {
  home.load()
  turn = window.setInterval(() => {
    // Not while the pointer is on the card — that is somebody reading it — and
    // not while the window is in the background, where it is only work.
    if (holding.value || document.hidden) return
    shuffle()
  }, ROTATE_MS)
})

onUnmounted(() => {
  if (turn) window.clearInterval(turn)
  if (swap) window.clearTimeout(swap)
})

const hour = new Date().getHours()
const greeting =
  hour < 6 ? 'Still up' : hour < 12 ? 'Good morning' : hour < 18 ? 'Good afternoon' : 'Good evening'
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

/**
 * A colour per project, kept by name.
 *
 * The same ten the commit graph draws its lanes in, so a project has one
 * colour wherever you meet it, and a list of ten repositories is scannable by
 * something other than reading every name.
 */
function lane(name: string) {
  let hash = 0
  for (const letter of name) hash = (hash * 31 + letter.charCodeAt(0)) >>> 0
  return `var(--lane-${(hash % 10) + 1})`
}

/** The same folder picker the welcome pane and the tab strip use. */
async function pick() {
  const path = await pickFolder({ directory: true, multiple: false, title: 'Open a repository' })
  if (typeof path === 'string') emit('open', path)
}

/**
 * Takes a project off the list.
 *
 * Only the recents, and only the ones that are not open: a tab is closed by
 * its own cross, and losing one from under the strip because a list somewhere
 * else was being tidied is not what tidying means. Nothing on disk is touched,
 * so it asks nothing — opening the folder again puts it straight back.
 */
async function forget(repo: RepoCard) {
  if (open.value.has(repo.path)) return
  await config.forgetProject(repo.path)
  const summary = home.store.summary
  if (summary) summary.repos = summary.repos.filter((one) => one.path !== repo.path)
}

function choose(repo: RepoCard) {
  if (!repo.exists) return
  emit('open', repo.path)
}
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
            <template v-if="attention.length">
              · <span class="sub-flag">{{ attention.length }} waiting</span>
            </template>
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

      <!-- The week in four figures. One block rather than four, because they
           are four readings of the same week and belong on one instrument. -->
      <section v-if="tiles.length" class="stats">
        <div v-for="tile in tiles" :key="tile.label" class="stat">
          <span class="stat-label">{{ tile.label }}</span>
          <span class="stat-value">
            <span v-for="(part, at) in tile.parts" :key="at" class="part" :class="part.tone">
              {{ part.text }}
            </span>
          </span>
          <span v-if="tile.hint" class="stat-hint" :class="tile.tone">{{ tile.hint }}</span>
        </div>
      </section>

      <!-- The year sits under the week's figures, because it is the same
           subject at a longer range: how it has been going. What is waiting
           and where the projects are follow, close enough to the top of the
           page to be reached without a scroll on a laptop. -->
      <section v-if="weeks.length" ref="yearCard" class="year" @mouseleave="bubble = null">
        <div class="head">
          <span class="head-title">A year of commits, across everything</span>
          <span v-if="home.store.summary?.author" class="head-note mono">
            {{ home.store.summary.author }}
          </span>
          <span class="legend">
            Less
            <span v-for="step in [0, 1, 2, 3, 4]" :key="step" class="swatch" :class="`l${step}`" />
            More
          </span>
        </div>
        <div class="grid">
          <div v-for="(week, at) in weeks" :key="at" class="week">
            <span
              v-for="(day, index) in week"
              :key="index"
              class="cell"
              :class="`l${level(day)}`"
              @mouseenter="overDay($event, at * 7 + index)"
            />
          </div>
        </div>

        <span
          v-if="bubble"
          class="bubble"
          :style="{ left: `${bubble.x}px`, top: `${bubble.y}px` }"
        >
          {{ bubble.text }}
        </span>
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
        <section class="projects">
          <div class="head">
            <span class="head-title">Projects</span>
            <span class="count">{{ shown.length }}</span>
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
              <span class="mark" :style="{ '--mark': lane(repo.name) }">
                {{ repo.name.slice(0, 1).toUpperCase() }}
              </span>

              <span class="about">
                <span class="line-one">
                  <span class="name">{{ repo.name }}</span>
                  <span class="branch">
                    <GitBranch :size="10" />
                    {{ repo.exists ? repo.branch : 'not on disk' }}
                  </span>
                </span>
                <MidTruncate class="path" :text="repo.path" :tail="14" />
              </span>

              <!-- Counted the way the file tree counts a folder: the icon
                   carries the colour and the number carries the count. A row of
                   filled pills was louder than the names beside them. -->
              <span class="tallies">
                <span v-if="repo.dirty" class="tally dirty" :title="`${repo.dirty} uncommitted`">
                  <Pencil :size="10" :stroke-width="2.25" />{{ repo.dirty }}
                </span>
                <span v-if="repo.ahead" class="tally ahead" :title="`${repo.ahead} to push`">
                  <ArrowUp :size="10" :stroke-width="2.25" />{{ repo.ahead }}
                </span>
                <span v-if="repo.behind" class="tally behind" :title="`${repo.behind} to pull`">
                  <ArrowDown :size="10" :stroke-width="2.25" />{{ repo.behind }}
                </span>
              </span>

              <span class="when">
                <span v-if="open.has(repo.path)" class="live">open</span>
                <template v-else>{{ ago(repo.last_commit) }}</template>
              </span>

              <!-- Only on the ones that are not open, and only on hover: the
                   list is for reading, not for a row of crosses down its edge. -->
              <button
                v-if="!open.has(repo.path)"
                class="drop"
                :title="`Take ${repo.name} off the list`"
                @click.stop="forget(repo)"
              >
                <X :size="12" />
              </button>
              <span v-else class="drop-space" />
            </li>
          </ul>
        </section>

        <div class="side">
          <section class="look">
            <div class="head">
              <span class="head-title">Needs a look</span>
              <span v-if="attention.length" class="count">{{ attention.length }}</span>
            </div>
            <ul class="rows">
              <li v-for="one in waiting" :key="`${one.path}${one.text}`" class="line">
                <component :is="ICONS[one.kind]" :size="13" class="line-icon" :class="one.kind" />
                <span class="line-body">
                  <span class="line-main">{{ one.text }}</span>
                  <span class="line-where">{{ one.where }}</span>
                </span>
                <button class="link" @click="emit('open', one.path)">{{ one.action }}</button>
              </li>
              <li v-if="!waiting.length" class="line quiet">
                Nothing waiting — everything is committed and pushed.
              </li>
              <li v-if="attention.length > waiting.length" class="line quiet">
                and {{ attention.length - waiting.length }} more, in the list beside this.
              </li>
            </ul>
          </section>

          <section
            class="tips"
            @mouseenter="holding = true"
            @mouseleave="holding = false"
          >
            <div class="head">
              <span class="head-title">Did you know</span>
              <button class="shuffle" title="Three others" @click="shuffle">
                <Shuffle :size="12" />
              </button>
            </div>
            <ul class="rows swapping" :class="{ out: fading }">
                <!-- The sentence first and the key after it, as part of the
                     sentence: a column of keys down the left made the ones
                     without a key look like they were missing something. -->
                <li v-for="tip in tips" :key="tip.id" class="line tip">
                  <span class="line-main">
                    {{ tip.text }}
                    <kbd v-if="tip.keys" class="cap">{{ keyLabel(tip.keys) }}</kbd>
                  </span>
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
  padding: 26px 30px 44px;
}

/* The greeting gets the room the sections below it get: it was sitting a
   third of the distance from the figures that everything else keeps. */
.top {
  display: flex;
  align-items: flex-end;
  gap: 16px;
  margin-bottom: 26px;
}

.titles {
  flex: 1;
  min-width: 0;
}

h1 {
  margin: 0;
  font-size: 21px;
  font-weight: 600;
  letter-spacing: -0.015em;
}

.sub {
  margin: 4px 0 0;
  font-size: 12.5px;
  color: var(--text-faint);
}

.sub-flag {
  color: var(--accent-soft);
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

/*
 * The figures and the year below them sit straight on the page, under a rule.
 *
 * A panel round every block turned the page into a stack of boxes with boxes
 * inside them; the two that are read straight through — four numbers and a
 * band of squares — need no walls to be told apart from what follows. The
 * lists keep theirs, because a list of rows on the bare page has no edge for
 * the rows to line up against.
 */
.stats {
  display: flex;
  justify-content: space-between;
  gap: 26px;
  padding: 0 2px 20px;
  margin-bottom: 20px;
  border-bottom: 1px solid var(--line-soft);
}

.stat {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

/* Spread rather than columned: four blocks of different widths in four equal
   columns left ragged gaps and a bare quarter at the right-hand end. Spread,
   the first starts at the page's edge, the last ends at it, and what is
   between them is even. */

.stat-label {
  font-size: 10.5px;
  letter-spacing: 0.07em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.stat-value {
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 20px;
  font-weight: 600;
  letter-spacing: -0.02em;
  font-variant-numeric: tabular-nums;
  line-height: 1.15;
}

/* The same green and red the diffs use for lines gained and lost. */
.part.up,
.stat-hint.up {
  color: var(--green);
}

.part.down,
.stat-hint.down {
  color: var(--red);
}

.stat-hint {
  margin-top: auto;
  padding-top: 4px;
  font-size: 11px;
  color: var(--text-faint);
}

/*
 * Nothing on the page is in a box.
 *
 * The lists had one so their rows had an edge to line up against; a rule down
 * the middle does that job without another four borders, and the page reads as
 * one sheet rather than as a tray of panels.
 */
/*
 * The two lists are panels; everything above them is read straight off the
 * sheet.
 *
 * A list of rows wants an edge to line up against, and the block that says
 * something is wrong wants to be a thing on the page rather than another
 * paragraph of it. The tips below it need neither, so they get neither.
 */
.projects,
.look {
  padding: 12px 14px 14px;
  border: 1px solid var(--line-soft);
  border-radius: var(--radius);
  background: var(--bg-panel);
}

.side > section + section {
  margin-top: 20px;
  padding: 0 2px;
}

.head {
  display: flex;
  align-items: center;
  gap: 9px;
  margin-bottom: 12px;
}

.head-title {
  font-size: 11px;
  letter-spacing: 0.07em;
  text-transform: uppercase;
  color: var(--text-faint);
}

/* The number beside a heading, so the card says how much is in it before it
   is read. */
.count {
  padding: 1px 7px;
  border-radius: 999px;
  background: var(--bg-raised);
  color: var(--text-dim);
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}

.head-note {
  margin-left: auto;
  font-size: 11px;
  color: var(--text-faint);
}

.head .find {
  margin-left: auto;
}

.columns {
  display: grid;
  grid-template-columns: minmax(0, 1.55fr) minmax(300px, 1fr);
  gap: 26px;
  align-items: start;
  margin-bottom: 12px;
  padding: 0 2px;
}

.rows {
  list-style: none;
  margin: 0;
  padding: 0;
}

/* Two lines: what it is, and where it is. The counts and the date keep their
   own columns so they line up down the list however long a name runs. */
.row {
  display: grid;
  grid-template-columns: 22px minmax(0, 1fr) auto 84px 18px;
  align-items: center;
  gap: 11px;
  padding: 8px 8px;
  margin: 0 -8px;
  border-radius: var(--radius-sm);
  font-size: 12.5px;
  cursor: pointer;
}

.row:hover {
  background: var(--bg-hover);
}

.row.gone {
  cursor: default;
  opacity: 0.5;
}

/* The project's own colour, the ten the graph draws lanes in. */
.mark {
  display: grid;
  place-items: center;
  width: 22px;
  height: 22px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--mark) 16%, transparent);
  color: var(--mark);
  font-size: 11px;
  font-weight: 600;
}

.about {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.line-one {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.branch {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
  max-width: 45%;
  padding: 1px 6px;
  border-radius: 5px;
  background: var(--bg-raised);
  color: var(--text-dim);
  font-family: var(--mono);
  font-size: 10.5px;
  overflow: hidden;
  white-space: nowrap;
}

.path {
  color: var(--text-faint);
  font-family: var(--mono);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tallies {
  display: flex;
  gap: 9px;
  justify-content: flex-end;
}

/* The file tree's own summary, borrowed: the icon carries the colour and the
   number carries the count, so a busy row is still quieter than the names. */
.tally {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  flex: none;
  color: var(--text-dim);
  font-size: 11px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.tally.dirty svg {
  color: var(--amber);
}

.tally.ahead svg {
  color: var(--green-soft);
}

.tally.behind svg {
  color: var(--accent);
}

.when {
  text-align: right;
  font-size: 11px;
  color: var(--text-faint);
  white-space: nowrap;
}

/* A tab that is open says so in the accent, without a box round it: the strip
   above already draws the box. Named under .when, because a bare .live also
   matched the row it was on and turned every open project blue. */
.when .live {
  color: var(--accent-soft);
}

.drop {
  display: grid;
  place-items: center;
  width: 18px;
  height: 18px;
  border-radius: 4px;
  color: var(--text-faint);
  opacity: 0;
}

.row:hover .drop {
  opacity: 1;
}

.drop:hover {
  color: var(--text);
  background: var(--bg-active);
}

.drop-space {
  width: 18px;
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
  align-items: flex-start;
  gap: 9px;
  padding: 8px 0;
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

.line-icon {
  flex: none;
  margin-top: 1px;
  color: var(--text-faint);
}

/* The pen the project list uses for the same count, in the same amber: one
   thing said twice on one page should not be said in two colours. */
.line-icon.dirty {
  color: var(--amber);
}

.line-icon.ahead {
  color: var(--green);
}

.line-icon.gone {
  color: var(--red);
}

.line-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.line-main {
  min-width: 0;
}

.line-where {
  font-size: 11px;
  color: var(--text-faint);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.link {
  flex: none;
  font-size: 11.5px;
  color: var(--primary-soft);
  padding: 2px 7px;
  border-radius: 5px;
}

.link:hover {
  background: var(--bg-active);
}

.tips .head-title {
  flex: 1;
}

.shuffle {
  display: grid;
  place-items: center;
  width: 22px;
  height: 22px;
  border-radius: 5px;
  color: var(--text-faint);
}

.shuffle:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.tip {
  align-items: flex-start;
  line-height: 1.55;
}

.cap {
  display: inline-block;
  margin-left: 4px;
  padding: 0 5px;
  vertical-align: baseline;
  white-space: nowrap;
  border: 1px solid var(--line);
  border-bottom-width: 2px;
  border-radius: 5px;
  background: var(--bg-raised);
  color: var(--text);
  font-size: 11px;
  font-family: inherit;
}

/* Slow enough to read as a change of mind rather than a flicker. */
.swapping {
  transition: opacity 0.18s ease;
}

.swapping.out {
  opacity: 0;
}



.year {
  position: relative;
  padding: 0 2px 20px;
  margin-bottom: 20px;
  border-bottom: 1px solid var(--line-soft);
}

.year .head {
  margin-bottom: 14px;
}

.legend {
  display: flex;
  align-items: center;
  gap: 3px;
  font-size: 10.5px;
  color: var(--text-faint);
}

/* The key's squares are not days, so they are not `.cell`: the grid below is
   the year, and counting it is how the year is checked. */
.swatch {
  width: 9px;
  height: 9px;
  border-radius: 2px;
  background: var(--bg-raised);
}

.swatch:first-of-type {
  margin-left: 4px;
}

.swatch:last-of-type {
  margin-right: 4px;
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

.cell.l1,
.swatch.l1 {
  background: color-mix(in srgb, var(--primary) 22%, var(--bg-raised));
}
.cell.l2,
.swatch.l2 {
  background: color-mix(in srgb, var(--primary) 45%, var(--bg-raised));
}
.cell.l3,
.swatch.l3 {
  background: color-mix(in srgb, var(--primary) 70%, var(--bg-raised));
}
.cell.l4,
.swatch.l4 {
  background: var(--primary);
}

/* Over the square it is about, and out of the pointer’s way. Nothing else
   can be hovered underneath it, so it takes no clicks. */
.bubble {
  position: absolute;
  transform: translate(-50%, -100%);
  margin-top: -7px;
  padding: 3px 8px;
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text);
  font-size: 11px;
  white-space: nowrap;
  pointer-events: none;
  box-shadow: 0 4px 12px var(--shadow);
  z-index: 2;
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

.footnote {
  margin-top: 18px;
  padding-top: 15px;
  border-top: 1px solid var(--line-soft);
  font-size: 11.5px;
  line-height: 1.6;
  color: var(--text-faint);
}

/* A narrow window puts the side column under the list rather than squeezing
   both; the stats go two by two. */
@media (max-width: 900px) {
  .columns {
    grid-template-columns: minmax(0, 1fr);
  }

  .stats {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .stat:nth-child(3) {
    border-left: none;
  }
}
</style>
