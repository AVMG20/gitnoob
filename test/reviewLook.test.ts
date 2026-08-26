import { describe, expect, it } from 'vitest'
import { checkLook, mergeSummary, verdictLook } from '~/composables/reviewLook'
import type { ReviewStatus } from '~/composables/useReview'

/**
 * The words and colours a review's state is drawn in.
 *
 * All of it is a lookup, which is exactly why it is worth pinning: the merge
 * box, the sidebar and the checks page all read from here, and the order the
 * blockers are said in is the whole point of the summary.
 */

const clean: ReviewStatus = {
  checks: [],
  checks_state: 'none',
  verdicts: [],
  approvals: 0,
  approvals_required: 0,
  mergeable: true,
  merge_status: 'clean',
  conflicts: false
}

describe('checkLook', () => {
  it('gives each state its own tone', () => {
    expect(checkLook('success').tone).toBe('good')
    expect(checkLook('failure').tone).toBe('bad')
    expect(checkLook('pending').tone).toBe('wait')
    expect(checkLook('skipped').tone).toBe('none')
    expect(checkLook('none').tone).toBe('none')
    expect(checkLook('none').label).toBe('nothing ran')
  })
})

describe('verdictLook', () => {
  it('says what somebody made of the review', () => {
    expect(verdictLook('approved').label).toBe('approved')
    expect(verdictLook('approved').tone).toBe('good')
    expect(verdictLook('changes_requested').tone).toBe('bad')
    expect(verdictLook('commented').tone).toBe('none')
    expect(verdictLook('dismissed').label).toBe('dismissed')
  })
})

describe('mergeSummary', () => {
  it('says a settled review is settled and stops there', () => {
    expect(mergeSummary(clean, 'merged', false).title).toBe('Merged')
    expect(mergeSummary(clean, 'closed', false).title).toBe('Closed')
    expect(mergeSummary(clean, 'closed', false).tone).toBe('none')
  })

  it('leads with whatever actually stops the merge', () => {
    // A conflict outranks a failed check, which outranks a running one.
    const conflicted: ReviewStatus = {
      ...clean,
      conflicts: true,
      checks_state: 'failure',
      mergeable: false
    }
    expect(mergeSummary(conflicted, 'open', false).title).toContain('Conflicts')
    expect(mergeSummary({ ...clean, checks_state: 'failure' }, 'open', false).title).toBe(
      'Checks failed'
    )
    const running = mergeSummary({ ...clean, checks_state: 'pending' }, 'open', false)
    expect(running.tone).toBe('wait')
    expect(running.detail).toContain('can still be merged')
  })

  it('counts the approvals a project insists on', () => {
    const short = mergeSummary(
      { ...clean, approvals: 1, approvals_required: 2 },
      'open',
      false
    )
    expect(short.title).toBe('1 more approval needed')
    expect(short.detail).toBe('1 of 2 given.')

    const enough = mergeSummary(
      { ...clean, approvals: 2, approvals_required: 2 },
      'open',
      false
    )
    expect(enough.title).toBe('Ready to merge')
  })

  it('says a draft is a draft before anything else about it', () => {
    const draft = mergeSummary({ ...clean, checks_state: 'failure' }, 'draft', true)
    expect(draft.title).toBe('Still a draft')
    expect(draft.tone).toBe('wait')
  })

  it('passes on the forge’s own refusal when it has one', () => {
    const blocked = mergeSummary(
      { ...clean, mergeable: false, merge_status: 'not_approved' },
      'open',
      false
    )
    expect(blocked.tone).toBe('bad')
    expect(blocked.detail).toContain('not approved')
  })

  it('says nothing is in the way when nothing is', () => {
    expect(mergeSummary(clean, 'open', false).title).toBe('Ready to merge')
    expect(mergeSummary(clean, 'open', false).tone).toBe('good')
  })

  it('does not call a review clear before the forge has said so', () => {
    const unknown = mergeSummary(null, 'open', false)
    expect(unknown.tone).toBe('none')
    expect(unknown.title).toBe('Merge when you are ready')
    expect(unknown.detail).toContain('has not said')
  })
})
