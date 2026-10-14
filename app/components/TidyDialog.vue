<script setup lang="ts">
import { computed, ref } from 'vue'
import { CircleCheck, TriangleAlert } from 'lucide-vue-next'
import { useGit } from '~/composables/useGit'
import { tickedAtFirst, tidyReason } from '~/composables/useSync'

/**
 * The branches a fetch found finished, each with why, and a tick box each.
 *
 * Only the ones whose work is provably on the trunk start ticked. A branch
 * whose upstream was deleted but whose work is on no trunk is listed too —
 * somebody may have closed the pull request rather than merged it — and goes
 * only when it is ticked by hand. Every deletion is offered back on the notice
 * that follows.
 */
const emit = defineEmits<{ close: [] }>()

const git = useGit()
const store = git.store

const ticked = ref(tickedAtFirst(store.tidy))

function toggle(name: string) {
  const next = new Set(ticked.value)
  if (!next.delete(name)) next.add(name)
  ticked.value = next
}

/** Ticked and still on the list: a refresh may have taken some away. */
const chosen = computed(() => store.tidy.filter((one) => ticked.value.has(one.name)))
const risky = computed(() => chosen.value.filter((one) => !one.proven).length)

async function remove() {
  // With the commit each one was offered at: a branch that moves on after
  // the list was read is refused rather than deleted at its new tip.
  const branches = chosen.value.map((one) => ({ name: one.name, oid: one.oid }))
  if (!branches.length) return
  await git.deleteBranches(branches)
  emit('close')
}
</script>

<template>
  <AppModal title="Tidy away finished branches" :width="500" @close="emit('close')">
    <p class="dim intro">
      Local branches only. Nothing on any remote is touched, and each one can be put back from
      the notice afterwards.
    </p>

    <ul class="list">
      <li v-for="one in store.tidy" :key="one.name" class="item" :class="{ risky: !one.proven }">
        <label>
          <input
            type="checkbox"
            :checked="ticked.has(one.name)"
            @change="toggle(one.name)"
          />
          <component
            :is="one.proven ? CircleCheck : TriangleAlert"
            :size="13"
            class="glyph"
          />
          <span class="mono name">{{ one.name }}</span>
          <span class="why">{{ tidyReason(one) }}</span>
        </label>
      </li>
    </ul>

    <p v-if="!store.tidy.length" class="dim">Nothing left to tidy.</p>
    <p v-if="risky" class="warn">
      {{ risky }} ticked {{ risky === 1 ? 'branch has' : 'branches have' }} work that is on no main
      branch. Undo on the notice brings {{ risky === 1 ? 'it' : 'them' }} back.
    </p>

    <template #footer>
      <button class="btn btn-ghost" @click="emit('close')">Cancel</button>
      <button
        class="btn"
        :class="risky ? 'btn-danger' : 'btn-primary'"
        :disabled="store.busy || !chosen.length"
        @click="remove"
      >
        Delete {{ chosen.length }} {{ chosen.length === 1 ? 'branch' : 'branches' }}
      </button>
    </template>
  </AppModal>
</template>

<style scoped>
.intro {
  margin: 0 0 10px;
  font-size: 12px;
}

.list {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 320px;
  overflow-y: auto;
}

.item label {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 4px 2px;
  font-size: 12.5px;
  cursor: pointer;
}

.glyph {
  flex: none;
  color: var(--green-soft);
}

.risky .glyph {
  color: var(--amber);
}

.name {
  flex: none;
  max-width: 50%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.why {
  min-width: 0;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.warn {
  margin: 10px 0 0;
  font-size: 12px;
  color: var(--red-soft);
}
</style>
