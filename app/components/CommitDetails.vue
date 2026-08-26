<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import {
  Check,
  Copy,
  FileText,
  GitCommitHorizontal,
  Hash,
  Pencil,
  Sparkles,
  TriangleAlert,
  X
} from 'lucide-vue-next'
import { copyText, fullTime, useGit } from '~/composables/useGit'
import type { RewordCheck } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useFileView } from '~/composables/useFileView'
import { useAi } from '~/composables/useAi'

const git = useGit()
const store = git.store
const menu = useContextMenu()
const view = useFileView()
const ai = useAi()

const openFile = computed(() =>
  store.viewer?.commit === store.detail?.oid ? store.viewer?.path : null
)

const detail = computed(() => store.detail)
const stats = computed(() => {
  const files = detail.value?.files ?? []
  return {
    files: files.length,
    additions: files.reduce((sum, f) => sum + f.additions, 0),
    deletions: files.reduce((sum, f) => sum + f.deletions, 0)
  }
})

/** Opens the file across the graph area rather than inline. */
function show(path: string) {
  if (!detail.value) return
  store.viewer =
    store.viewer?.path === path && store.viewer?.commit === detail.value.oid
      ? null
      : { path, commit: detail.value.oid }
}

// Moving to another commit closes whatever file was open from the last one,
// and abandons a message being edited: both belong to the commit they were
// opened from.
watch(
  () => store.detail?.oid,
  () => {
    if (store.viewer?.commit) store.viewer = null
    if (editing.value) cancel()
  }
)

// --- editing the message
//
// The mistake this is for is typing a message in haste, committing, and
// wanting it back a second later, so it goes no further than git's own amend:
// the newest commit, and only while it is the newest. Anything older would
// have to be replayed, which rewrites every commit above it — a different
// operation with different consequences, and not this one.

const editing = ref(false)
const draft = ref('')
const check = ref<RewordCheck | null>(null)
const saving = ref(false)
const editor = ref<HTMLTextAreaElement | null>(null)

/** The tip of the checked-out branch: the one commit a message can be changed on. */
const headOid = computed(() => store.refs?.locals.find((branch) => branch.is_head)?.oid ?? null)
/** Detached HEAD has no branch to move, so there is nothing to offer. */
const canEdit = computed(() => !!headOid.value && detail.value?.oid === headOid.value)

/** The first line is the subject, the way git reads it. */
const subject = computed(() => draft.value.split('\n')[0]?.trim() ?? '')
/** Git's own soft limit; past it `git log --oneline` starts truncating. */
const SUBJECT_LIMIT = 72

/**
 * Asks first, then opens.
 *
 * The button is only there for the newest commit, but the answer also carries
 * whether a remote already has it — which is the one thing worth saying before
 * the message is changed rather than after.
 */
async function edit() {
  const oid = detail.value?.oid
  if (!oid) return
  const answer = await git.rewordCheck(oid)
  if (!answer) return
  if (!answer.can) {
    git.note(answer.reason ?? 'That commit cannot be reworded', 'error')
    return
  }
  check.value = answer
  draft.value = answer.body ? `${answer.summary}\n\n${answer.body}` : answer.summary
  editing.value = true
  await nextTick()
  editor.value?.focus()
}

function cancel() {
  editing.value = false
  check.value = null
  draft.value = ''
}

/**
 * Normalises the typed text into a git commit message: git takes the first
 * line as the subject whatever follows it, but everything that shows a subject
 * and a body expects a blank line between them.
 */
function composed() {
  const lines = draft.value.split('\n')
  const head = (lines.shift() ?? '').trim()
  const rest = lines.join('\n').replace(/^\s*\n/, '').trimEnd()
  return rest ? `${head}\n\n${rest}` : head
}

/** Writes a message from the commit's own diff, for the user to read and edit. */
async function generate() {
  const oid = detail.value?.oid
  if (!oid || ai.store.busy) return
  try {
    const written = await ai.commitMessageFor(oid)
    if (!written) return
    draft.value = written.body ? `${written.summary}\n\n${written.body}` : written.summary
    git.note('Commit message written by the model — read it before saving')
  } catch (error) {
    git.note(`Commit message: ${String(error)}`, 'error')
  }
}

async function save() {
  const oid = detail.value?.oid
  if (!oid || !subject.value || saving.value) return
  saving.value = true
  // Amending writes a new object, so the commit comes back with a different
  // id. Following it keeps the panel on what the user was reading rather than
  // dropping them back at the working tree.
  const now = await git.reword(oid, composed())
  saving.value = false
  if (!now) return
  cancel()
  await git.select(now)
}

function fileMenu(event: MouseEvent, path: string) {
  menu.show(
    event,
    [
      { label: 'Copy path', icon: Copy, action: () => copyText(path, 'Path') },
      { label: git.revealLabel, icon: FileText, action: () => git.reveal(path) }
    ],
    path
  )
}
</script>

<template>
  <div class="details">
    <p v-if="!detail" class="empty dim">Select a commit to see what it changed.</p>

    <template v-else>
      <div class="head">
        <div class="head-top">
          <button class="oid mono faint" title="Copy hash" @click="copyText(detail.oid, 'Hash')">
            <Hash :size="12" /> {{ detail.short }}
            <Copy :size="11" class="copy" />
          </button>
          <div class="controls" :class="{ live: editing }">
            <template v-if="editing">
              <button
                v-if="ai.configured.value"
                class="icon-btn ai"
                :disabled="saving || !!ai.store.busy"
                title="Write the message from this commit's diff"
                @click="generate"
              >
                <Spinner v-if="ai.store.busy" :size="13" />
                <Sparkles v-else :size="13" />
              </button>
              <button class="icon-btn" :disabled="saving" title="Leave it as it was" @click="cancel">
                <X :size="13" />
              </button>
              <button
                class="icon-btn save"
                :disabled="saving || !subject.length"
                title="Give the commit this message"
                @click="save"
              >
                <Spinner v-if="saving" :size="13" />
                <Check v-else :size="13" />
              </button>
            </template>
            <button
              v-else-if="canEdit"
              class="icon-btn"
              title="Change this commit's message"
              @click="edit"
            >
              <Pencil :size="13" />
            </button>
          </div>
        </div>

        <template v-if="editing">
          <div class="field">
            <textarea
              ref="editor"
              v-model="draft"
              rows="5"
              placeholder="Summary on the first line, why it changed below"
              @keydown.meta.enter="save"
              @keydown.ctrl.enter="save"
              @keydown.esc="cancel"
            />
            <span
              v-if="subject.length"
              class="counter"
              :class="{ over: subject.length > SUBJECT_LIMIT }"
              :title="`Subject length — git truncates past ${SUBJECT_LIMIT}`"
            >
              {{ subject.length }}
            </span>
          </div>
          <p v-if="check?.is_pushed" class="warn-line">
            <TriangleAlert :size="12" /> It is already on a remote — publishing this needs a force
            push.
          </p>
        </template>

        <template v-else>
          <h3>{{ detail.summary }}</h3>
          <pre v-if="detail.body" class="body">{{ detail.body }}</pre>
        </template>

        <div class="who">
          <Avatar :name="detail.author" :email="detail.email" :size="28" />
          <div>
            <div>
              <span class="dim">{{ detail.author }}</span>
              <span class="faint"> &lt;{{ detail.email }}&gt;</span>
            </div>
            <div class="faint">{{ fullTime(detail.time) }}</div>
            <div v-if="detail.committer !== detail.author" class="faint">
              committed by {{ detail.committer }}
            </div>
          </div>
        </div>

        <div class="parents">
          <GitCommitHorizontal :size="12" class="faint" />
          <button
            v-for="parent in detail.parents"
            :key="parent"
            class="parent mono"
            @click="git.select(parent)"
          >
            {{ parent.slice(0, 7) }}
          </button>
          <span v-if="!detail.parents.length" class="faint">root commit</span>
        </div>
      </div>

      <div class="files-head">
        <span>{{ stats.files }} {{ stats.files === 1 ? 'file' : 'files' }}</span>
        <span class="plus">+{{ stats.additions }}</span>
        <span class="minus">−{{ stats.deletions }}</span>
        <span class="toggle">
          <button
            class="seg"
            :class="{ on: view.state.mode === 'path' }"
            @click="view.state.mode = 'path'"
          >
            Path
          </button>
          <button
            class="seg"
            :class="{ on: view.state.mode === 'tree' }"
            @click="view.state.mode = 'tree'"
          >
            Tree
          </button>
        </span>
      </div>

      <FileList
        :files="
          detail.files.map((file) => ({
            path: file.path,
            kind: file.status,
            additions: file.additions,
            deletions: file.deletions
          }))
        "
        :selected="openFile"
        empty="No files in this commit."
        @select="show"
        @menu="(event, entry) => fileMenu(event, entry.path)"
      />
    </template>
  </div>
</template>

<style scoped>
.details {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
}

.empty {
  padding: 18px 14px;
  font-size: 12px;
}

.head {
  padding: 12px 14px;
  border-bottom: 1px solid var(--line);
}

.head-top {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-bottom: 6px;
}

/* Faded until the pointer is in the panel: changing a message is an occasional
   thing and the button should not compete with the message the rest of the
   time. Icons rather than words for the same reason — the panel is narrow, and
   what each one does is a hover away. */
.controls {
  margin-left: auto;
  display: flex;
  gap: 2px;
  opacity: 0;
  transition: opacity 0.1s;
}

.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 22px;
  border-radius: 5px;
  color: var(--text-faint);
}

.icon-btn:hover:not(:disabled) {
  background: var(--bg-hover);
  color: var(--text);
}

.icon-btn:disabled {
  opacity: 0.4;
  cursor: default;
}

.icon-btn.ai:not(:disabled) {
  color: var(--purple);
}

.icon-btn.save:not(:disabled) {
  color: var(--accent);
}

.details:hover .controls,
.controls:focus-within,
.controls.live {
  opacity: 1;
}

.field {
  position: relative;
  margin-bottom: 7px;
}

.field textarea {
  width: 100%;
  display: block;
  /* Room for the counter in the corner. */
  padding-right: 38px;
}

.counter {
  position: absolute;
  top: 6px;
  right: 8px;
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
  color: var(--text-faint);
  pointer-events: none;
}

.counter.over {
  color: var(--amber);
}

.warn-line {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 0 7px;
  font-size: 11.5px;
  color: var(--amber);
}

.oid {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 6px;
  border-radius: 4px;
}

.oid:hover {
  background: var(--bg-hover);
  color: var(--text);
}

.copy {
  opacity: 0;
}

.oid:hover .copy {
  opacity: 0.7;
}

h3 {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  line-height: 1.35;
}

.body {
  margin: 8px 0 0;
  font-family: var(--font);
  font-size: 12px;
  color: var(--text-dim);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 160px;
  overflow: auto;
}

.who {
  display: flex;
  align-items: flex-start;
  gap: 9px;
  margin-top: 11px;
  font-size: 12px;
  line-height: 1.5;
}

.parents {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 9px;
}

.parent {
  font-size: 11px;
  color: var(--text-faint);
  padding: 1px 5px;
  border-radius: 4px;
}

.parent:hover {
  background: var(--bg-hover);
  color: var(--accent);
}

.files-head {
  display: flex;
  gap: 10px;
  padding: 6px 14px;
  font-size: 11px;
  color: var(--text-faint);
  border-bottom: 1px solid var(--line-soft);
}

.toggle {
  margin-left: auto;
  display: flex;
  border: 1px solid var(--line);
  border-radius: 5px;
  overflow: hidden;
}

.seg {
  padding: 1px 7px;
  font-size: 10.5px;
  color: var(--text-faint);
}

.seg:hover {
  color: var(--text);
}

.seg.on {
  background: var(--bg-active);
  color: var(--text);
}

.plus {
  color: var(--green);
  font-size: 11px;
}

.minus {
  color: var(--red);
  font-size: 11px;
}

</style>
