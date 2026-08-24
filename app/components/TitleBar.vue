<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  Archive,
  GitBranchPlus,
  History,
  Pencil,
  RefreshCw,
  Redo2,
  Settings,
  TriangleAlert,
  Undo2
} from 'lucide-vue-next'
import { useGit } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'

const git = useGit()
const store = git.store
const config = useConfig()

const showPush = ref(false)
const showAmend = ref(false)
const showBranch = ref(false)
const showHistory = ref(false)

const head = computed(() => store.refs?.locals.find((b) => b.is_head) ?? null)
const conflicts = computed(() => store.status?.conflicted.length ?? 0)
const nextUndo = computed(() => store.history.undo[0] ?? null)
const nextRedo = computed(() => store.history.redo[0] ?? null)
</script>

<template>
  <header class="bar">
    <div class="repo">
      <strong class="name">{{ store.repo?.name }}</strong>
      <span class="faint">/</span>
      <span class="branch mono">
        {{ store.repo?.head }}
        <span v-if="store.repo?.detached" class="pill">detached</span>
      </span>
      <span v-if="head?.ahead" class="pill up" :title="`${head.ahead} to push`">
        ↑{{ head.ahead }}
      </span>
      <span v-if="head?.behind" class="pill down" :title="`${head.behind} to pull`">
        ↓{{ head.behind }}
      </span>
    </div>

    <div class="actions">
      <button class="btn" :disabled="store.busy" title="Fetch all remotes" @click="git.fetch()">
        <RefreshCw :size="14" /> Fetch
      </button>
      <button class="btn" :disabled="store.busy" title="Pull, stashing local work first" @click="git.pull()">
        <ArrowDownToLine :size="14" /> Pull
      </button>
      <button class="btn" :disabled="store.busy" @click="showPush = true">
        <ArrowUpFromLine :size="14" /> Push
      </button>

      <span class="sep" />

      <button class="btn" :disabled="store.busy" @click="showBranch = true">
        <GitBranchPlus :size="14" /> Branch
      </button>
      <button class="btn" :disabled="store.busy" @click="showAmend = true">
        <Pencil :size="14" /> Amend
      </button>
      <button class="btn" :disabled="store.busy" title="Stash everything" @click="git.stashPush()">
        <Archive :size="14" /> Stash
      </button>
    </div>

    <div class="tools">
      <button
        class="btn icon-only"
        :disabled="store.busy || !nextUndo"
        :title="nextUndo ? `Undo ${nextUndo.label}` : 'Nothing to undo'"
        @click="git.undo()"
      >
        <Undo2 :size="15" />
      </button>
      <button
        class="btn icon-only"
        :disabled="store.busy || !nextRedo"
        :title="nextRedo ? `Redo ${nextRedo.label}` : 'Nothing to redo'"
        @click="git.redo()"
      >
        <Redo2 :size="15" />
      </button>
      <button class="btn icon-only" title="History" @click="showHistory = !showHistory">
        <History :size="15" />
      </button>

      <span class="sep" />

      <button class="btn icon-only" title="Settings" @click="config.openSettings('profiles')">
        <Settings :size="15" />
      </button>
      <ProfileMenu />
    </div>

    <div v-if="conflicts" class="banner">
      <TriangleAlert :size="14" />
      <span>
        {{ conflicts }} conflicted {{ conflicts === 1 ? 'file' : 'files' }} to resolve
      </span>
      <button class="btn tiny" @click="store.resolving = store.status?.conflicted[0] ?? ''">
        Resolve
      </button>
      <button class="btn tiny ghost" :disabled="store.busy" @click="git.abortMerge()">
        Abort merge
      </button>
    </div>

    <HistoryMenu v-if="showHistory" @close="showHistory = false" />
    <PushDialog v-if="showPush" @close="showPush = false" />
    <AmendDialog v-if="showAmend" @close="showAmend = false" />
    <BranchDialog v-if="showBranch" @close="showBranch = false" />
  </header>
</template>

<style scoped>
.bar {
  position: relative;
  /* Three tracks so the action group sits in the true centre of the window,
     whatever the repository name happens to be. */
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: 16px;
  padding: 7px 10px;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
}

.repo {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}

.name {
  white-space: nowrap;
}

.branch {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--accent);
}

.actions {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 1px;
}

.tools {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 1px;
}

.icon-only {
  padding: 5px 7px;
}

.sep {
  width: 1px;
  height: 18px;
  background: var(--line);
  margin: 0 5px;
}

.up {
  background: rgba(87, 193, 132, 0.16);
  color: var(--green);
}

.down {
  background: rgba(79, 156, 249, 0.16);
  color: var(--accent);
}

.banner {
  position: absolute;
  left: 0;
  right: 0;
  top: 100%;
  z-index: 6;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 5px 12px;
  font-size: 12px;
  color: #f2bd6e;
  background: #2a2114;
  border-bottom: 1px solid rgba(240, 168, 60, 0.35);
}

.tiny {
  font-size: 11px;
  padding: 2px 8px;
  background: var(--amber);
  color: #1a1206;
  font-weight: 600;
}

.tiny.ghost {
  background: none;
  color: #f2bd6e;
  border: 1px solid rgba(240, 168, 60, 0.4);
}
</style>
