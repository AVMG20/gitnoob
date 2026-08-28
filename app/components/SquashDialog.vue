<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import { TriangleAlert } from 'lucide-vue-next'
import { relativeTime, useGit, type SquashPreview } from '~/composables/useGit'

/**
 * Folding a run of commits into one.
 *
 * The message is the whole point of the dialog. Git's own squash joins the
 * messages of everything it folded and drops you in an editor to make one
 * message out of them; there is no editor here, so the join arrives already
 * written and the box is where it gets cut down.
 *
 * Whether the fold is possible at all is the backend's answer, not a guess
 * made here: it is the one that knows what sits between the chosen commits.
 */
const props = defineProps<{ oids: string[] }>()
const emit = defineEmits<{ close: []; done: [] }>()

const git = useGit()
const store = git.store

const preview = ref<SquashPreview | null>(null)
/** Set when the preview could not be read at all — a null with a reason. */
const failed = ref(false)
const message = ref('')
const box = ref<HTMLTextAreaElement | null>(null)

const commits = computed(() => preview.value?.commits ?? [])
const pushed = computed(() => commits.value.filter((one) => one.pushed).length)
const refusal = computed(() => preview.value?.refusal ?? null)
const ready = computed(() => !!preview.value && !refusal.value && !!message.value.trim())

onMounted(async () => {
  preview.value = await git.squashPreview(props.oids)
  // The toast already carries git's words; the dialog must not sit on
  // "Reading those commits…" for ever as though it were still working.
  failed.value = preview.value === null
  message.value = preview.value?.message ?? ''
  if (!preview.value || preview.value.refusal) return
  await nextTick()
  box.value?.focus()
  // The cursor lands at the end of the first line rather than the end of the
  // join: the summary is the line that has to be rewritten, and every other
  // line is context for writing it.
  const at = message.value.indexOf('\n')
  box.value?.setSelectionRange(at < 0 ? message.value.length : at, at < 0 ? message.value.length : at)
})

async function apply() {
  if (!ready.value || store.busy) return
  const said = await git.squash(props.oids, message.value.trim())
  if (said === null) return
  git.note(said)
  emit('done')
  emit('close')
}
</script>

<template>
  <AppModal
    :title="`Squash ${commits.length || props.oids.length} commits into one`"
    :width="620"
    @close="emit('close')"
  >
    <p v-if="failed" class="note bad">
      Could not read those commits — the activity log has git's answer.
    </p>
    <p v-else-if="!preview" class="dim">Reading those commits…</p>

    <template v-else>
      <div class="block">
        <div class="block-head">
          {{ commits.length }} {{ commits.length === 1 ? 'commit' : 'commits' }} become one
        </div>
        <ul class="commits">
          <li v-for="commit in commits" :key="commit.oid">
            <span class="mono faint">{{ commit.short }}</span>
            <span class="truncate">{{ commit.summary }}</span>
            <span v-if="commit.pushed" class="tag">pushed</span>
            <span class="faint when">{{ relativeTime(commit.time) }}</span>
          </li>
        </ul>
      </div>

      <p v-if="refusal" class="note bad">
        <TriangleAlert :size="13" />
        <span>{{ refusal }}</span>
      </p>

      <template v-else>
        <label class="field">
          <span class="label">The message the one commit carries</span>
          <textarea
            ref="box"
            v-model="message"
            rows="7"
            placeholder="Summary on the first line, why it changed below"
            @keydown.meta.enter="apply"
            @keydown.ctrl.enter="apply"
          />
        </label>

        <p class="dim small">
          {{
            preview.onto
              ? `They fold onto ${preview.onto}.`
              : 'They fold together at the start of the history.'
          }}
          <template v-if="preview.above === 1">
            The commit above is replayed onto the result, so it gets a new hash too.
          </template>
          <template v-else-if="preview.above">
            The {{ preview.above }} commits above are replayed onto the result, so they get new
            hashes too.
          </template>
          Undo puts every one of them back.
        </p>

        <p v-if="pushed" class="note">
          <TriangleAlert :size="13" />
          <span>
            {{ pushed }} of {{ commits.length === pushed ? 'them' : `the ${commits.length}` }}
            {{ pushed === 1 ? 'is' : 'are' }} already on a remote. Publishing
            {{ preview.branch ?? 'this branch' }} afterwards needs a force push.
          </span>
        </p>
      </template>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button class="btn btn-primary" :disabled="store.busy || !ready" @click="apply">
        Squash {{ commits.length }} into one
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.block {
  padding: 11px;
  border: 1px solid var(--line);
  border-radius: 7px;
  margin-bottom: 14px;
}

.block-head {
  font-weight: 600;
  font-size: 12.5px;
  margin-bottom: 4px;
}

.commits {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 150px;
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

.tag {
  flex: none;
  padding: 0 5px;
  border-radius: 4px;
  font-size: 10px;
  letter-spacing: 0.04em;
  color: var(--amber-soft);
  background: var(--warning-bg);
  border: 1px solid var(--warning-line);
}

.when {
  margin-left: auto;
  white-space: nowrap;
  font-size: 11px;
}

.field {
  display: block;
}

.label {
  display: block;
  margin-bottom: 5px;
  font-size: 11px;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-faint);
}

textarea {
  width: 100%;
  display: block;
  resize: vertical;
}

.small {
  font-size: 11.5px;
  line-height: 1.5;
  margin: 8px 0 0;
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

.note.bad {
  color: var(--red-soft);
  background: var(--danger-bg);
  border-color: var(--danger-line);
}
</style>
