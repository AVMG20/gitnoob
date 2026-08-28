<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Archive, ArrowDownToLine, GitBranchPlus, Trash2, X } from 'lucide-vue-next'
import { fullTime, relativeTime, useGit, type CommitDetail } from '~/composables/useGit'

/**
 * A stash, read the way a commit is read.
 *
 * A stash is a commit, so it has files and diffs like any other, and until now
 * the only way to see them was a line in the sidebar and a panel off to one
 * side. This puts it where the work is — what it holds, and the four things
 * you can do with it, in one place.
 */
const git = useGit()
const store = git.store

/** The entry this pane is about, found by id so a shifting list cannot
    quietly swap it for another. */
const stash = computed(() => store.stashes.find((one) => one.oid === store.stashView) ?? null)

const detail = ref<CommitDetail | null>(null)
const loading = ref(false)

/** Its own read rather than the shared `store.detail`, which belongs to
    whatever was last selected in the graph. */
watch(
  () => store.stashView,
  async (oid) => {
    detail.value = null
    if (!oid) return
    loading.value = true
    const found = await git.commitDetail(oid)
    if (store.stashView === oid) detail.value = found
    loading.value = false
  },
  { immediate: true }
)

// A stash that is dropped, popped or applied away takes its pane with it.
watch(stash, (found) => {
  if (!found) store.stashView = null
})

const files = computed(() => detail.value?.files ?? [])
const totals = computed(() => ({
  additions: files.value.reduce((sum, one) => sum + one.additions, 0),
  deletions: files.value.reduce((sum, one) => sum + one.deletions, 0)
}))

/** Opens one of its files in the viewer, at the stash's own commit. */
function openFile(path: string) {
  if (!store.stashView) return
  store.viewer = { path, commit: store.stashView }
}

const branchName = ref('')
const branching = ref(false)

async function apply(drop: boolean) {
  const one = stash.value
  if (!one) return
  const said = drop ? await git.stashPop(one.index) : await git.stashApply(one.index)
  if (said !== null) git.note(said)
}

async function drop() {
  const one = stash.value
  if (!one) return
  const said = await git.stashDrop(one.index)
  if (said !== null) {
    git.note(said)
    store.stashView = null
  }
}

async function toBranch() {
  const one = stash.value
  const name = branchName.value.trim()
  if (!one || !name) return
  const said = await git.stashBranch(one.index, name)
  if (said !== null) {
    git.note(said)
    branching.value = false
    branchName.value = ''
  }
}

function close() {
  store.stashView = null
}
</script>

<template>
  <section v-if="stash" class="stash-pane">
    <header class="head">
      <Archive :size="15" class="mark" />
      <div class="titles">
        <h2 class="truncate">{{ stash.message }}</h2>
        <p class="sub faint">
          <span v-if="stash.branch">on {{ stash.branch }} · </span>
          <span :title="fullTime(stash.time)">{{ relativeTime(stash.time) }}</span>
          · {{ files.length }} {{ files.length === 1 ? 'file' : 'files' }}
          <span v-if="totals.additions" class="add">+{{ totals.additions }}</span>
          <span v-if="totals.deletions" class="del">−{{ totals.deletions }}</span>
        </p>
      </div>
      <span class="grow" />
      <button class="btn icon" title="Close" @click="close">
        <X :size="16" />
      </button>
    </header>

    <div class="files">
      <p v-if="loading" class="none faint">Reading what it holds…</p>
      <p v-else-if="!files.length" class="none faint">This stash holds no changes.</p>
      <button
        v-for="file in files"
        :key="file.path"
        class="file"
        :class="{ on: store.viewer?.path === file.path }"
        @click="openFile(file.path)"
      >
        <span class="status" :class="file.status">{{ file.status.slice(0, 1).toUpperCase() }}</span>
        <span class="path truncate">{{ file.path }}</span>
        <span v-if="file.additions" class="add">+{{ file.additions }}</span>
        <span v-if="file.deletions" class="del">−{{ file.deletions }}</span>
      </button>
    </div>

    <!-- Turning it into a branch is the way out of a stash that will not go on
         where you are, so the field lives here rather than behind a dialog. -->
    <div v-if="branching" class="branching">
      <input
        v-model="branchName"
        type="text"
        placeholder="Name for the branch"
        spellcheck="false"
        autofocus
        @keyup.enter="toBranch"
        @keyup.esc="branching = false"
      />
      <button class="btn btn-primary" :disabled="store.busy || !branchName.trim()" @click="toBranch">
        Create it
      </button>
      <button class="btn btn-ghost" @click="branching = false">Cancel</button>
    </div>

    <footer v-else class="foot">
      <button
        class="btn btn-ghost"
        :disabled="store.busy"
        title="Put these changes back and keep the stash"
        @click="apply(false)"
      >
        <ArrowDownToLine :size="14" /> Apply
      </button>
      <button
        class="btn btn-primary"
        :disabled="store.busy"
        title="Put these changes back and take the stash off the list"
        @click="apply(true)"
      >
        <ArrowDownToLine :size="14" /> Pop
      </button>
      <button
        class="btn btn-ghost"
        :disabled="store.busy"
        title="Start a branch from it, for a stash that will not go on here"
        @click="branching = true"
      >
        <GitBranchPlus :size="14" /> Branch
      </button>
      <span class="grow" />
      <button class="btn danger" :disabled="store.busy" title="Throw the stash away" @click="drop">
        <Trash2 :size="14" /> Drop
      </button>
    </footer>
  </section>
</template>

<style scoped>
.stash-pane {
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--bg);
}

.head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 14px;
  background: var(--bg-panel);
  border-bottom: 1px solid var(--line);
}

.mark {
  flex: none;
  color: var(--amber);
}

.titles {
  min-width: 0;
}

.titles h2 {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
}

.sub {
  margin: 0;
  font-size: 11px;
}

.grow {
  margin-left: auto;
}

.icon {
  padding: 4px 6px;
}

.files {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 6px 8px;
}

.file {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 4px 8px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  text-align: left;
}

.file:hover {
  background: var(--bg-hover);
}

.file.on {
  background: var(--bg-active);
}

.status {
  flex: none;
  width: 15px;
  font-family: var(--mono);
  font-size: 10px;
  color: var(--text-faint);
  text-align: center;
}

.status.added {
  color: var(--green);
}

.status.deleted {
  color: var(--red);
}

.path {
  flex: 1;
  min-width: 0;
}

.add {
  flex: none;
  color: var(--green);
  font-size: 11px;
}

.del {
  flex: none;
  color: var(--red);
  font-size: 11px;
}

.foot,
.branching {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  background: var(--bg-panel);
  border-top: 1px solid var(--line);
}

.branching input {
  flex: 1;
  min-width: 0;
}

.danger {
  color: var(--red);
}

.danger:hover:not(:disabled) {
  background: var(--danger-bg);
  color: var(--red-soft);
}

.none {
  padding: 12px;
  font-size: 12px;
}
</style>
