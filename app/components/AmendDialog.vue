<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useGit, type AmendDraft } from '~/composables/useGit'

const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const draft = ref<AmendDraft | null>(null)
const summary = ref('')
const body = ref('')
const acknowledged = ref(false)

const staged = computed(() => store.status?.staged.length ?? 0)
const blocked = computed(
  () => !summary.value.trim() || (draft.value?.is_pushed === true && !acknowledged.value)
)

async function submit() {
  if (blocked.value) return
  const message = body.value.trim() ? `${summary.value.trim()}\n\n${body.value.trim()}` : summary.value.trim()
  if (await git.commit(message, true)) emit('close')
}

onMounted(async () => {
  draft.value = await git.amendDraft()
  summary.value = draft.value?.summary ?? ''
  body.value = draft.value?.body ?? ''
})
</script>

<template>
  <AppModal title="Amend last commit" :width="560" @close="emit('close')">
    <p v-if="!draft" class="dim">Reading HEAD…</p>

    <template v-else>
      <p class="dim intro">
        Amending replaces commit <span class="mono">{{ draft.short }}</span>
        <template v-if="staged">
          and folds in {{ staged }} staged {{ staged === 1 ? 'change' : 'changes' }}</template
        >.
      </p>

      <label class="field">
        <span class="label">Summary</span>
        <input v-model="summary" type="text" placeholder="What changed" />
      </label>

      <label class="field">
        <span class="label">Body</span>
        <textarea v-model="body" rows="6" placeholder="Why it changed" />
      </label>

      <!-- Amending a published commit rewrites history others may already have. -->
      <div v-if="draft.is_pushed" class="warning">
        <div class="warning-head">This commit is already on a remote</div>
        <p>
          Amending gives it a new hash, so the branch diverges from its upstream and can only be
          published with a force push. Anyone who already fetched it will have to reset.
        </p>
        <label class="ack">
          <input v-model="acknowledged" type="checkbox" />
          I understand this rewrites published history
        </label>
      </div>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn"
        :class="draft?.is_pushed ? 'btn-danger' : 'btn-primary'"
        :disabled="store.busy || blocked"
        @click="submit"
      >
        Amend commit
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.intro {
  margin: 0 0 14px;
  font-size: 12px;
}

.field {
  display: block;
  margin-bottom: 12px;
}

.label {
  display: block;
  margin-bottom: 4px;
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

.field input,
.field textarea {
  width: 100%;
}

.warning {
  padding: 11px;
  border: 1px solid rgba(240, 168, 60, 0.4);
  background: rgba(240, 168, 60, 0.07);
  border-radius: 7px;
}

.warning-head {
  font-weight: 600;
  color: var(--amber);
}

.warning p {
  margin: 4px 0 9px;
  font-size: 12px;
  color: var(--text-dim);
}

.ack {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  cursor: pointer;
}
</style>
