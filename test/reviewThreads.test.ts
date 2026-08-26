import { describe, expect, it } from 'vitest'
import { foldThreads, threadKey } from '../app/composables/reviewThreads'
import type { RComment } from '../app/composables/useReview'

const who = { login: 'arno', name: 'Arno', avatar: null }

function comment(over: Partial<RComment>): RComment {
  return {
    id: 1,
    author: who,
    body: '',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    kind: 'issue',
    path: null,
    line: null,
    side: null,
    reply_to: null,
    ...over
  }
}

describe('foldThreads', () => {
  it('keeps the conversation and the diff apart', () => {
    const { talk, byLine } = foldThreads([
      comment({ id: 1, kind: 'issue', body: 'talk' }),
      comment({ id: 2, kind: 'diff', path: 'a.ts', line: 7, side: 'new', body: 'here' })
    ])
    expect(talk.map((thread) => thread.root.body)).toEqual(['talk'])
    expect(byLine.get(threadKey('a.ts', 'new', 7))![0]!.root.body).toBe('here')
  })

  it('hangs replies under their root, however deep the chain points', () => {
    const { talk } = foldThreads([
      comment({ id: 1, kind: 'issue', body: 'root' }),
      comment({ id: 2, kind: 'issue', body: 'answer', reply_to: 1 }),
      // A reply aimed at another reply still belongs to the conversation it
      // opened, which is what a thread is.
      comment({ id: 3, kind: 'issue', body: 'answer to the answer', reply_to: 2 })
    ])
    expect(talk).toHaveLength(1)
    expect(talk[0]!.replies.map((one) => one.body)).toEqual(['answer', 'answer to the answer'])
  })

  it('keeps several threads standing on one line in arrival order', () => {
    const { byLine } = foldThreads([
      comment({ id: 1, kind: 'diff', path: 'a.ts', line: 9, side: 'new' }),
      comment({ id: 2, kind: 'diff', path: 'a.ts', line: 9, side: 'new' })
    ])
    expect(byLine.get(threadKey('a.ts', 'new', 9))!).toHaveLength(2)
  })

  it('files an old-side thread apart from a new-side one on the same number', () => {
    const { byLine } = foldThreads([
      comment({ id: 1, kind: 'diff', path: 'a.ts', line: 5, side: 'old' }),
      comment({ id: 2, kind: 'diff', path: 'a.ts', line: 5, side: 'new' })
    ])
    expect(byLine.get(threadKey('a.ts', 'old', 5))!).toHaveLength(1)
    expect(byLine.get(threadKey('a.ts', 'new', 5))!).toHaveLength(1)
  })

  it('drops a reply whose root never arrived rather than losing it silently elsewhere', () => {
    const { talk, byLine } = foldThreads([
      comment({ id: 9, kind: 'issue', body: 'orphan answer', reply_to: 404 })
    ])
    expect(talk).toHaveLength(0)
    expect(byLine.size).toBe(0)
  })

  it('treats a diff remark without a line as conversation rather than nowhere', () => {
    // The forge said which file but not which line; the remark is still worth
    // reading, and the conversation is the one place left to stand.
    const { talk, byLine } = foldThreads([
      comment({ id: 1, kind: 'diff', path: 'a.ts', line: null, side: 'new' })
    ])
    expect(talk.map((thread) => thread.root.path)).toEqual(['a.ts'])
    expect(byLine.size).toBe(0)
  })
})
