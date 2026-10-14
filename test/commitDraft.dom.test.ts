// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { moveDraft, resetDrafts, useCommitDraft } from '~/composables/useCommitDraft'

const where = ref({ repo: '/repo' as string | null, branch: 'main' as string | null })

beforeEach(() => {
  localStorage.clear()
  resetDrafts()
  where.value = { repo: '/repo', branch: 'main' }
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('a commit draft', () => {
  it('belongs to its repository and branch', () => {
    const draft = useCommitDraft(() => where.value)
    draft.message.value = 'On main'
    draft.amend.value = true

    where.value = { repo: '/repo', branch: 'feature' }
    expect(draft.message.value).toBe('')
    expect(draft.amend.value).toBe(false)
    draft.message.value = 'On feature'

    where.value = { repo: '/other', branch: 'main' }
    expect(draft.message.value).toBe('')

    where.value = { repo: '/repo', branch: 'main' }
    expect(draft.message.value).toBe('On main')
    expect(draft.amend.value).toBe(true)
  })

  it('comes back after a restart, from storage', () => {
    useCommitDraft(() => where.value).message.value = 'Kept'
    resetDrafts()
    expect(useCommitDraft(() => where.value).message.value).toBe('Kept')
  })

  it('is gone once cleared', () => {
    const draft = useCommitDraft(() => where.value)
    draft.message.value = 'Committed'
    draft.clear()
    expect(draft.message.value).toBe('')
    resetDrafts()
    expect(useCommitDraft(() => where.value).message.value).toBe('')
  })

  it('works without storage at all', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('denied')
    })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('full')
    })
    resetDrafts()
    const draft = useCommitDraft(() => where.value)
    draft.message.value = 'Still held'
    expect(draft.message.value).toBe('Still held')
  })

  it('ignores storage it cannot read', () => {
    localStorage.setItem('gitnoob.commit-drafts', '{not json')
    resetDrafts()
    expect(useCommitDraft(() => where.value).message.value).toBe('')
  })

  it('never comes back after a restart ticked to amend', () => {
    const draft = useCommitDraft(() => where.value)
    draft.message.value = 'What I typed'
    draft.amend.value = true
    draft.stashed.value = 'What I typed'
    draft.message.value = "HEAD's own message"
    resetDrafts()
    const again = useCommitDraft(() => where.value)
    expect(again.amend.value).toBe(false)
    expect(again.message.value).toBe('What I typed')
  })

  it('reads a draft an older version stored with the amend tick, without it', () => {
    localStorage.setItem(
      'gitnoob.commit-drafts',
      JSON.stringify({
        '/repo\u0000main': {
          message: 'Old',
          amend: true,
          stashed: 'x',
          pushed: true,
          at: Date.now()
        }
      })
    )
    resetDrafts()
    const draft = useCommitDraft(() => where.value)
    expect(draft.message.value).toBe('Old')
    expect(draft.amend.value).toBe(false)
    expect(draft.stashed.value).toBe(null)
    expect(draft.pushed.value).toBe(false)
  })

  it('follows its branch to a new name', () => {
    const draft = useCommitDraft(() => where.value)
    draft.message.value = 'Half written'
    moveDraft('/repo', 'main', 'trunk')
    expect(draft.message.value).toBe('')
    where.value = { repo: '/repo', branch: 'trunk' }
    expect(draft.message.value).toBe('Half written')
  })
})
