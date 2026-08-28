<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { TriangleAlert } from 'lucide-vue-next'
import { relativeTime, useGit, type ResetMode, type ResetPreview } from '~/composables/useGit'

const props = defineProps<{
  oid: string
  /** Preselected when the caller already asked which kind of reset. */
  mode?: ResetMode
}>()
const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const preview = ref<ResetPreview | null>(null)
const mode = ref<ResetMode>(props.mode ?? 'mixed')
const acknowledged = ref(false)

const dropped = computed(() => preview.value?.dropped ?? [])
const dirty = computed(
  () => (preview.value?.staged_files ?? 0) + (preview.value?.unstaged_files ?? 0)
)
/** Only a hard reset can destroy work that is not in a commit. */
const risky = computed(() => mode.value === 'hard' && dirty.value > 0)
const blocked = computed(() => risky.value && !acknowledged.value)

const modes: { id: ResetMode; title: string; what: string }[] = [
  {
    id: 'soft',
    title: 'Soft',
    what: 'Moves the branch. Everything those commits changed ends up staged, ready to recommit.'
  },
  {
    id: 'mixed',
    title: 'Mixed',
    what: 'Moves the branch. The changes stay in your files but are no longer staged.'
  },
  {
    id: 'hard',
    title: 'Hard',
    what: 'Moves the branch and rewrites your files to match. Uncommitted work is destroyed.'
  }
]

async function apply() {
  if (blocked.value) return
  // Null means it failed; any string, empty or not, means it ran.
  if ((await git.reset(props.oid, mode.value)) !== null) emit('close')
}

onMounted(async () => {
  preview.value = await git.resetPreview(props.oid)
})
</script>

<template>
  <AppModal
    :title="`Move ${preview?.branch ?? 'branch'} to ${preview?.short ?? ''}`"
    :width="620"
    @close="emit('close')"
  >
    <p v-if="!preview" class="dim">Working out what this would do…</p>

    <template v-else>
      <p class="target">
        <span class="mono">{{ preview.short }}</span>
        <span class="truncate">{{ preview.summary }}</span>
      </p>

      <div class="modes">
        <button
          v-for="option in modes"
          :key="option.id"
          class="mode"
          :class="{ on: mode === option.id, danger: option.id === 'hard' }"
          @click="mode = option.id"
        >
          <span class="mode-title">{{ option.title }}</span>
          <span class="mode-what">{{ option.what }}</span>
        </button>
      </div>

      <div v-if="dropped.length" class="block">
        <div class="block-head">
          {{ dropped.length }} {{ dropped.length === 1 ? 'commit' : 'commits' }} would no longer be
          on {{ preview.branch }}
        </div>
        <p class="dim small">
          The commits themselves survive — undo brings the branch straight back, and they stay
          reachable until git eventually collects them.
        </p>
        <ul class="commits">
          <li v-for="commit in dropped" :key="commit.oid">
            <span class="mono faint">{{ commit.short }}</span>
            <span class="truncate">{{ commit.summary }}</span>
            <span class="faint when">{{ relativeTime(commit.time) }}</span>
          </li>
        </ul>
      </div>

      <p v-if="preview.diverges" class="note">
        <TriangleAlert :size="13" />
        That commit is not an ancestor of {{ preview.branch }}, so this moves the branch sideways
        rather than rewinding it.
      </p>

      <div v-if="dirty" class="dirty" :class="{ bad: mode === 'hard' }">
        <TriangleAlert :size="13" />
        <span v-if="mode === 'hard'">
          You have {{ dirty }} uncommitted {{ dirty === 1 ? 'change' : 'changes' }}. A hard reset
          throws them away — stash first if you might want them.
        </span>
        <span v-else>
          Your {{ dirty }} uncommitted {{ dirty === 1 ? 'change' : 'changes' }} will be left alone.
        </span>
      </div>

      <label v-if="risky" class="ack">
        <input v-model="acknowledged" type="checkbox" />
        I understand {{ dirty }} uncommitted
        {{ dirty === 1 ? 'change' : 'changes' }} will be destroyed
      </label>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn"
        :class="mode === 'hard' ? 'btn-danger' : 'btn-primary'"
        :disabled="store.busy || !preview || blocked"
        @click="apply"
      >
        {{ mode === 'hard' ? 'Hard reset' : `${mode === 'soft' ? 'Soft' : 'Mixed'} reset` }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.target {
  display: flex;
  gap: 9px;
  margin: 0 0 14px;
  font-size: 13px;
}

.modes {
  display: grid;
  gap: 6px;
}

.mode {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 9px 11px;
  text-align: left;
  border: 1px solid var(--line);
  border-radius: 7px;
}

.mode:hover {
  background: var(--bg-hover);
}

.mode.on {
  border-color: var(--accent);
  background: color-mix(in srgb, var(--accent) 10%, transparent);
}

.mode.on.danger {
  border-color: var(--red);
  background: var(--danger-bg);
}

.mode-title {
  font-weight: 600;
  font-size: 12.5px;
}

.mode-what {
  font-size: 11.5px;
  color: var(--text-dim);
  line-height: 1.45;
}

.block {
  margin-top: 14px;
  padding: 11px;
  border: 1px solid var(--line);
  border-radius: 7px;
}

.block-head {
  font-weight: 600;
  font-size: 12.5px;
  margin-bottom: 3px;
}

.small {
  font-size: 11.5px;
  margin: 0 0 8px;
  line-height: 1.5;
}

.commits {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 160px;
  overflow: auto;
}

.commits li {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 3px 0;
  font-size: 12px;
  border-top: 1px solid var(--line-soft);
}

.commits li:first-child {
  border-top: none;
}

.when {
  margin-left: auto;
  white-space: nowrap;
  font-size: 11px;
}

.note,
.dirty {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin: 12px 0 0;
  padding: 9px 11px;
  border-radius: 7px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--amber-soft);
  background: var(--warning-bg);
  border: 1px solid var(--warning-line);
}

.dirty:not(.bad) {
  color: var(--text-dim);
  background: var(--bg-raised);
  border-color: var(--line);
}

.dirty.bad {
  color: var(--red-soft);
  background: var(--danger-bg);
  border-color: var(--danger-line);
}

.ack {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
  font-size: 12px;
  color: var(--red);
  cursor: pointer;
}
</style>
