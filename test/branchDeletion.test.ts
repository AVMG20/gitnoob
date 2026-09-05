import { describe, expect, it } from 'vitest'
import { localVerdict, needsForce, remoteVerdict } from '../app/composables/useBranchDeletion'
import type { BranchDeletion } from '../app/composables/useGit'

/** A preview with nothing at stake, for a case to change only what it is about. */
function preview(over: Partial<BranchDeletion> = {}): BranchDeletion {
  return {
    name: 'feature',
    is_head: false,
    merged: true,
    head: 'main',
    also_on: [],
    against: 'main',
    trunk_holds: true,
    only_here: 0,
    upstream: null,
    unpushed: 0,
    remote: null,
    other_remotes: [],
    ...over
  }
}

describe('deleting the branch here', () => {
  it('calls a merged branch safe, in a handful of words', () => {
    const verdict = localVerdict(preview())
    expect(verdict.tone).toBe('safe')
    expect(verdict.acknowledge).toBe(false)
    expect(verdict.headline).toContain('main')
    // The title already says which branch this is about.
    expect(verdict.headline).not.toContain('feature')
    expect(verdict.detail).toBeUndefined()
  })

  // The old dialog took "ahead of its upstream" as loss on its own, and warned
  // that commits already sitting on main would be collected.
  it('does not warn about unpushed commits the main branch already has', () => {
    const verdict = localVerdict(
      preview({ trunk_holds: true, upstream: 'origin/feature', unpushed: 3, only_here: 0 })
    )
    expect(verdict.tone).toBe('safe')
    expect(verdict.acknowledge).toBe(false)
  })

  // It also said "it has no upstream holding a copy" whenever nothing was
  // ahead — including when the upstream held every commit.
  it('says the remote keeps a copy when the branch has not landed but is fully pushed', () => {
    const verdict = localVerdict(
      preview({ trunk_holds: false, upstream: 'origin/feature', unpushed: 0, only_here: 2 })
    )
    expect(verdict.tone).toBe('careful')
    expect(verdict.acknowledge).toBe(false)
    // The remote, not the long remote-tracking name.
    expect(verdict.headline).toContain('origin')
    expect(verdict.headline).not.toContain('origin/feature')
  })

  // And "reachable from nothing" was said about branches another local branch
  // could reach perfectly well.
  it('names the other local branch holding the work', () => {
    const verdict = localVerdict(
      preview({ trunk_holds: false, also_on: ['develop'], only_here: 2 })
    )
    expect(verdict.acknowledge).toBe(false)
    expect(verdict.headline).toContain('develop')
  })

  // The case that started this: a branch that has only reached `staging` was
  // called safe to delete, and `staging` gets reset most weeks.
  it('does not call a branch safe just because a branch that resets holds it', () => {
    const verdict = localVerdict(
      preview({ trunk_holds: false, against: 'main', also_on: ['staging'], only_here: 2 })
    )
    expect(verdict.tone).toBe('careful')
    expect(verdict.headline).toContain('main')
    expect(verdict.detail).toContain('reset')
  })

  it('lists several holders readably', () => {
    const verdict = localVerdict(
      preview({ trunk_holds: false, also_on: ['develop', 'release', 'staging'], only_here: 1 })
    )
    expect(verdict.headline).toContain('develop, release and staging')
  })

  // Landing on the trunk is the question, not landing on wherever you stand.
  it('measures against the main branch rather than the one checked out', () => {
    const verdict = localVerdict(
      preview({ trunk_holds: true, against: 'main', head: 'some-other-branch' })
    )
    expect(verdict.tone).toBe('safe')
    expect(verdict.headline).toContain('main')
    expect(verdict.headline).not.toContain('some-other-branch')
  })

  it('warns, counts and asks for a tick when the commits are only here', () => {
    const verdict = localVerdict(
      preview({ trunk_holds: false, upstream: 'origin/feature', unpushed: 2, only_here: 2 })
    )
    expect(verdict.tone).toBe('danger')
    expect(verdict.acknowledge).toBe(true)
    expect(verdict.headline).toContain('2 commits')
    // A beginner deserves to know the local delete is not quite the end.
    expect(verdict.detail).toContain('reflog')
  })

  it('says so plainly when there is no remote copy at all', () => {
    const verdict = localVerdict(preview({ trunk_holds: false, upstream: null, only_here: 1 }))
    expect(verdict.tone).toBe('danger')
    expect(verdict.headline).toContain('1 commit')
    expect(verdict.detail).toContain('No remote copy')
  })

  // No trunk and no branch under HEAD: nothing to name but the commit itself.
  it('speaks of the commit rather than a branch on a detached HEAD', () => {
    expect(localVerdict(preview({ head: null, against: null })).headline).toContain(
      'the commit you are on'
    )
  })

  it('forces only what git would refuse', () => {
    expect(needsForce(preview({ merged: true }))).toBe(false)
    expect(needsForce(preview({ merged: false }))).toBe(true)
  })
})

describe('deleting the copy on the remote', () => {
  it('has nothing to say when there is no remote copy', () => {
    expect(remoteVerdict(preview())).toBeNull()
  })

  it('is calm and brief when the remote holds nothing new', () => {
    const verdict = remoteVerdict(
      preview({ remote: { name: 'origin/feature', remote: 'origin', unmerged: 0 } })
    )
    expect(verdict?.tone).toBe('careful')
    expect(verdict?.acknowledge).toBe(false)
    expect(verdict?.headline).toContain('origin')
    // Nothing is at stake, so nothing beyond the line itself.
    expect(verdict?.detail).toBeUndefined()
    expect(verdict?.headline).not.toContain('feature')
  })

  // The case that used to pass as "loses nothing" with a one-click delete:
  // merged locally, nothing unpushed, and commits on the remote regardless.
  it('warns about commits that exist only on the remote even when the branch is merged here', () => {
    const verdict = remoteVerdict(
      preview({
        trunk_holds: true,
        unpushed: 0,
        only_here: 0,
        remote: { name: 'origin/feature', remote: 'origin', unmerged: 2 }
      })
    )
    expect(verdict?.tone).toBe('danger')
    expect(verdict?.acknowledge).toBe(true)
    expect(verdict?.headline).toContain('2 commits')
    expect(verdict?.detail).toContain('reflog')
  })
})
