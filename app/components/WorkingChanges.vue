<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  Archive,
  Ban,
  Check,
  Copy,
  EyeOff,
  FolderOpen,
  Minus,
  Plus,
  Sparkles,
  TriangleAlert,
  Undo2
} from 'lucide-vue-next'
import { copyText, useGit } from '~/composables/useGit'
import { useContextMenu } from '~/composables/useContextMenu'
import { useDragDrop } from '~/composables/useDragDrop'
import { useAi } from '~/composables/useAi'
import { useFileView } from '~/composables/useFileView'

const git = useGit()
const store = git.store
const menu = useContextMenu()
const drag = useDragDrop()
const ai = useAi()
const view = useFileView()

const selected = computed(() =>
  store.viewer && !store.viewer.commit
    ? { path: store.viewer.path, side: store.viewer.side ?? 'unstaged' }
    : null
)
const summary = ref('')
const body = ref('')
const amend = ref(false)
/** What the user had typed before ticking amend, so unticking restores it. */
const stashedMessage = ref<{ summary: string; body: string } | null>(null)
const amendPushed = ref(false)

const staged = computed(() => store.status?.staged ?? [])
const unstaged = computed(() => store.status?.unstaged ?? [])
const conflicted = computed(() => store.status?.conflicted ?? [])
const canCommit = computed(
  () =>
    (staged.value.length > 0 || amend.value) &&
    summary.value.trim().length > 0 &&
    !conflicted.value.length
)

/** Ticking amend loads HEAD's message; unticking gives back what was typed. */
async function toggleAmend(on: boolean) {
  amend.value = on
  if (on) {
    stashedMessage.value = { summary: summary.value, body: body.value }
    const draft = await git.amendDraft()
    if (draft) {
      summary.value = draft.summary
      body.value = draft.body
      amendPushed.value = draft.is_pushed
    }
  } else {
    summary.value = stashedMessage.value?.summary ?? ''
    body.value = stashedMessage.value?.body ?? ''
    amendPushed.value = false
  }
}
/**
 * The commit box is pinned to the bottom and the two file lists share what is
 * left equally. The diff only claims space once a file is actually open.
 */
const rows = computed(() =>
  [
    conflicted.value.length ? 'auto' : '0px',
    conflicted.value.length ? 'auto' : '0px',
    'minmax(80px, 1fr)',
    'minmax(80px, 1fr)',
    'auto'
  ].join(' ')
)

/** Committing straight to a shared branch is worth a word of warning. */
const onProtected = computed(() => ['main', 'master', 'develop'].includes(store.repo?.head ?? ''))

/** Opens the file across the graph area, the way GitKraken does. */
function show(path: string, side: 'staged' | 'unstaged') {
  store.viewer =
    store.viewer?.path === path && store.viewer?.side === side ? null : { path, side }
}

async function commit() {
  if (!canCommit.value) return
  const message = body.value.trim()
    ? `${summary.value.trim()}\n\n${body.value.trim()}`
    : summary.value.trim()
  if (await git.commit(message, amend.value)) {
    summary.value = ''
    body.value = ''
    stashedMessage.value = null
    amend.value = false
    amendPushed.value = false
  }
}

/**
 * Sends the change to the stash instead of committing it, using whatever is in
 * the summary box as the stash's name — otherwise stashes are impossible to
 * tell apart later.
 */
async function stashInstead() {
  const name = summary.value.trim()
  if (await git.stashPush(name || undefined)) {
    summary.value = ''
    body.value = ''
  }
}

async function generate() {
  const message = await ai.commitMessage()
  if (!message) return
  summary.value = message.summary
  body.value = message.body
  git.note('Commit message written by the model — read it before committing')
}

function fileMenu(event: MouseEvent, path: string, side: 'staged' | 'unstaged', kind: string) {
  menu.show(
    event,
    [
      side === 'staged'
        ? { label: 'Unstage', icon: Minus, action: () => git.unstage([path]) }
        : { label: 'Stage', icon: Plus, action: () => git.stage([path]) },
      {
        label: 'Discard changes to this file',
        icon: Undo2,
        danger: true,
        disabled: kind === 'untracked',
        action: () => git.discard([path])
      },
      { separator: true, label: '' },
      {
        label: 'Add to .gitignore',
        icon: EyeOff,
        disabled: kind !== 'untracked',
        action: () => git.addToGitignore(path)
      },
      { label: 'Reveal in Finder', icon: FolderOpen, action: () => git.reveal(path) },
      { label: 'Copy path', icon: Copy, action: () => copyText(path, 'Path') }
    ],
    path
  )
}

</script>

<template>
  <div class="working" :style="{ gridTemplateRows: rows }">
    <div v-if="conflicted.length" class="conflict">
      <TriangleAlert :size="14" />
      <span class="grow">
        {{ conflicted.length }} conflicted {{ conflicted.length === 1 ? 'file' : 'files' }}
      </span>
      <button class="btn tiny warn" @click="store.resolving = conflicted[0]">Resolve</button>
    </div>
    <div v-if="conflicted.length" class="conflict-files">
      <button
        v-for="path in conflicted"
        :key="path"
        class="conflict-file truncate"
        @click="store.resolving = path"
      >
        {{ path }}
      </button>
    </div>

    <!-- Unstaged -->
    <div
      class="group"
      :class="{ drop: drag.state.over === 'zone:unstaged' }"
      @dragover="drag.hover($event, 'zone:unstaged', ['file'])"
      @dragleave="drag.leave('zone:unstaged')"
      @drop.prevent="
        (() => {
          const payload = drag.take(['file'])
          if (payload?.kind === 'file' && payload.staged) git.unstage([payload.path])
        })()
      "
    >
      <div class="group-head">
        <span class="section-title">Unstaged <span class="num">{{ unstaged.length }}</span></span>
        <span class="head-tools">
          <span class="toggle">
            <button
              class="seg"
              :class="{ on: view.state.mode === 'path' }"
              title="Flat list of full paths"
              @click="view.state.mode = 'path'"
            >
              Path
            </button>
            <button
              class="seg"
              :class="{ on: view.state.mode === 'tree' }"
              title="Grouped by folder"
              @click="view.state.mode = 'tree'"
            >
              Tree
            </button>
          </span>
          <button class="btn tiny" :disabled="store.busy || !unstaged.length" @click="git.stageAll()">
            Stage all
          </button>
        </span>
      </div>
      <FileList
        :files="unstaged"
        :selected="selected?.side === 'unstaged' ? selected.path : null"
        empty="Nothing changed."
        draggable
        @select="(path) => show(path, 'unstaged')"
        @menu="(event, entry) => fileMenu(event, entry.path, 'unstaged', entry.kind)"
        @dragstart="
          (event, entry) => drag.begin(event, { kind: 'file', path: entry.path, staged: false })
        "
        @dragend="drag.end()"
      />
    </div>

    <!-- Staged -->
    <div
      class="group"
      :class="{ drop: drag.state.over === 'zone:staged' }"
      @dragover="drag.hover($event, 'zone:staged', ['file'])"
      @dragleave="drag.leave('zone:staged')"
      @drop.prevent="
        (() => {
          const payload = drag.take(['file'])
          if (payload?.kind === 'file' && !payload.staged) git.stage([payload.path])
        })()
      "
    >
      <div class="group-head">
        <span class="section-title">Staged <span class="num">{{ staged.length }}</span></span>
        <button
          class="btn tiny"
          :disabled="store.busy || !staged.length"
          @click="git.unstage(staged.map((e) => e.path))"
        >
          Unstage all
        </button>
      </div>
      <FileList
        :files="staged"
        :selected="selected?.side === 'staged' ? selected.path : null"
        empty="Drag files here, or stage them below."
        draggable
        @select="(path) => show(path, 'staged')"
        @menu="(event, entry) => fileMenu(event, entry.path, 'staged', entry.kind)"
        @dragstart="
          (event, entry) => drag.begin(event, { kind: 'file', path: entry.path, staged: true })
        "
        @dragend="drag.end()"
      />
    </div>

    <!-- Commit box -->
    <div class="commit">
      <div class="commit-head">
        <label class="amend">
          <input
            type="checkbox"
            :checked="amend"
            @change="toggleAmend(($event.target as HTMLInputElement).checked)"
          />
          Amend previous commit
        </label>
        <button
          v-if="ai.configured.value"
          class="btn tiny ai"
          :disabled="!staged.length || !!ai.store.busy"
          title="Write a message from the staged diff"
          @click="generate"
        >
          <Spinner v-if="ai.store.busy" :size="12" />
          <Sparkles v-else :size="12" />
          {{ ai.store.busy ? 'Writing…' : 'Generate' }}
        </button>
      </div>
      <input v-model="summary" class="summary" type="text" placeholder="Summary" />
      <textarea v-model="body" rows="3" placeholder="Why (optional)" />
      <p v-if="amend && amendPushed" class="warn-line">
        <TriangleAlert :size="12" /> That commit is already on a remote — amending it will need a
        force push.
      </p>
      <p v-else-if="onProtected && (staged.length || amend)" class="warn-line">
        <TriangleAlert :size="12" /> You are committing straight to
        <span class="mono">{{ store.repo?.head }}</span>.
      </p>
      <div class="buttons">
        <button class="btn btn-primary wide" :disabled="store.busy || !canCommit" @click="commit">
        <Spinner v-if="store.busy" :size="13" />
        <Check v-else :size="14" />
        <template v-if="amend">
          Amend commit{{ staged.length ? ` with ${staged.length} more` : '' }}
        </template>
        <template v-else>
          Commit {{ staged.length }} {{ staged.length === 1 ? 'file' : 'files' }}
        </template>
        </button>
        <button
          class="btn btn-ghost stash-btn"
          :disabled="store.busy || (!staged.length && !unstaged.length)"
          :title="
            summary.trim()
              ? `Stash everything as \u201c${summary.trim()}\u201d instead of committing`
              : 'Stash everything instead of committing'
          "
          @click="stashInstead"
        >
          <Archive :size="14" />
          Stash
        </button>
      </div>
      <p v-if="conflicted.length" class="blocked faint">
        <Ban :size="12" /> Resolve the conflicts first.
      </p>
    </div>
  </div>
</template>

<style scoped>
.working {
  display: grid;
  min-height: 0;
  overflow: hidden;
}

.conflict {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  font-size: 12px;
  color: #f2bd6e;
  background: rgba(240, 168, 60, 0.1);
  border-bottom: 1px solid var(--line-soft);
}

.grow {
  flex: 1;
}

.conflict-files {
  border-bottom: 1px solid var(--line-soft);
  max-height: 90px;
  overflow-y: auto;
}

.conflict-file {
  display: block;
  width: 100%;
  text-align: left;
  padding: 3px 12px;
  font-size: 11.5px;
  color: #ef8d9c;
}

.conflict-file:hover {
  background: var(--bg-hover);
}

.group {
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-bottom: 1px solid var(--line-soft);
}

.group.drop {
  background: rgba(79, 156, 249, 0.1);
  outline: 1px dashed var(--accent);
  outline-offset: -3px;
}

.group-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-right: 8px;
}

.num {
  color: var(--text-dim);
}

.tiny {
  font-size: 11px;
  padding: 2px 7px;
}

.tiny.ai {
  color: var(--purple);
  border: 1px solid rgba(169, 123, 240, 0.4);
}

.tiny.warn {
  background: var(--amber);
  color: #1a1206;
  font-weight: 600;
}

.head-tools {
  display: flex;
  align-items: center;
  gap: 7px;
}

.toggle {
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

.commit {
  padding: 9px 10px 10px;
  border-top: 1px solid var(--line);
  background: var(--bg-panel);
}

.commit-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.amend {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 11.5px;
  color: var(--text-dim);
  cursor: pointer;
  padding: 2px 0 5px;
}

.summary,
textarea {
  width: 100%;
  margin-bottom: 7px;
}

.buttons {
  display: flex;
  gap: 7px;
}

.wide {
  flex: 1;
  justify-content: center;
}

.stash-btn {
  flex: none;
}

.warn-line,
.blocked {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0 0 7px;
  font-size: 11.5px;
  color: var(--amber);
}

.blocked {
  margin: 7px 0 0;
  color: var(--text-faint);
}
</style>
