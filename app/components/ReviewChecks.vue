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
  padding: 18px 20px 56px;
}

/* The checks are one bordered box: how they add up as its header strip, in
   the colour of the verdict, and a row for each check under it. */
.head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 42px;
  padding: 4px 8px 4px 14px;
  border: 1px solid var(--line);
  border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  background: var(--surface);
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.head.good > svg {
  color: var(--success);
}

.head.bad > svg {
  color: var(--danger);
}

.head.wait > svg {
  color: var(--warning);
}

.grow {
  flex: 1;
}

.list {
  display: flex;
  flex-direction: column;
  margin: 0;
  padding: 0;
  list-style: none;
  border: 1px solid var(--line);
  border-top: none;
  border-radius: 0 0 var(--radius-lg) var(--radius-lg);
  overflow: hidden;
}

.check {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 32px;
  padding: 4px 10px 4px 14px;
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

/* The state as a small tinted tag beside the name. */
.state {
  flex: none;
  padding: 0 6px;
  border-radius: var(--radius-sm);
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
  border-radius: var(--radius-sm);
  color: var(--text-faint);
}

.open:hover {
  color: var(--text);
  background: var(--bg-hover);
}

/* With nothing to list, the sentence closes the box instead. */
.none {
  margin: 0;
  padding: 12px 14px;
  border: 1px solid var(--line);
  border-top: none;
  border-radius: 0 0 var(--radius-lg) var(--radius-lg);
  font-size: 12.5px;
}
</style>
