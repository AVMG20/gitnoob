<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { ChevronUp, SquareTerminal } from 'lucide-vue-next'
import { useGit } from '~/composables/useGit'
import {
  commonPrefix,
  completionsFor,
  replaceWord,
  type CompletionSource
} from '~/composables/useCompletion'
import { usePanes } from '~/composables/usePanes'

/**
 * The console: what git has run, and a prompt to run more.
 *
 * Read the way a terminal is read — oldest at the top, newest at the bottom,
 * and the prompt under all of it. The store keeps the log newest-first because
 * the collapsed strip wants the latest line without walking the list; the
 * reading order is put back here, where the reading happens.
 */
const git = useGit()
const store = git.store

const open = ref(false)
const { layout } = usePanes()

const latest = computed(() => store.log[0] ?? null)
/** The log the way a terminal shows it. */
const lines = computed(() => [...store.log].reverse())

const scroller = ref<HTMLElement | null>(null)
const prompt = ref<HTMLInputElement | null>(null)

/** The newest line is the one worth seeing, so the view follows it down. */
async function toBottom() {
  await nextTick()
  const box = scroller.value
  if (box) box.scrollTop = box.scrollHeight
}

watch(() => store.log.length, toBottom)
watch(open, (showing) => showing && toBottom())

async function toggle() {
  open.value = !open.value
  if (open.value) await toBottom()
}

/** The terminal button: opens it with the caret already in the prompt. */
async function focusPrompt() {
  open.value = true
  await nextTick()
  prompt.value?.focus()
  await toBottom()
}

// --- the prompt

const line = ref('')
/** What was typed before, newest last, walked with the arrow keys. */
const history = ref<string[]>([])
/** Where in the history the arrows are; `history.length` is the empty line. */
const cursor = ref(0)

async function submit() {
  const text = line.value.trim()
  if (!text || store.busy) return
  if (history.value[history.value.length - 1] !== text) history.value.push(text)
  cursor.value = history.value.length
  line.value = ''
  offers.value = []
  await git.typed(text)
  await toBottom()
}

function recall(step: -1 | 1) {
  offers.value = []
  const at = Math.min(Math.max(cursor.value + step, 0), history.value.length)
  cursor.value = at
  line.value = history.value[at] ?? ''
}

// --- what Tab offers

/** The names in this repository worth completing to. */
const source = computed<CompletionSource>(() => ({
  branches: (store.refs?.locals ?? []).map((one) => one.name),
  remotes: (store.refs?.remotes ?? []).map((one) => `${one.remote}/${one.name}`),
  tags: (store.refs?.tags ?? []).map((one) => one.name),
  files: [
    ...(store.status?.staged ?? []),
    ...(store.status?.unstaged ?? []),
    ...(store.status?.conflicted ?? []).map((path) => ({ path }))
  ].map((one) => one.path)
}))

/** The matches Tab could not choose between. */
const offers = ref<string[]>([])

/**
 * Fills in as far as the matches agree, and lists them when they disagree —
 * which is what a shell does, and so the only behaviour nobody has to learn.
 */
function complete() {
  const { word, matches } = completionsFor(line.value, source.value)
  if (!matches.length) {
    offers.value = []
    return
  }
  if (matches.length === 1) {
    line.value = replaceWord(line.value, `${matches[0]} `)
    offers.value = []
    return
  }
  const shared = commonPrefix(matches)
  if (shared.length > word.length) line.value = replaceWord(line.value, shared)
  offers.value = matches
}

/** Taking one from the list finishes the word and puts the caret back. */
async function take(match: string) {
  line.value = replaceWord(line.value, `${match} `)
  offers.value = []
  await nextTick()
  prompt.value?.focus()
}
</script>

<template>
  <footer class="console" :class="{ open }">
    <!-- Only while it is open: closed, there is a one-line strip and nothing
         to resize. -->
    <ResizeHandle v-if="open" side="console" />
    <div class="strip-row">
      <button class="strip" :title="open ? 'Collapse' : 'Expand'" @click="toggle">
        <ChevronUp :size="12" class="chev" :class="{ down: open }" />
        <span v-if="store.busy" class="busy">working…</span>
        <span v-else-if="latest" class="line truncate" :class="latest.level">
          <span v-if="latest.level === 'command' || latest.level === 'failed'" class="prompt">$</span>{{ latest.text }}
        </span>
        <span v-else class="faint">Ready</span>
        <span class="faint count">{{ store.log.length }}</span>
      </button>
      <!-- Beside the strip rather than in it: a button inside a button is not
           a thing, and this one has its own job. -->
      <button class="term" title="Type a git command" @click="focusPrompt">
        <SquareTerminal :size="14" />
      </button>
    </div>

    <template v-if="open">
      <div ref="scroller" class="body" :style="{ height: `${layout.console}px` }">
        <div v-for="entry in lines" :key="entry.id" class="entry" :class="entry.level">
          <span class="faint time">{{ new Date(entry.at).toLocaleTimeString() }}</span>
          <!-- A command is shown as it would be typed, so the log reads as the
               terminal session the clicks stood in for. -->
          <span v-if="entry.level === 'command' || entry.level === 'failed'" class="prompt">$</span>
          <pre class="text">{{ entry.text }}</pre>
        </div>
        <p v-if="!lines.length" class="faint pad">Nothing yet.</p>
      </div>

      <!-- What Tab could not choose between, directly above the line being
           typed, which is where a shell puts it. -->
      <div v-if="offers.length" class="offers">
        <button v-for="one in offers" :key="one" class="offer" @click="take(one)">{{ one }}</button>
      </div>

      <form class="prompt-row" @submit.prevent="submit">
        <span class="prompt">$</span>
        <span class="git">git</span>
        <input
          ref="prompt"
          v-model="line"
          class="input"
          type="text"
          spellcheck="false"
          autocomplete="off"
          :disabled="!store.repo"
          :placeholder="store.repo ? 'log --oneline -5' : 'Open a repository first'"
          @keydown.tab.prevent="complete"
          @keydown.up.prevent="recall(-1)"
          @keydown.down.prevent="recall(1)"
          @keydown.esc="offers = []"
        />
      </form>
    </template>
  </footer>
</template>

<style scoped>
/*
 * A status bar along the bottom of the window, in the chrome tone: the last
 * thing git did, how many lines the log holds, and the way to a prompt.
 * Opened, the log is a panel on the page above the bar, with the hairline
 * handle along its top edge and the prompt at its foot; the bar stays at the
 * very bottom, where it was.
 */
.console {
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: var(--canvas);
  border-top: 1px solid var(--line);
}

/* The handle is the hairline while the log is open. */
.console.open {
  border-top: none;
}

.strip-row {
  order: 10;
  display: flex;
  align-items: center;
  flex: none;
  gap: 2px;
  height: 24px;
  padding: 0 4px 0 2px;
  background: var(--canvas);
}

.console.open .strip-row {
  border-top: 1px solid var(--line);
}

.strip {
  display: flex;
  align-items: center;
  gap: 7px;
  flex: 1;
  min-width: 0;
  height: 20px;
  padding: 0 8px;
  border-radius: var(--radius-sm);
  font-size: 11.5px;
  text-align: left;
  color: var(--text-dim);
}

.strip:hover {
  background: var(--bg-hover);
}

.chev {
  flex: none;
  color: var(--text-faint);
  transition: transform 0.15s;
}

.chev.down {
  transform: rotate(180deg);
}

.term {
  display: grid;
  place-items: center;
  flex: none;
  width: 22px;
  height: 20px;
  border-radius: var(--radius-sm);
  color: var(--text-faint);
}

.term:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.line {
  flex: 1;
  min-width: 0;
  color: var(--text-dim);
}

.line.error {
  color: var(--danger-soft);
}

.line.command,
.entry.command .text {
  color: var(--text);
}

/* A command that came back non-zero: still the command line, in the colour of
   what happened to it. What went wrong is said in a notice, not here. */
.line.failed,
.entry.failed .text {
  color: var(--danger-soft);
}

.line.output {
  color: var(--text-faint);
}

.prompt {
  flex: none;
  margin-right: 4px;
  font-family: var(--mono);
  font-size: 11px;
  color: var(--text-faint);
}

/* Working: a small pulsing dot ahead of the word, so the line reads as alive
   rather than as another message. */
.busy {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  flex: 1;
  color: var(--text);
}

.busy::before {
  content: '';
  width: 6px;
  height: 6px;
  border-radius: var(--radius-pill);
  background: var(--accent);
  animation: pulse 1s ease-in-out infinite;
}

@keyframes pulse {
  50% {
    opacity: 0.3;
  }
}

/* How many lines the log holds. */
.count {
  flex: none;
  min-width: 18px;
  padding: 0 5px;
  border-radius: var(--radius-pill);
  background: var(--bg-raised);
  font-size: 10.5px;
  font-weight: 600;
  line-height: 16px;
  text-align: center;
  font-variant-numeric: tabular-nums;
}

/*
 * The transcript, on the page. Oldest at the top, so it is read downwards and
 * the newest line is the one nearest the prompt.
 *
 * As tall as the handle above it says, and never taller than the window can
 * spare. Written as `flex: 1` with a height it took its size from its own
 * contents instead: a few hundred lines of log, and opening the console
 * covered the whole window.
 */
.body {
  flex: none;
  min-height: 0;
  max-height: 60vh;
  overflow-y: auto;
  padding: 4px 0;
  background: var(--bg);
}

.entry {
  display: flex;
  gap: 12px;
  padding: 1px 12px;
}

.entry:hover {
  background: var(--bg-hover);
}

.entry.error .text {
  color: var(--danger-soft);
}

/* What a typed command printed, as it printed it: a shade quieter than the
   command line above it, so the eye finds the command first. */
.entry.output .text {
  color: var(--text-faint);
}

.time {
  flex: none;
  font-family: var(--mono);
  font-size: 11px;
  line-height: 1.6;
  font-variant-numeric: tabular-nums;
}

.text {
  margin: 0;
  flex: 1;
  min-width: 0;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-dim);
}

.pad {
  margin: 0;
  padding: 6px 12px;
  font-size: 12px;
}

.offers {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  flex: none;
  max-height: 84px;
  overflow-y: auto;
  padding: 6px 12px;
  border-top: 1px solid var(--line-soft);
  background: var(--bg);
}

.offer {
  padding: 1px 7px;
  border-radius: var(--radius-sm);
  font-family: var(--mono);
  font-size: 11px;
  color: var(--text-dim);
  background: var(--bg-raised);
}

.offer:hover {
  background: var(--bg-hover);
  color: var(--text);
}

/* The line you type on, at the foot of the log: a bordered field, the one
   part of the window that is a terminal. */
.prompt-row {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
  padding: 6px 12px 8px;
  background: var(--bg);
}

.prompt-row > .prompt,
.git {
  line-height: 26px;
}

.git {
  font-family: var(--mono);
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
}

.input {
  flex: 1;
  min-width: 0;
  height: 26px;
  padding: 2px 8px;
  font-family: var(--mono);
  font-size: 12px;
}

.input:disabled {
  color: var(--text-faint);
  background: var(--bg-raised);
}
</style>
