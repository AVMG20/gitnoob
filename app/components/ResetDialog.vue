<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { TriangleAlert } from 'lucide-vue-next'
import { relativeTime, useGit, type ResetPreview } from '~/composables/useGit'

/**
 * The question asked before a hard reset.
 *
 * Only this one asks. A soft or mixed reset keeps every change on disk and the
 * commits it takes off the branch are a keystroke away in undo, so those run
 * on the click that asked for them; the mode is chosen in the menu, not here.
 * What is left is the one reset that writes over the working tree, and the two
 * things worth knowing before it does: which commits leave the branch, and how
 * much uncommitted work goes with them.
 */
const props = defineProps<{ oid: string }>()
const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const preview = ref<ResetPreview | null>(null)
const acknowledged = ref(false)

const dropped = computed(() => preview.value?.dropped ?? [])
const dirty = computed(
  () => (preview.value?.staged_files ?? 0) + (preview.value?.unstaged_files ?? 0)
)
/** The tick is only asked for when there is work on disk to lose. */
const blocked = computed(() => dirty.value > 0 && !acknowledged.value)

async function apply() {
  if (blocked.value) return
  // Null means it failed; any string, empty or not, means it ran.
  if ((await git.reset(props.oid, 'hard')) !== null) emit('close')
}

onMounted(async () => {
  preview.value = await git.resetPreview(props.oid)
})
</script>

<template>
  <AppModal
    :title="`Hard reset ${preview?.branch ?? 'branch'} to ${preview?.short ?? ''}`"
    :width="560"
    @close="emit('close')"
  >
    <p v-if="!preview" class="dim">Working out what this would do…</p>

    <template v-else>
      <p class="target">
        <span class="mono">{{ preview.short }}</span>
        <span class="truncate">{{ preview.summary }}</span>
      </p>

      <div v-if="dropped.length" class="block">
        <div class="block-head">
          {{ dropped.length }} {{ dropped.length === 1 ? 'commit leaves' : 'commits leave' }}
          {{ preview.branch }}. Undo brings them back.
        </div>
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
        Sideways, not back: that commit is not on {{ preview.branch }}.
      </p>

      <!-- The one thing said about uncommitted work, and it is the tick: saying
           it in a banner above as well was the same sentence twice. -->
      <label v-if="dirty" class="ack">
        <input v-model="acknowledged" type="checkbox" />
        Throw away {{ dirty }} uncommitted {{ dirty === 1 ? 'change' : 'changes' }}. Stash to keep
        them.
      </label>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn btn-danger"
        :disabled="store.busy || !preview || blocked"
        @click="apply"
      >
        Hard reset
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.target {
  display: flex;
  gap: 9px;
  margin: 0 0 4px;
  font-size: 13px;
}

.what {
  margin: 0;
  font-size: 12px;
}

.block {
  margin-top: 14px;
  padding: 11px;
  border: 1px solid var(--line);
  border-radius: 7px;
}

.block-head {
  font-size: 12.5px;
  margin-bottom: 5px;
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

.note {
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

.ack {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-top: 14px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--text);
  cursor: pointer;
}
</style>
