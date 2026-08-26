import {
  Check,
  CircleDashed,
  CircleSlash,
  MessageSquare,
  MinusCircle,
  X
} from 'lucide-vue-next'
import type { Component } from 'vue'
import type { ReviewCheck, ReviewStatus, ReviewVerdict } from './useReview'

/**
 * How a review's states are drawn.
 *
 * Checks, verdicts and merge readiness are each a small closed set of words
 * the forge sends, and every place that draws one wants the same icon and the
 * same colour for it. Kept here so the merge box, the sidebar and the checks
 * page cannot drift into three different greens.
 */

export interface Look {
  icon: Component
  /** The class the colour hangs off: `good`, `bad`, `wait`, `none`. */
  tone: 'good' | 'bad' | 'wait' | 'none'
  label: string
}

export function checkLook(state: ReviewCheck['state'] | ReviewStatus['checks_state']): Look {
  switch (state) {
    case 'success':
      return { icon: Check, tone: 'good', label: 'passed' }
    case 'failure':
      return { icon: X, tone: 'bad', label: 'failed' }
    case 'pending':
      return { icon: CircleDashed, tone: 'wait', label: 'running' }
    case 'cancelled':
      return { icon: CircleSlash, tone: 'none', label: 'cancelled' }
    case 'skipped':
      return { icon: MinusCircle, tone: 'none', label: 'skipped' }
    default:
      return { icon: CircleDashed, tone: 'none', label: 'nothing ran' }
  }
}

export function verdictLook(state: ReviewVerdict['state']): Look {
  switch (state) {
    case 'approved':
      return { icon: Check, tone: 'good', label: 'approved' }
    case 'changes_requested':
      return { icon: X, tone: 'bad', label: 'requested changes' }
    case 'dismissed':
      return { icon: MinusCircle, tone: 'none', label: 'dismissed' }
    default:
      return { icon: MessageSquare, tone: 'none', label: 'commented' }
  }
}

/**
 * The one sentence the merge box leads with.
 *
 * Said in the order a reader cares about it: something that stops the merge
 * outright, then something that ought to stop it, then the go-ahead.
 */
export function mergeSummary(
  status: ReviewStatus | null,
  state: string,
  draft: boolean
): { tone: Look['tone']; title: string; detail: string } {
  if (state === 'merged') {
    return { tone: 'good', title: 'Merged', detail: 'This review has landed.' }
  }
  if (state === 'closed') {
    return { tone: 'none', title: 'Closed', detail: 'Closed without merging.' }
  }
  if (draft) {
    return {
      tone: 'wait',
      title: 'Still a draft',
      detail: 'Mark it ready when it is meant to be read.'
    }
  }
  // Green is a claim, and claiming a clear run before the forge has answered is
  // the one thing this box must not do.
  if (!status) {
    return {
      tone: 'none',
      title: 'Merge when you are ready',
      detail: 'The forge has not said how this one stands.'
    }
  }
  if (status?.conflicts) {
    return {
      tone: 'bad',
      title: 'Conflicts with the target branch',
      detail: 'Pull the target into the branch and settle them first.'
    }
  }
  if (status?.checks_state === 'failure') {
    return { tone: 'bad', title: 'Checks failed', detail: 'Something that ran here did not pass.' }
  }
  if (status?.checks_state === 'pending') {
    return { tone: 'wait', title: 'Checks are still running', detail: 'It can still be merged.' }
  }
  if (status && status.approvals_required > status.approvals) {
    const left = status.approvals_required - status.approvals
    return {
      tone: 'wait',
      title: `${left} more approval${left === 1 ? '' : 's'} needed`,
      detail: `${status.approvals} of ${status.approvals_required} given.`
    }
  }
  if (status?.mergeable === false) {
    return {
      tone: 'bad',
      title: 'The forge will not merge this yet',
      detail: status.merge_status ? `It says: ${status.merge_status.replace(/_/g, ' ')}.` : ''
    }
  }
  return { tone: 'good', title: 'Ready to merge', detail: 'Nothing is standing in the way.' }
}
