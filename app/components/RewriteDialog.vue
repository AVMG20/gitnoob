<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { TriangleAlert } from 'lucide-vue-next'
import { useGit } from '~/composables/useGit'
import {
  useRewrite,
  type AbsorbPlan,
  type RewriteCheck
} from '~/composables/useRewrite'

/**
 * The one question every rewrite from a commit asks before it runs.
 *
 * Fixing a commit up, giving it a new message, splitting it, and absorbing
 * staged hunks all rewrite history, so each shows what it would do — which
 * commit, how many above it get new hashes, whether a remote already has
 * them — and only then asks. What the backend refuses (a merge in the way, a
 * rebase already running) is shown as the answer, not left for an error.
 */
const git = useGit()
const rewrite = useRewrite()
const store = git.store

const dialog = computed(() => rewrite.store.dialog)
const check = ref<RewriteCheck | null>(null)
const plan = ref<AbsorbPlan | null>(null)
const failed = ref(false)
/** Confirms a force push will be needed afterwards. */
const allowPushed = ref(false)
/** For fixup and absorb: fold in now, rather than leave fixup commits. */
const fold = ref(true)
const message = ref('')

const title = computed(() => {
  switch (dialog.value?.kind) {
    case 'fixup':
      return 'Fix up a commit with the staged changes'
    case 'reword':
      return 'Change a commit message'
    case 'split':
      return 'Split a commit'
    case 'absorb':
      return 'Absorb the staged changes'
    default:
      return ''
  }
})

const refusal = computed(() => {
  if (dialog.value?.kind === 'absorb') return plan.value?.refusal ?? null
  const found = check.value
  if (!found) return null
  if (found.refusal) return found.refusal
  if (dialog.value?.kind === 'fixup' && !found.staged) {
    return 'Nothing is staged. Stage the changes that belong in that commit first.'
  }
  return null
})

const absorbed = computed(() => plan.value?.hunks.filter((hunk) => hunk.target) ?? [])
const staying = computed(() => plan.value?.hunks.filter((hunk) => !hunk.target) ?? [])

const ready = computed(() => {
  if (store.busy || refusal.value) return false
  if (dialog.value?.kind === 'absorb') return !!plan.value && absorbed.value.length > 0
  if (!check.value) return false
  if (check.value.pushed && !allowPushed.value) return false
  if (dialog.value?.kind === 'reword') return !!message.value.trim()
  return true
})

onMounted(async () => {
  const now = dialog.value
  if (!now) return
  if (now.kind === 'absorb') {
    plan.value = await rewrite.absorbPlan()
    failed.value = !plan.value
    return
  }
  check.value = await rewrite.check(now.oid)
  failed.value = !check.value
  message.value = check.value?.message ?? ''
})

async function apply() {
  const now = dialog.value
  if (!now || !ready.value) return
  let said: string | null = null
  if (now.kind === 'absorb' && plan.value) {
    said = await rewrite.absorb(plan.value.fingerprint, fold.value)
  } else if (now.kind === 'fixup') {
    said = await rewrite.fixup(now.oid, fold.value, allowPushed.value)
  } else if (now.kind === 'reword') {
    said = await rewrite.reword(now.oid, message.value.trim(), allowPushed.value)
  } else if (now.kind === 'split') {
    said = await rewrite.splitStart(now.oid, allowPushed.value)
  }
  if (said !== null) rewrite.close()
}
</script>

<template>
  <AppModal :title="title" :width="600" keep @close="rewrite.close()">
    <p v-if="failed" class="note bad">
      Could not read that — the activity log has git's answer.
    </p>
    <p v-else-if="!check && !plan" class="dim">Reading…</p>

    <template v-else>
      <div v-if="check" class="block" data-test="target">
        <span class="mono faint">{{ check.short }}</span>
        <span class="truncate">{{ check.summary }}</span>
        <span v-if="check.pushed" class="tag">pushed</span>
      </div>

      <p v-if="refusal" class="note bad" data-test="refusal">
        <TriangleAlert :size="13" />
        <span>{{ refusal }}</span>
      </p>

      <template v-else>
        <!-- What each one does, in a sentence, before the button that does it. -->
        <p v-if="dialog?.kind === 'fixup' && check" class="dim small">
          The {{ check.staged }} staged {{ check.staged === 1 ? 'file becomes' : 'files become' }}
          a <span class="mono">fixup!</span> commit for it.
        </p>
        <p v-if="dialog?.kind === 'split'" class="dim small">
          The rebase stops at this commit with its changes taken out of it and left unstaged.
          Commit them in as many pieces as you like, then Continue. Abort puts the branch back as
          it was, and anything you changed on the way is kept in the stash.
        </p>
        <div v-if="dialog?.kind === 'reword'" class="field">
          <span class="label">The message</span>
          <textarea
            v-model="message"
            rows="6"
            data-test="message"
            @keydown.meta.enter="apply"
            @keydown.ctrl.enter="apply"
          />
        </div>

        <template v-if="dialog?.kind === 'absorb' && plan">
          <div class="block-head">
            {{ absorbed.length }} {{ absorbed.length === 1 ? 'hunk goes' : 'hunks go' }} into
            the commit that last touched those lines
          </div>
          <ul class="hunks" data-test="plan">
            <li v-for="(hunk, at) in plan.hunks" :key="at" :class="{ stays: !hunk.target }">
              <div class="hunk-head">
                <span class="mono">{{ hunk.path }}</span>
                <span class="faint mono">{{ hunk.header }}</span>
                <span class="arrow">→</span>
                <span v-if="hunk.target" class="truncate">
                  <span class="mono faint">{{ hunk.target.short }}</span>
                  {{ hunk.target.summary }}
                </span>
                <span v-else class="faint">stays staged</span>
              </div>
              <pre class="preview">{{ hunk.preview.join('\n') }}</pre>
              <div v-if="hunk.why" class="faint small why">{{ hunk.why }}</div>
            </li>
          </ul>
          <p v-if="plan.kept_files.length || staying.length" class="dim small">
            <template v-if="plan.kept_files.length">
              New, deleted and binary files stay as they are:
              {{ plan.kept_files.join(', ') }}.
            </template>
            Whatever stays comes back unstaged once the commits are folded.
          </p>
        </template>

        <label v-if="dialog?.kind === 'fixup' || dialog?.kind === 'absorb'" class="check">
          <input v-model="fold" type="checkbox" data-test="fold" />
          <span>
            Fold {{ dialog?.kind === 'absorb' ? 'them' : 'it' }} in now
            <span class="faint block">
              Rewrites the branch from there up with an autosquash. Off, the fixup commits are
              made and left on top for later.
            </span>
          </span>
        </label>

        <p v-if="check && check.above && dialog?.kind !== 'absorb'" class="dim small">
          {{ check.above }} {{ check.above === 1 ? 'commit sits' : 'commits sit' }} above it and
          {{ check.above === 1 ? 'gets a new hash' : 'get new hashes' }}. Undo puts the branch
          back.
        </p>

        <label v-if="check?.pushed" class="check note" data-test="pushed">
          <input v-model="allowPushed" type="checkbox" />
          <span>
            A remote already has this commit. Rewriting it means publishing with a force push
            afterwards — I understand.
          </span>
        </label>
      </template>
    </template>

    <template #footer>
      <button class="btn btn-ghost" @click="rewrite.close()">Cancel</button>
      <button class="btn btn-primary" :disabled="!ready" data-test="apply" @click="apply">
        <template v-if="dialog?.kind === 'split'">Start splitting</template>
        <template v-else-if="dialog?.kind === 'reword'">Change the message</template>
        <template v-else-if="dialog?.kind === 'absorb'">Absorb</template>
        <template v-else>Fix it up</template>
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.block {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 10px 12px;
  margin-bottom: 12px;
  background: var(--surface);
  border: 1px solid var(--line-soft);
  border-radius: var(--radius);
  font-size: 12.5px;
}

.block-head {
  font-weight: 600;
  font-size: 12.5px;
  margin-bottom: 6px;
}

.tag {
  flex: none;
  margin-left: auto;
  padding: 0 7px;
  border-radius: var(--radius-sm);
  font-size: 10.5px;
  font-weight: 600;
  color: var(--amber-soft);
  background: var(--warning-bg);
}

.hunks {
  list-style: none;
  margin: 0 0 10px;
  padding: 0;
  max-height: 300px;
  overflow: auto;
  border: 1px solid var(--line-soft);
  border-radius: var(--radius);
}

.hunks li {
  padding: 6px 10px;
  border-top: 1px solid var(--line-soft);
  font-size: 12px;
}

.hunks li:first-child {
  border-top: none;
}

.hunks li.stays {
  opacity: 0.75;
}

.hunk-head {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}

.arrow {
  color: var(--text-faint);
}

.preview {
  margin: 4px 0 0;
  font-family: var(--mono);
  font-size: 11px;
  white-space: pre-wrap;
  word-break: break-all;
  color: var(--text-dim);
}

.why {
  margin-top: 2px;
}

.field {
  display: block;
}

.label {
  display: block;
  margin-bottom: 5px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-dim);
}

textarea {
  width: 100%;
  display: block;
  resize: vertical;
}

.check {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-top: 12px;
  font-size: 12.5px;
}

.block .truncate {
  min-width: 0;
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
  padding: 10px 12px;
  border-radius: var(--radius);
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
