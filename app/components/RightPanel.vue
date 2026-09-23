<script setup lang="ts">
import { computed } from 'vue'
import { FilePen, TriangleAlert } from 'lucide-vue-next'
import { WIP, useGit } from '~/composables/useGit'

const git = useGit()
const store = git.store

/** The working tree and a commit are different enough to be different panels. */
const showWorking = computed(() => store.selected === WIP)

const dirty = computed(
  () => (store.status?.staged.length ?? 0) + (store.status?.unstaged.length ?? 0)
)
const conflicts = computed(() => store.status?.conflicted.length ?? 0)

/**
 * Work in progress, said while you are reading something else.
 *
 * Reading a commit fills this panel with that commit, and from then on nothing
 * on screen says you have uncommitted work at all — the one row that did is at
 * the top of a list you have since scrolled. Files you meant to commit are then
 * only ever remembered, which is how they get left behind. So the count follows
 * you into the panel and carries the way back, rather than asking you to find
 * the working-tree row again.
 */
const pending = computed(() => (showWorking.value ? 0 : conflicts.value || dirty.value))
</script>

<template>
  <aside class="panel">
    <button
      v-if="pending"
      class="pending"
      :class="{ bad: conflicts }"
      :title="
        conflicts
          ? 'Resolve the conflicts before committing'
          : 'Show what is waiting in your working tree'
      "
      @click="git.select(WIP)"
    >
      <component :is="conflicts ? TriangleAlert : FilePen" :size="13" class="glyph" />
      <span class="truncate">
        {{ pending }}
        <template v-if="conflicts">{{ pending === 1 ? 'conflict' : 'conflicts' }}</template>
        <template v-else>{{ pending === 1 ? 'change' : 'changes' }} in your working tree</template>
      </span>
      <span class="go">View</span>
    </button>
    <WorkingChanges v-if="showWorking" />
    <CommitDetails v-else />
  </aside>
</template>

<style scoped>
/* A flex column rather than a grid with named row tracks. The banner comes and
   goes, so the number of children varies, and a grid told to expect two rows
   drops a lone panel into the first one — the row sized to its content — which
   is what would leave the panel collapsed to the height of its text whenever
   the working tree was clean. Flex stacks whatever is there. */
.panel {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

/* Whichever panel is showing takes the rest of the height, and scrolls under a
   banner that stays put rather than pushing it out of reach. */
.panel > :last-child {
  flex: 1;
  min-height: 0;
}

.pending {
  flex: none;
  display: flex;
  align-items: center;
  gap: 7px;
  width: 100%;
  min-height: 36px;
  padding: 5px 8px 5px 12px;
  border-bottom: 1px solid var(--line);
  font-size: 12px;
  font-weight: 600;
  text-align: left;
  /* The accent, not the amber it started in. Having uncommitted work is the
     ordinary state of a working day, and a warning colour said something was
     wrong with it — which also spent the one colour that means "look at this"
     on the case where nothing is the matter. Amber is left for the conflicts
     below, where it is earned. */
  color: var(--accent-soft);
  background: var(--primary-bg);
}

.pending:hover {
  background: color-mix(in srgb, var(--accent) 18%, var(--bg));
}

.pending.bad {
  color: var(--red-soft);
  background: color-mix(in srgb, var(--red) 16%, transparent);
}

.pending.bad:hover {
  background: color-mix(in srgb, var(--red) 24%, transparent);
}

.pending .glyph {
  flex: none;
}

/* Pushed to the far end and bordered: the banner is a sentence and this is the
   button at the end of it, not another word in the sentence. */
.go {
  flex: none;
  margin-left: auto;
  padding: 1px 9px;
  border-radius: var(--radius-sm);
  font-size: 11.5px;
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--line);
}

.pending:hover .go {
  border-color: currentColor;
}
</style>
