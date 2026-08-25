<script setup lang="ts">
import { computed } from 'vue'
import { WIP, useGit } from '~/composables/useGit'

const git = useGit()
const store = git.store

/** The working tree and a commit are different enough to be different panels. */
const showWorking = computed(() => store.selected === WIP)
</script>

<template>
  <aside class="panel">
    <WorkingChanges v-if="showWorking" />
    <CommitDetails v-else />
  </aside>
</template>

<style scoped>
.panel {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr);
  min-height: 0;
  background: var(--bg-panel);
  border-left: 1px solid var(--line);
  overflow: hidden;
}
</style>
