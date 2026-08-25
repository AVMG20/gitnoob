<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { TriangleAlert } from 'lucide-vue-next'
import { useGit, type BranchDeletion } from '~/composables/useGit'

const props = defineProps<{ name: string }>()
const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const preview = ref<BranchDeletion | null>(null)
const acknowledged = ref(false)

/** Nothing is lost when HEAD can already reach it and the remote has it all. */
const safe = computed(
  () => !!preview.value && preview.value.merged && preview.value.unpushed === 0
)
const blocked = computed(() => !safe.value && !acknowledged.value)

async function remove(alsoRemote: boolean) {
  const found = preview.value
  if (!found || blocked.value) return

  // Force only when git would otherwise refuse, and only once the warning
  // above has been ticked.
  if ((await git.deleteBranch(found.name, !found.merged)) === null) return
  if (alsoRemote) {
    for (const full of found.remotes) {
      const [remote, ...rest] = full.split('/')
      await git.deleteRemoteBranch(remote, rest.join('/'))
    }
  }
  emit('close')
}

onMounted(async () => {
  preview.value = await git.deleteBranchPreview(props.name)
  // A branch with nothing at stake needs no tick, only a press.
  if (preview.value?.is_head) emit('close')
})
</script>

<template>
  <AppModal
    :title="`Delete ${props.name}?`"
    :width="540"
    :tone="safe ? 'normal' : 'danger'"
    @close="emit('close')"
  >
    <p v-if="!preview" class="dim">Working out what this would lose…</p>

    <template v-else>
      <p v-if="safe" class="dim line">
        Every commit on <span class="mono">{{ preview.name }}</span> is already reachable from the
        branch you are on, so deleting it loses nothing.
      </p>

      <div v-else class="warn">
        <TriangleAlert :size="14" />
        <span>
          <template v-if="preview.unpushed">
            {{ preview.unpushed }}
            {{ preview.unpushed === 1 ? 'commit is' : 'commits are' }} on
            <span class="mono">{{ preview.name }}</span> and not on
            <span class="mono">{{ preview.upstream }}</span
            >.
          </template>
          <template v-else-if="!preview.merged">
            <span class="mono">{{ preview.name }}</span> has not been merged into the branch you
            are on, and it has no upstream holding a copy.
          </template>
          Deleting it leaves those commits reachable from nothing, and git will eventually collect
          them.
        </span>
      </div>

      <p v-if="preview.remotes.length" class="line">
        It also exists on
        <span v-for="(full, i) in preview.remotes" :key="full">
          <span class="mono">{{ full }}</span
          ><span v-if="i < preview.remotes.length - 1">, </span> </span
        >. Deleting there removes it for everyone, not only here.
      </p>

      <label v-if="!safe" class="ack">
        <input v-model="acknowledged" type="checkbox" />
        I understand what this loses
      </label>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn"
        :class="safe ? 'btn-primary' : 'btn-danger'"
        :disabled="store.busy || !preview || blocked"
        @click="remove(false)"
      >
        {{ preview?.remotes.length ? 'Delete here only' : 'Delete branch' }}
      </button>
      <button
        v-if="preview?.remotes.length"
        class="btn btn-danger"
        :disabled="store.busy || blocked"
        @click="remove(true)"
      >
        Delete here and on {{ preview.remotes.map((r) => r.split('/')[0]).join(', ') }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.line {
  margin: 0 0 12px;
  font-size: 12.5px;
  line-height: 1.55;
}

.warn {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin: 0 0 12px;
  padding: 10px 11px;
  border-radius: 7px;
  font-size: 12.5px;
  line-height: 1.55;
  color: var(--red-soft);
  background: rgba(224, 87, 109, 0.08);
  border: 1px solid rgba(224, 87, 109, 0.35);
}

.ack {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
  font-size: 12px;
  color: var(--red);
  cursor: pointer;
}
</style>
