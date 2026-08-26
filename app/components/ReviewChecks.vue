<script setup lang="ts">
import { computed } from 'vue'
import { ExternalLink, RotateCw } from 'lucide-vue-next'
import Spinner from './Spinner.vue'
import { useReview } from '~/composables/useReview'
import { useForge } from '~/composables/useForge'
import { checkLook } from '~/composables/reviewLook'

/**
 * What ran against this branch, and how it went.
 *
 * The forges answer with wildly different shapes — GitHub's check runs and
 * legacy statuses, GitLab's pipeline jobs — and all a reader wants is a list
 * of names with a mark beside each and a way through to the log.
 */
const review = useReview()
const forge = useForge()
const store = review.store

const checks = computed(() => store.status?.checks ?? [])

/** Failures first: the list exists to be acted on, not admired. */
const ORDER = ['failure', 'pending', 'success', 'cancelled', 'skipped']
const sorted = computed(() =>
  [...checks.value].sort((a, b) => ORDER.indexOf(a.state) - ORDER.indexOf(b.state))
)

const roll = computed(() => checkLook(store.status?.checks_state ?? 'none'))

function refresh() {
  const number = store.current?.number
  if (number) void review.loadStatus(number)
}
</script>

<template>
  <div class="checks-page" data-testid="checks-page">
    <header class="head" :class="roll.tone">
      <component :is="roll.icon" :size="15" />
      <span class="what">
        <template v-if="!checks.length">Nothing ran against this branch</template>
        <template v-else>
          {{ checks.length }} {{ checks.length === 1 ? 'check' : 'checks' }} · {{ roll.label }}
        </template>
      </span>
      <span class="grow" />
      <button class="btn" title="Ask the forge again" @click="refresh">
        <Spinner v-if="store.loadingStatus" :size="12" />
        <RotateCw v-else :size="13" />
      </button>
    </header>

    <ul v-if="sorted.length" class="list">
      <li v-for="check in sorted" :key="check.name + check.url" class="check" data-testid="check-row">
        <span class="mark" :class="checkLook(check.state).tone">
          <component :is="checkLook(check.state).icon" :size="13" />
        </span>
        <span class="name truncate">{{ check.name }}</span>
        <span class="state" :class="checkLook(check.state).tone">{{ checkLook(check.state).label }}</span>
        <span class="detail faint truncate">{{ check.description }}</span>
        <button
          v-if="check.url"
          class="open"
          title="Read the run on the forge"
          @click="forge.open(check.url)"
        >
          <ExternalLink :size="12" />
        </button>
      </li>
    </ul>

    <p v-else class="none faint">
      No pipeline, no workflow, no status: nothing has been run against this branch.
    </p>
  </div>
</template>

<style scoped>
.checks-page {
  display: flex;
  flex-direction: column;
  width: 100%;
  max-width: 900px;
  margin: 0 auto;
  padding: 14px 22px 48px;
}

.head {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 10px 13px;
  border: 1px solid var(--line-soft);
  border-left-width: 3px;
  border-radius: 8px;
  background: var(--bg-panel);
  font-size: 12.5px;
  color: var(--text-dim);
}

.head.good {
  border-left-color: var(--green);
  color: var(--green-soft);
}

.head.bad {
  border-left-color: var(--red);
  color: var(--red-soft);
}

.head.wait {
  border-left-color: var(--amber);
  color: var(--amber-soft);
}

.head.none {
  border-left-color: var(--line);
}

.grow {
  flex: 1;
}

.list {
  display: flex;
  flex-direction: column;
  margin: 10px 0 0;
  padding: 0;
  list-style: none;
  border: 1px solid var(--line-soft);
  border-radius: 8px;
  overflow: hidden;
}

.check {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  font-size: 12px;
  background: var(--bg-panel);
  min-width: 0;
}

.check + .check {
  border-top: 1px solid var(--line-soft);
}

.check:hover {
  background: var(--bg-hover);
}

.mark {
  display: inline-flex;
  flex: none;
}

.mark.good,
.state.good {
  color: var(--green);
}

.mark.bad,
.state.bad {
  color: var(--red);
}

.mark.wait,
.state.wait {
  color: var(--amber);
}

.mark.none,
.state.none {
  color: var(--text-faint);
}

.name {
  min-width: 0;
  color: var(--text);
}

.state {
  flex: none;
  font-size: 10.5px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.detail {
  flex: 1;
  min-width: 0;
  text-align: right;
  font-size: 11px;
}

.open {
  flex: none;
  padding: 3px;
  border-radius: 4px;
  color: var(--text-faint);
}

.open:hover {
  color: var(--accent);
  background: var(--bg-hover);
}

.none {
  margin: 12px 0 0;
  font-size: 12px;
}
</style>
