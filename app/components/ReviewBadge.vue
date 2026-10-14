<script setup lang="ts">
import { computed } from 'vue'
import { Check, CircleDot, X } from 'lucide-vue-next'
import { useForge, type Review } from '~/composables/useForge'
import { useReview } from '~/composables/useReview'
import { badgeLook } from '~/composables/useRefChips'

/**
 * A branch's open review, as a small mark beside its name.
 *
 * The number, a dot for how its checks are going and a tick or a cross for
 * where the reviewing is. Clicking it opens the review — the one place in the
 * window a branch and its review are one click apart.
 */
const props = defineProps<{ review: Review }>()

const forge = useForge()
const pages = useReview()

const look = computed(() => badgeLook(props.review, forge.sigil.value))
</script>

<template>
  <button
    class="review-badge"
    :class="look.tone"
    :title="`${look.title}\nOpen the review`"
    data-testid="review-badge"
    @click.stop="pages.show(review)"
    @dblclick.stop
  >
    <span class="dot" />
    {{ look.text }}
    <Check v-if="review.review_state === 'approved'" :size="10" :stroke-width="3" class="verdict good" />
    <X v-else-if="review.review_state === 'changes_requested'" :size="10" :stroke-width="3" class="verdict bad" />
    <CircleDot v-else-if="review.draft" :size="10" class="verdict" />
  </button>
</template>

<style scoped>
.review-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: none;
  height: 17px;
  padding: 0 6px;
  border-radius: 9px;
  border: 1px solid var(--line);
  background: var(--surface);
  color: var(--text-dim);
  font-size: 10.5px;
  font-weight: 600;
  line-height: 1;
  cursor: pointer;
}

.review-badge:hover {
  color: var(--text);
  background: var(--bg-hover);
}

.dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-faint);
}

.passing .dot {
  background: var(--success);
}

.failing .dot {
  background: var(--danger);
}

.pending .dot {
  background: var(--warning);
  animation: badge-pulse 1.4s ease-in-out infinite;
}

@keyframes badge-pulse {
  50% {
    opacity: 0.35;
  }
}

.verdict.good {
  color: var(--success);
}

.verdict.bad {
  color: var(--danger);
}
</style>
