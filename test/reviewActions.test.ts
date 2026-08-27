import { flushPromises } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useReview } from '../app/composables/useReview'
import { useForge, type Review } from '../app/composables/useForge'

/**
 * The actions that talk to the forge directly, against a mock that can be
 * told to fail. What matters here is what the store looks like afterwards,
 * not what got sent — the round trip itself is covered where it is mounted.
 */

const who = { login: 'kai', name: 'Kai Moens', avatar: null }
const now = '2026-08-20T10:00:00Z'

const CURRENT: Review = {
  number: 7,
  title: 'A review',
  author: 'kai',
  state: 'open',
  draft: false,
  source_branch: 'feature/x',
  target_branch: 'main',
  url: 'https://example.test/pr/7',
  updated_at: now,
  is_current: false,
  head_sha: 'a'.repeat(40),
  source: null,
  warning: null
}

let reviewState = 'open'
let mergeFails = false
let sendFails = false
let filesFail = false
let commitsFail = false

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    switch (cmd) {
      case 'forge_review_detail':
        return {
          number: 7,
          title: 'A review',
          body: '',
          state: reviewState,
          draft: false,
          author: who,
          assignees: [],
          reviewers: [],
          labels: [],
          milestone: null,
          source_branch: 'feature/x',
          target_branch: 'main',
          url: CURRENT.url,
          created_at: now,
          updated_at: now,
          comments: 0,
          merge_status: null,
          base_sha: 'b'.repeat(40),
          head_sha: 'a'.repeat(40),
          start_sha: 'c'.repeat(40)
        }
      case 'forge_reviews':
        return []
      case 'forge_review_status':
        return null
      case 'forge_review_comments':
        return []
      case 'forge_review_files':
        if (filesFail) throw new Error('503 Service Unavailable')
        return []
      case 'forge_review_commits':
        if (commitsFail) throw new Error('503 Service Unavailable')
        return []
      case 'forge_merge_review':
        if (mergeFails) throw new Error('protected branch: review required')
        reviewState = 'merged'
        return 'Merged'
      case 'forge_add_diff_comment':
        if (sendFails) throw new Error('502 Bad Gateway')
        return null
      default:
        return null
    }
  })
}))

const review = useReview()
const forge = useForge()

beforeEach(() => {
  vi.clearAllMocks()
  reviewState = 'open'
  mergeFails = false
  sendFails = false
  filesFail = false
  commitsFail = false

  forge.store.status = {
    kind: 'github',
    host: 'github.com',
    has_token: true,
    user: 'kai',
    slug: { host: 'github.com', owner: 'me', name: 'repo' },
    error: null
  }
  forge.store.details = {}
  forge.store.detailsFor = null
  forge.store.reviews = []
  forge.store.error = null

  review.close()
  Object.assign(review.store, {
    current: CURRENT,
    detail: {
      number: 7,
      title: 'A review',
      body: '',
      state: 'open',
      draft: false,
      author: who,
      assignees: [],
      reviewers: [],
      labels: [],
      milestone: null,
      source_branch: 'feature/x',
      target_branch: 'main',
      url: CURRENT.url,
      created_at: now,
      updated_at: now,
      comments: 0,
      merge_status: null,
      base_sha: 'b'.repeat(40),
      head_sha: 'a'.repeat(40),
      start_sha: 'c'.repeat(40)
    },
    comments: [],
    files: [],
    commits: [],
    status: null,
    draft: null,
    pending: [],
    sending: false,
    acting: null,
    drafts: { talk: '', lines: {} }
  })
})

describe('merge', () => {
  it('marks the review merged once the forge confirms it', async () => {
    const note = await review.merge()
    expect(note).toBe('Merged')
    expect(review.store.detail?.state).toBe('merged')
  })

  it('leaves the review exactly as it stood when the merge fails', async () => {
    mergeFails = true
    const note = await review.merge()
    expect(note).toBeNull()
    // Not hardcoded back to 'open' either — whatever it was before the
    // attempt is what a refusal leaves it as.
    expect(review.store.detail?.state).toBe('open')
    expect(forge.store.error).toContain('protected branch')
  })

  it('rolls a draft back to draft rather than to open', async () => {
    mergeFails = true
    reviewState = 'draft'
    review.store.detail!.state = 'draft'
    await review.merge()
    expect(review.store.detail?.state).toBe('draft')
  })
})

describe('sendDraft', () => {
  beforeEach(() => {
    review.store.draft = { path: 'a.ts', line: 3, side: 'new' }
  })

  it('closes the composer once the remark lands', async () => {
    const ok = await review.sendDraft('looks good')
    expect(ok).toBe(true)
    expect(review.store.draft).toBeNull()
  })

  it('keeps the composer open so a failed send is not lost off screen', async () => {
    sendFails = true
    const ok = await review.sendDraft('looks good')
    expect(ok).toBe(false)
    expect(review.store.draft).toEqual({ path: 'a.ts', line: 3, side: 'new' })
    expect(forge.store.error).toContain('502')
  })
})

describe('loading files and commits', () => {
  it('tells a load failure apart from a genuinely empty diff', async () => {
    filesFail = true
    commitsFail = true

    review.show(CURRENT)
    await flushPromises()

    expect(review.store.files).toEqual([])
    expect(review.store.commits).toEqual([])
    expect(review.store.filesError).toContain('503')
    expect(review.store.commitsError).toContain('503')
  })

  it('leaves no error once a load actually succeeds', async () => {
    review.show(CURRENT)
    await flushPromises()

    expect(review.store.filesError).toBeNull()
    expect(review.store.commitsError).toBeNull()
  })
})
