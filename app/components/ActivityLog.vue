<script setup lang="ts">
import { computed, ref } from 'vue'
import { useGit } from '~/composables/useGit'

const git = useGit()
const store = git.store

const open = ref(false)
const latest = computed(() => store.log[0] ?? null)
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

    <div v-if="open" class="body">
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
  border-top: 1px solid var(--line);
  background: var(--bg-panel);
}

.strip {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 4px 12px;
  font-size: 12px;
  text-align: left;
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
