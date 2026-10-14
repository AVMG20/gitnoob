import { describe, expect, it } from 'vitest'
import {
  failedRemotes,
  leadWithForce,
  protectedPushWarning,
  tickedAtFirst,
  tidyKey,
  tidyReason,
  trunkLocalName
} from '~/composables/useSync'
import { localVerdict } from '~/composables/useBranchDeletion'
import type { BranchDeletion, HistoryEntry, LocalBranch, PushBlock, Tidy } from '~/composables/useGit'

const step = (kind: string, branch: string | null): HistoryEntry => ({
  id: 1,
  label: kind,
  kind,
  branch,
  before: 'a',
  after: 'b',
  mode: 'hard',
  destructive: true,
  at: 0
})

const block = (over: Partial<PushBlock> = {}): PushBlock => ({
  remote: 'origin',
  branch: 'feature',
  upstream: 'origin/feature',
  message: 'rejected',
  ...over
})

const local = (name: string, upstream: string | null = null): LocalBranch => ({
  name,
  oid: name,
  is_head: false,
  upstream,
  ahead: 0,
  behind: 0
})

const tidy = (over: Partial<Tidy>): Tidy => ({
  name: 'x',
  oid: '1111111',
  reason: 'merged',
  into: 'origin/main',
  gone: null,
  proven: true,
  ...over
})

describe('which way a refused push leads', () => {
  it('leads with a pull for somebody else’s commits', () => {
    expect(leadWithForce(block())).toBe(false)
  })

  it('leads with force when the backend found only rewritten copies', () => {
    expect(leadWithForce(block({ rewritten: true }))).toBe(true)
  })

  it('does not guess from the undo history', () => {
    // A rebase here says nothing about what a colleague pushed meanwhile; the
    // backend's measurement is the only thing that puts force first.
    expect(leadWithForce(block({ rewritten: false }))).toBe(false)
    expect(leadWithForce(null)).toBe(false)
  })
})

describe('the protected-branch nudge', () => {
  const trunk = { name: 'main', chosen: false }

  it('asks only when switched on and only for the trunk', async () => {
    const locals = [local('main', 'origin/main'), local('feature')]
    expect(await protectedPushWarning('main', trunk, locals, false)).toBeNull()
    expect(await protectedPushWarning('feature', trunk, locals, true)).toBeNull()
    expect(await protectedPushWarning('main', trunk, locals, true)).toMatch(/main branch/)
    expect(await protectedPushWarning('release', trunk, locals, true, async () => true)).toMatch(/protected on the forge/)
    expect(await protectedPushWarning('release', trunk, locals, true, async () => null)).toBeNull()
    expect(await protectedPushWarning('release', trunk, locals, false, async () => true)).toBeNull()
  })

  it('knows a remote-only trunk by its local name', () => {
    expect(trunkLocalName({ name: 'origin/develop', chosen: false }, [])).toBe('develop')
    expect(trunkLocalName({ name: 'main', chosen: false }, [local('main')])).toBe('main')
    expect(trunkLocalName({ name: null, chosen: false }, [])).toBeNull()
  })
})

describe('a fetch that failed on a remote', () => {
  it('names each remote --all could not reach', () => {
    const text = "fatal: repository 'https://x/alice/r.git/' not found\nerror: could not fetch alice\nerror: could not fetch bob-2"
    expect(failedRemotes(text)).toEqual(['alice', 'bob-2'])
  })

  it('names the one remote that was asked for', () => {
    expect(failedRemotes('fatal: could not read from remote repository', 'carol')).toEqual(['carol'])
    expect(failedRemotes('', 'carol')).toEqual([])
  })
})

describe('the tidy list', () => {
  it('says why each branch is on it', () => {
    expect(tidyReason(tidy({ reason: 'merged' }))).toBe('merged into origin/main')
    expect(tidyReason(tidy({ reason: 'squash' }))).toBe('squash-merged into origin/main')
    expect(tidyReason(tidy({ reason: 'gone', gone: 'origin/x', into: null, proven: false }))).toMatch(
      /origin\/x was deleted/
    )
  })

  it('ticks only what is proven to have landed', () => {
    const list = [tidy({ name: 'a' }), tidy({ name: 'b', reason: 'gone', proven: false })]
    expect([...tickedAtFirst(list)]).toEqual(['a'])
  })

  it('tells one list from another by name and tip', () => {
    const one = [tidy({ name: 'a' }), tidy({ name: 'b' })]
    expect(tidyKey(one)).toBe(tidyKey([...one].reverse()))
    expect(tidyKey(one)).not.toBe(tidyKey([tidy({ name: 'a', oid: '2' }), tidy({ name: 'b' })]))
  })
})

describe('deleting a branch a forge squashed in', () => {
  const found = (over: Partial<BranchDeletion>): BranchDeletion => ({
    name: 'feature',
    is_head: false,
    merged: false,
    head: 'main',
    also_on: [],
    against: 'origin/main',
    trunk_holds: true,
    only_here: 0,
    upstream: null,
    unpushed: 0,
    remote: null,
    other_remotes: [],
    ...over
  })

  it('reads as safe, and says how it landed', () => {
    const verdict = localVerdict(found({ landed: 'squash' }))
    expect(verdict.tone).toBe('safe')
    expect(verdict.acknowledge).toBe(false)
    expect(verdict.headline).toBe('Squash-merged into origin/main')
    expect(localVerdict(found({ landed: 'merged' })).headline).toBe('Merged into origin/main')
  })
})
