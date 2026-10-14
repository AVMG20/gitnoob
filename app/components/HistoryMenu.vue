<script setup lang="ts">
import { computed, ref } from 'vue'
import { ArrowLeft, GitBranch, History, Redo2, RotateCcw, TriangleAlert, Undo2 } from 'lucide-vue-next'
import PromptDialog from '~/components/PromptDialog.vue'
import ResetDialog from '~/components/ResetDialog.vue'
import { relativeTime, useGit, type LostStash, type Moment } from '~/composables/useGit'

const emit = defineEmits<{ close: [] }>()
const git = useGit()
const store = git.store

const undo = computed(() => store.history.undo)
const redo = computed(() => store.history.redo)

/**
 * Two pages in one menu: the steps this app recorded, and "More history…" —
 * the reflog, where HEAD and the branch have been whoever moved them, plus the
 * stashes that fell off the list. The second page only reads until a row's
 * action is picked, and each action is one that cannot lose anything: a new
 * branch, a stash put back on the list, or a reset that keeps open work and
 * asks first.
 */
const view = ref<'steps' | 'more'>('steps')
const moments = ref<Moment[] | null>(null)
const lost = ref<LostStash[] | null>(null)
const looking = ref(false)

/** A dialog opened from a row. The menu steps aside while it is up. */
const naming = ref<Moment | null>(null)
const resetting = ref<Moment | null>(null)
const dialogUp = computed(() => naming.value !== null || resetting.value !== null)

async function showMore() {
  view.value = 'more'
  moments.value = (await git.reflogMoments()) ?? []
}

async function lookForStashes() {
  looking.value = true
  try {
    lost.value = (await git.lostStashes()) ?? []
  } finally {
    looking.value = false
  }
}

async function restore(stash: LostStash) {
  if ((await git.restoreStash(stash.oid)) === null) return
  lost.value = (lost.value ?? []).filter((one) => one.oid !== stash.oid)
}

async function branchHere(name: string) {
  const moment = naming.value
  naming.value = null
  if (moment) await git.createBranchAt(name, moment.oid)
  emit('close')
}

async function forget(id: number) {
  await git.forgetStep(id)
}

function closeDialog() {
  naming.value = null
  resetting.value = null
  emit('close')
}
</script>

<template>
  <div v-if="!dialogUp" class="scrim" @click="emit('close')">
    <div class="menu" :class="{ wide: view === 'more' }" @click.stop>
      <template v-if="view === 'steps'">
        <div class="section-title">Undo</div>
        <p v-if="!undo.length" class="empty faint">Nothing to undo yet.</p>
        <div v-for="(entry, index) in undo" :key="entry.id" class="row">
          <button
            class="item"
            :class="{ gone: entry.unavailable }"
            :disabled="index > 0 || store.busy || !!entry.unavailable"
            :title="
              entry.unavailable ??
              (index > 0 ? 'Undo the step above first' : `Undo ${entry.label}`)
            "
            @click="((emit('close')), git.undo())"
          >
            <Undo2 :size="14" class="glyph" />
            <span class="grow truncate">{{ entry.label }}</span>
            <TriangleAlert
              v-if="entry.destructive"
              :size="13"
              class="warn"
              title="Touches your working tree"
            />
            <span class="faint when">{{ relativeTime(entry.at) }}</span>
          </button>
          <button
            v-if="entry.unavailable"
            class="forget faint"
            :disabled="store.busy"
            :title="`${entry.unavailable}. Take it off the history.`"
            @click="forget(entry.id)"
          >
            Forget
          </button>
        </div>

        <template v-if="redo.length">
          <div class="divider" />
          <div class="section-title">Redo</div>
          <div v-for="(entry, index) in redo" :key="entry.id" class="row">
            <button
              class="item"
              :class="{ gone: entry.unavailable }"
              :disabled="index > 0 || store.busy || !!entry.unavailable"
              :title="entry.unavailable ?? `Redo ${entry.label}`"
              @click="((emit('close')), git.redo())"
            >
              <Redo2 :size="14" class="glyph" />
              <span class="grow truncate">{{ entry.label }}</span>
              <span class="faint when">{{ relativeTime(entry.at) }}</span>
            </button>
            <button
              v-if="entry.unavailable"
              class="forget faint"
              :disabled="store.busy"
              :title="`${entry.unavailable}. Take it off the history.`"
              @click="forget(entry.id)"
            >
              Forget
            </button>
          </div>
        </template>

        <p class="note faint">
          Undo moves the branch pointer; it never throws away a commit object. Steps marked with a
          warning also change your working tree. Greyed-out steps no longer fit the repository.
        </p>
        <button class="item more" @click="showMore">
          <History :size="14" class="glyph" />
          <span class="grow">More history…</span>
        </button>
      </template>

      <template v-else>
        <button class="item back" @click="view = 'steps'">
          <ArrowLeft :size="14" class="glyph" />
          <span class="grow">Where HEAD has been</span>
        </button>
        <p v-if="moments === null" class="empty faint">Reading the reflog…</p>
        <p v-else-if="!moments.length" class="empty faint">The reflog is empty.</p>
        <div v-for="moment in moments ?? []" :key="`${moment.oid}-${moment.time}-${moment.action}`" class="moment">
          <div class="moment-text">
            <div class="truncate">
              <span class="mono faint">{{ moment.short }}</span>
              {{ moment.subject }}
            </div>
            <div class="faint small truncate" :title="moment.action">
              {{ relativeTime(moment.time) }} · {{ moment.action }}
            </div>
          </div>
          <button
            class="act"
            title="Make a new branch at this commit. Nothing else changes."
            :disabled="store.busy"
            @click="naming = moment"
          >
            <GitBranch :size="13" /> Branch
          </button>
          <button
            class="act"
            :title="
              store.repo && !store.repo.detached
                ? `Move ${store.repo.head} here, keeping your open work`
                : 'Move the branch here, keeping your open work'
            "
            :disabled="store.busy"
            @click="resetting = moment"
          >
            <RotateCcw :size="13" /> Reset
          </button>
        </div>

        <div class="divider" />
        <div class="section-title">Dropped stashes</div>
        <p v-if="lost === null" class="empty faint">
          Stashes that were dropped are kept for a while. Looking for them reads the whole
          repository, so it can take a moment.
        </p>
        <button v-if="lost === null" class="item" :disabled="looking" @click="lookForStashes">
          <History :size="14" class="glyph" />
          <span class="grow">{{ looking ? 'Looking…' : 'Look for dropped stashes' }}</span>
        </button>
        <p v-else-if="!lost.length" class="empty faint">None found.</p>
        <div v-for="stash in lost ?? []" :key="stash.oid" class="moment">
          <div class="moment-text">
            <div class="truncate">{{ stash.message }}</div>
            <div class="faint small">{{ relativeTime(stash.time) }} · {{ stash.short }}</div>
          </div>
          <button
            class="act"
            title="Put it back on the stash list"
            :disabled="store.busy"
            @click="restore(stash)"
          >
            Restore
          </button>
        </div>
      </template>
    </div>
  </div>

  <PromptDialog
    v-if="naming"
    title="Branch from here"
    label="Branch name"
    :hint="`Made at ${naming.short}. Nothing is checked out or moved.`"
    confirm="Create branch"
    @close="closeDialog"
    @submit="branchHere"
  />
  <ResetDialog v-if="resetting" :oid="resetting.oid" keep @close="closeDialog" />
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 45;
}

.menu {
  position: absolute;
  /* Under the history button: the tab strip and the toolbar are 38 and 52
     pixels, and the button sits left of settings and the profile. */
  right: 150px;
  top: 84px;
  width: 380px;
  max-height: 60vh;
  overflow-y: auto;
  padding: 4px;
  background: var(--bg);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-pop);
  animation: pop-in 0.08s ease-out;
}

.menu.wide {
  width: 460px;
  max-height: 70vh;
}

@keyframes pop-in {
  from {
    opacity: 0;
    transform: translateY(-3px);
  }
}

.row {
  display: flex;
  align-items: center;
  gap: 4px;
}

.item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 26px;
  padding: 3px 8px;
  border-radius: var(--radius-sm);
  text-align: left;
  font-size: 12.5px;
}

/* Filled in the accent under the pointer, as a native menu marks its row. */
.item:hover:not(:disabled) {
  background: var(--accent);
  color: var(--on-accent);
}

.item:hover:not(:disabled) .faint,
.item:hover:not(:disabled) .warn {
  color: var(--on-accent);
}

.item:disabled {
  opacity: 0.45;
}

.item.gone .grow {
  text-decoration: line-through;
}

.forget,
.act {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 7px;
  border-radius: var(--radius-sm);
  font-size: 11.5px;
}

.forget:hover:not(:disabled),
.act:hover:not(:disabled) {
  background: var(--surface);
  color: var(--text);
}

.glyph {
  flex: none;
  opacity: 0.7;
}

.grow {
  flex: 1;
  min-width: 0;
}

.warn {
  color: var(--amber);
  flex: none;
}

.when {
  font-size: 10.5px;
  white-space: nowrap;
}

.empty,
.note {
  padding: 4px 8px 8px;
  font-size: 11.5px;
  margin: 0;
}

.note {
  border-top: 1px solid var(--line-soft);
  margin-top: 5px;
  padding-top: 8px;
}

.more,
.back {
  margin-top: 2px;
}

.moment {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  border-radius: var(--radius-sm);
  font-size: 12.5px;
}

.moment:hover {
  background: var(--surface);
}

.moment-text {
  flex: 1;
  min-width: 0;
}

.small {
  font-size: 11px;
}

.divider {
  height: 1px;
  margin: 4px 6px;
  background: var(--line-soft);
}
</style>
