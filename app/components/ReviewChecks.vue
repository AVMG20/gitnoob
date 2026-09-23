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
          :title="`Read the run on ${forge.forgeName.value}`"
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
  padding: 20px 24px 56px;
}

/* How the checks add up, as a tinted banner in the colour of the verdict. */
.head {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
  border-radius: var(--radius);
  background: var(--surface);
  font-size: 13px;
  font-weight: 550;
  color: var(--text-dim);
}

.head.good {
  background: var(--success-bg);
  color: var(--success-soft);
}

.head.bad {
  background: var(--danger-bg);
  color: var(--danger-soft);
}

.head.wait {
  background: var(--warning-bg);
  color: var(--warning-soft);
}

.grow {
  flex: 1;
}

.list {
  display: flex;
  flex-direction: column;
  margin: 12px 0 0;
  padding: 0;
  list-style: none;
  border-radius: var(--radius);
  box-shadow: var(--shadow-card);
  overflow: hidden;
}

.check {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  font-size: 12.5px;
  background: var(--bg);
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
  color: var(--success);
}

.mark.bad,
.state.bad {
  color: var(--danger);
}

.mark.wait,
.state.wait {
  color: var(--warning);
}

.mark.none,
.state.none {
  color: var(--text-faint);
}

.name {
  min-width: 0;
  color: var(--text);
}

/* The state as a small tinted pill beside the name. */
.state {
  flex: none;
  padding: 1px 8px;
  border-radius: var(--radius-pill);
  background: color-mix(in srgb, currentColor 12%, transparent);
  font-size: 10.5px;
  font-weight: 600;
  text-transform: capitalize;
}

.detail {
  flex: 1;
  min-width: 0;
  text-align: right;
  font-size: 11.5px;
}

.open {
  flex: none;
  padding: 4px;
  border-radius: var(--radius-pill);
  color: var(--text-faint);
}

.open:hover {
  color: var(--text);
  background: var(--bg-active);
}

.none {
  margin: 14px 0 0;
  font-size: 12.5px;
}
</style>
