<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { CircleCheck, Info, TriangleAlert } from 'lucide-vue-next'
import { useGit, type BranchDeletion } from '~/composables/useGit'
import { localVerdict, needsForce, remoteVerdict } from '~/composables/useBranchDeletion'

const props = defineProps<{ name: string }>()
const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const preview = ref<BranchDeletion | null>(null)
const acknowledged = ref(false)

/** Deleting the branch here, and deleting the copy on its remote, are two
 *  questions with two costs. Each gets its own answer. */
const local = computed(() => (preview.value ? localVerdict(preview.value) : null))
const remote = computed(() => (preview.value ? remoteVerdict(preview.value) : null))

/** One tick box, shown when either half of what is on offer needs one. */
const asks = computed(() => !!local.value?.acknowledge || !!remote.value?.acknowledge)
const localBlocked = computed(() => !!local.value?.acknowledge && !acknowledged.value)
const remoteBlocked = computed(() => asks.value && !acknowledged.value)

const icon = { safe: CircleCheck, careful: Info, danger: TriangleAlert }

async function remove(alsoRemote: boolean) {
  const found = preview.value
  if (!found) return
  if (alsoRemote ? remoteBlocked.value : localBlocked.value) return

  // Force only when git would otherwise refuse, and only once anything at
  // stake has been read and ticked.
  if ((await git.deleteBranch(found.name, needsForce(found))) === null) return
  // Only ever the branch's own remote. A fork or a mirror carrying the same
  // name belongs to somebody else and is left alone.
  if (alsoRemote && found.remote) {
    await git.deleteRemoteBranch(found.remote.remote, found.name)
  }
  emit('close')
}

onMounted(async () => {
  preview.value = await git.deleteBranchPreview(props.name)
})
</script>

<template>
  <AppModal
    :title="`Delete ${props.name}?`"
    :width="460"
    @close="emit('close')"
  >
    <p v-if="!preview" class="dim">Checking…</p>

    <p v-else-if="preview.is_head" class="verdict careful">
      <Info :size="14" class="glyph" />
      <span>You are on this branch. Switch to another one first.</span>
    </p>

    <template v-else>
      <!-- One line each. The buttons carry the risk in their colour, so
           nothing here has to spell out what deleting means. -->
      <p v-if="local" class="verdict" :class="local.tone">
        <component :is="icon[local.tone]" :size="14" class="glyph" />
        <span>
          {{ local.headline }}<template v-if="local.detail"> — {{ local.detail }}</template>
        </span>
      </p>

      <p v-if="remote" class="verdict" :class="remote.tone">
        <component :is="icon[remote.tone]" :size="14" class="glyph" />
        <span>
          {{ remote.headline }}<template v-if="remote.detail"> — {{ remote.detail }}</template>
        </span>
      </p>

      <p v-if="preview.other_remotes.length" class="verdict careful">
        <Info :size="14" class="glyph" />
        <span>
          Same name on
          <span v-for="(full, i) in preview.other_remotes" :key="full">
            <span class="mono">{{ full }}</span
            ><span v-if="i < preview.other_remotes.length - 1">, </span> </span
          >, left alone.
        </span>
      </p>

      <label v-if="asks" class="ack">
        <input v-model="acknowledged" type="checkbox" />
        I understand what this loses
      </label>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">
        {{ preview?.is_head ? 'Close' : 'Cancel' }}
      </button>
      <template v-if="preview && !preview.is_head">
        <button
          class="btn"
          :class="local?.tone === 'danger' ? 'btn-danger' : 'btn-primary'"
          :disabled="store.busy || localBlocked"
          @click="remove(false)"
        >
          {{ preview.remote ? 'Delete here' : 'Delete' }}
        </button>
        <button
          v-if="preview.remote"
          class="btn"
          :class="remote?.tone === 'danger' || local?.tone === 'danger' ? 'btn-danger' : 'btn-primary'"
          :disabled="store.busy || remoteBlocked"
          @click="remove(true)"
        >
          Delete here and on {{ preview.remote.remote }}
        </button>
      </template>
    </template>
  </AppModal>
</template>

<style scoped>
/* A verdict is a line, not a panel. Only the case that loses work gets the
   weight of a box; the rest is a coloured line and its glyph. */
.verdict {
  display: flex;
  align-items: flex-start;
  gap: 7px;
  margin: 0 0 6px;
  font-size: 12.5px;
  line-height: 1.5;
}

.verdict .glyph {
  flex: none;
  margin-top: 2px;
}

.verdict.safe {
  color: var(--green-soft);
}

.verdict.careful {
  color: var(--text-dim);
}

.verdict.danger {
  color: var(--red-soft);
  background: var(--danger-bg);
  border: 1px solid var(--danger-line);
  border-radius: 7px;
  padding: 8px 10px;
}

.ack {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
  font-size: 12px;
  color: var(--red);
  cursor: pointer;
}
</style>
