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
.console {
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-top: 1px solid var(--line);
  background: var(--bg-panel);
}

.strip-row {
  display: flex;
  align-items: stretch;
  flex: none;
}

.strip {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 0;
  padding: 4px 12px;
  font-size: 12px;
  text-align: left;
}

.strip:hover {
  background: var(--bg-hover);
}

.chev {
  flex: none;
  color: var(--text-faint);
  transition: transform 0.12s;
}

.chev.down {
  transform: rotate(180deg);
}

.term {
  display: flex;
  align-items: center;
  padding: 0 10px;
  color: var(--text-faint);
  border-left: 1px solid var(--line-soft);
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
  color: var(--red);
}

.line.command,
.entry.command .text {
  color: var(--accent);
}

/* A command that came back non-zero: still the command line, in the colour of
   what happened to it. What went wrong is said in a notice, not here. */
.line.failed,
.entry.failed .text {
  color: var(--red-soft);
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

.busy {
  flex: 1;
  color: var(--accent);
}

.count {
  font-size: 11px;
}

/*
 * The transcript. Oldest at the top, so it is read downwards and the newest
 * line is the one nearest the prompt.
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
  border-top: 1px solid var(--line-soft);
}

.entry {
  display: flex;
  gap: 10px;
  padding: 3px 12px;
  border-bottom: 1px solid var(--line-soft);
}

.entry.error .text {
  color: var(--red);
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
}

.text {
  margin: 0;
  flex: 1;
  min-width: 0;
  font-family: var(--mono);
  font-size: 11px;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-dim);
}

.pad {
  padding: 8px 12px;
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
  background: var(--bg-raised);
}

.offer {
  padding: 1px 6px;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 11px;
  color: var(--text-dim);
  border: 1px solid var(--line-soft);
}

.offer:hover {
  background: var(--bg-hover);
  color: var(--text);
}

/* The line you type on, at the very bottom and darker than the transcript
   above it — the one part of the window that is a terminal. */
.prompt-row {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: none;
  padding: 6px 12px;
  border-top: 1px solid var(--line);
  background: var(--bg-deep);
}

.git {
  font-family: var(--mono);
  font-size: 11px;
  color: var(--accent);
}

.input {
  flex: 1;
  min-width: 0;
  padding: 2px 4px;
  background: none;
  border: none;
  outline: none;
  font-family: var(--mono);
  font-size: 11px;
  color: var(--text);
}

.input::placeholder {
  color: var(--text-faint);
}

.input:disabled {
  color: var(--text-faint);
}
</style>
