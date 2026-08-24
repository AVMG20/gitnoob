<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { relativeTime, useGit, type PushPreview } from '~/composables/useGit'

const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const preview = ref<PushPreview | null>(null)
const loading = ref(true)
const acknowledged = ref(false)
const force = ref(false)

const diverged = computed(() => (preview.value?.behind ?? 0) > 0)
const orphans = computed(() => preview.value?.will_orphan ?? [])
const nothingToDo = computed(
  () => !!preview.value && preview.value.ahead === 0 && !preview.value.new_upstream && !diverged.value
)
const blocked = computed(() => force.value && !acknowledged.value)

async function load(fetchFirst = false) {
  loading.value = true
  preview.value = await git.pushPreview(undefined, fetchFirst)
  // A diverged branch cannot be pushed without a rewrite, so preselect force
  // rather than letting the user discover the rejection the hard way.
  force.value = diverged.value
  acknowledged.value = false
  loading.value = false
}

async function push() {
  if (!preview.value || blocked.value) return
  const ok = await git.push(
    preview.value.remote,
    preview.value.branch,
    force.value,
    preview.value.new_upstream
  )
  if (ok) emit('close')
  else await load()
}

onMounted(() => load())
</script>

<template>
  <AppModal
    :title="force ? 'Force push' : 'Push'"
    :tone="force ? 'danger' : 'normal'"
    :width="620"
    @close="emit('close')"
  >
    <p v-if="loading" class="dim">Reading branch state…</p>

    <template v-else-if="preview">
      <div class="route mono">
        <span class="local">{{ preview.branch }}</span>
        <span class="faint">→</span>
        <span class="remote">{{ preview.upstream ?? `${preview.remote}/${preview.branch}` }}</span>
        <span v-if="preview.new_upstream" class="pill">new branch</span>
      </div>

      <div class="counts">
        <span :class="{ on: preview.ahead > 0 }">{{ preview.ahead }} to push</span>
        <span :class="{ warn: preview.behind > 0 }">{{ preview.behind }} only on remote</span>
        <button class="btn btn-ghost recheck" :disabled="store.busy" @click="load(true)">
          Fetch and re-check
        </button>
      </div>

      <!-- The whole point of this dialog: name the commits a rewrite destroys. -->
      <div v-if="diverged" class="warning">
        <div class="warning-head">
          {{ orphans.length }}
          {{ orphans.length === 1 ? 'commit' : 'commits' }} on
          <span class="mono">{{ preview.upstream }}</span>
          {{ orphans.length === 1 ? 'is' : 'are' }} not in your branch
        </div>
        <p class="dim">
          Force pushing replaces the remote branch with yours. These commits stop being
          reachable from it — if they exist nowhere else, they are lost.
        </p>
        <ul class="commits">
          <li v-for="commit in orphans" :key="commit.oid">
            <span class="mono faint">{{ commit.short }}</span>
            <span class="truncate">{{ commit.summary }}</span>
            <span class="faint who">{{ commit.author }}, {{ relativeTime(commit.time) }}</span>
          </li>
        </ul>
        <p class="lease">
          gitui pushes with
          <span class="mono">--force-with-lease</span>, never a plain
          <span class="mono">--force</span>: if the remote moved since your last fetch, the push is
          refused instead of overwriting work you have not seen.
        </p>
      </div>

      <div v-else-if="preview.will_push.length" class="commits-plain">
        <div class="section-title">Pushing</div>
        <ul class="commits">
          <li v-for="commit in preview.will_push.slice(0, 12)" :key="commit.oid">
            <span class="mono faint">{{ commit.short }}</span>
            <span class="truncate">{{ commit.summary }}</span>
          </li>
        </ul>
      </div>

      <p v-else-if="nothingToDo" class="dim">Nothing to push — the remote already has this branch.</p>

      <label v-if="diverged" class="ack">
        <input v-model="force" type="checkbox" />
        Rewrite the remote branch (force push with lease)
      </label>
      <label v-if="force" class="ack danger-text">
        <input v-model="acknowledged" type="checkbox" />
        I understand {{ orphans.length }}
        {{ orphans.length === 1 ? 'commit' : 'commits' }} will be dropped from
        <span class="mono">{{ preview.upstream }}</span>
      </label>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn"
        :class="force ? 'btn-danger' : 'btn-primary'"
        :disabled="store.busy || loading || blocked || nothingToDo"
        @click="push"
      >
        {{ force ? 'Force push with lease' : 'Push' }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.route {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
}

.local {
  color: var(--accent);
}

.remote {
  color: var(--purple);
}

.counts {
  display: flex;
  align-items: center;
  gap: 14px;
  margin: 10px 0 4px;
  color: var(--text-faint);
  font-size: 12px;
}

.counts .on {
  color: var(--green);
}

.counts .warn {
  color: var(--amber);
}

.recheck {
  margin-left: auto;
  font-size: 12px;
  padding: 3px 8px;
}

.warning {
  margin-top: 12px;
  padding: 12px;
  border: 1px solid rgba(224, 87, 109, 0.4);
  background: rgba(224, 87, 109, 0.07);
  border-radius: 7px;
}

.warning-head {
  font-weight: 600;
  color: var(--red);
  margin-bottom: 4px;
}

.warning p {
  margin: 0 0 8px;
  font-size: 12px;
}

.lease {
  margin: 10px 0 0 !important;
  color: var(--text-faint);
}

.commits {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 190px;
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

.who {
  margin-left: auto;
  white-space: nowrap;
  font-size: 11px;
}

.commits-plain {
  margin-top: 10px;
}

.ack {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
  font-size: 12px;
  cursor: pointer;
}

.danger-text {
  color: var(--red);
}
</style>
