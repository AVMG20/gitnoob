<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { SquareTerminal } from 'lucide-vue-next'
import { useGit } from '~/composables/useGit'

const git = useGit()
const store = git.store

const open = ref(false)
const latest = computed(() => store.log[0] ?? null)

// --- the prompt

const prompt = ref<HTMLInputElement | null>(null)
const line = ref('')
/** What was typed before, newest last, walked with the arrow keys. */
const history = ref<string[]>([])
/** Where in the history the arrows are; `history.length` is the empty line. */
const cursor = ref(0)

/** The terminal button: opens the panel with the caret already in the prompt. */
async function focusPrompt() {
  open.value = true
  await nextTick()
  prompt.value?.focus()
}

async function submit() {
  const text = line.value.trim()
  if (!text || store.busy) return
  if (history.value[history.value.length - 1] !== text) history.value.push(text)
  cursor.value = history.value.length
  line.value = ''
  await git.typed(text)
}

function recall(step: -1 | 1) {
  const at = Math.min(Math.max(cursor.value + step, 0), history.value.length)
  cursor.value = at
  line.value = history.value[at] ?? ''
}
</script>

<template>
  <footer class="log" :class="{ open }">
    <button class="strip" @click="open = !open">
      <span class="chev" :class="{ up: open }">▴</span>
      <span v-if="store.busy" class="busy">working…</span>
      <span v-else-if="latest" class="line truncate" :class="latest.level">
        <span v-if="latest.level === 'command' || latest.level === 'failed'" class="prompt">$</span>{{ latest.text }}
      </span>
      <span v-else class="faint">Ready</span>
      <span class="faint count">{{ store.log.length }}</span>
    </button>
    <!-- Beside the strip rather than in it: a button inside a button is not a
         thing, and this one has its own job. -->
    <button class="term" title="Type a git command" @click="focusPrompt">
      <SquareTerminal :size="14" />
    </button>

    <div v-if="open" class="body">
      <!-- A prompt above the log, since the log reads newest-first: what you
           type lands directly under where you typed it. Anything git can do
           can be typed here, in the repository the window is showing, and the
           window catches up afterwards the same as it does for a button. -->
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
          @keydown.up.prevent="recall(-1)"
          @keydown.down.prevent="recall(1)"
        />
      </form>
      <div v-for="entry in store.log" :key="entry.id" class="entry" :class="entry.level">
        <span class="faint time">{{ new Date(entry.at).toLocaleTimeString() }}</span>
        <!-- A command is shown as it would be typed, so the log reads as the
             terminal session the clicks stood in for. -->
        <span v-if="entry.level === 'command' || entry.level === 'failed'" class="prompt">$</span>
        <pre class="text">{{ entry.text }}</pre>
      </div>
      <p v-if="!store.log.length" class="faint pad">Nothing yet.</p>
    </div>
  </footer>
</template>

<style scoped>
.log {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  border-top: 1px solid var(--line);
  background: var(--bg-panel);
}

.strip {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  padding: 4px 12px;
  font-size: 12px;
  text-align: left;
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

.body {
  grid-column: 1 / -1;
}

.prompt-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 12px;
  border-bottom: 1px solid var(--line-soft);
  background: var(--bg-raised);
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

.chev {
  font-size: 9px;
  color: var(--text-faint);
  transition: transform 0.12s;
}

.chev.up {
  transform: rotate(180deg);
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

.body {
  max-height: 200px;
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

.line.output {
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
</style>
