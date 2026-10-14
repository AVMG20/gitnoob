import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  changes,
  draftFromCommits,
  issueReference,
  nextDelay,
  offerReviewAfterPush,
  runHomeAct,
  sameProject,
  titleFromBranch
} from '~/composables/useForgeNudges'
import { useForge, type Review } from '~/composables/useForge'
import { useGit } from '~/composables/useGit'
import { useToasts } from '~/composables/useToasts'
import { useConfig } from '~/composables/useConfig'
import { useReview } from '~/composables/useReview'
import { needsMe, type RepoCard } from '~/composables/useHome'
import { badgeLook, reviewForBranch } from '~/composables/useRefChips'
import { aimAt } from '~/composables/useInvoke'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const forge = useForge()
const git = useGit()
const toasts = useToasts()
const config = useConfig()

function review(over: Partial<Review> = {}): Review {
  return {
    number: 12,
    title: 'Fix the login',
    author: 'robin',
    state: 'open',
    draft: false,
    source_branch: 'fix-login',
    target_branch: 'main',
    url: 'https://github.com/acme/app/pull/12',
    updated_at: '',
    is_current: false,
    head_sha: 'a'.repeat(40),
    source: { full_name: 'acme/app', owner: 'acme', ssh_url: '', https_url: '', is_fork: false },
    warning: null,
    checks: 'none',
    review_state: null,
    ...over
  }
}

describe('a new review, filled in from the branch', () => {
  it('reads an issue out of the branch name', () => {
    expect(issueReference('123-fix-login')).toBe('Closes #123')
    expect(issueReference('fix/123-login')).toBe('Closes #123')
    expect(issueReference('123')).toBe('Closes #123')
    // Somebody else's tracker: kept as text, nothing claimed about closing it.
    expect(issueReference('ABC-123-login')).toBe('ABC-123')
    expect(issueReference('feature/ABC-123')).toBe('ABC-123')
    expect(issueReference('fix-login')).toBeNull()
    expect(issueReference('v2-cleanup')).toBeNull()
  })

  it('makes a title of a branch name nobody has typed one for', () => {
    expect(titleFromBranch('fix/login-crash')).toBe('Login crash')
    expect(titleFromBranch('123-fix_the_thing')).toBe('Fix the thing')
    expect(titleFromBranch('ABC-12-make-it-fast')).toBe('Make it fast')
  })

  it('takes one commit as the whole argument', () => {
    const start = draftFromCommits('123-login', [{ subject: 'Fix the login', body: 'It crashed.' }])
    expect(start.title).toBe('Fix the login')
    expect(start.body).toBe('It crashed.\n\nCloses #123')
  })

  it('lists several, under the branch name', () => {
    const start = draftFromCommits('login-crash', [
      { subject: 'Read the token', body: '' },
      { subject: 'Stop crashing', body: '' }
    ])
    expect(start.title).toBe('Login crash')
    expect(start.body).toBe('- Read the token\n- Stop crashing')
  })
})

describe('what is worth a notification', () => {
  const before = [review({ checks: 'pending' }), review({ number: 13, author: 'kai', checks: 'pending' })]

  it('says nothing about the first reading', () => {
    expect(changes(null, before, 'robin')).toEqual([])
  })

  it('tells the author when checks finish or fail and when somebody reviews', () => {
    const passed = changes(before, [review({ checks: 'success' })], 'robin')
    expect(passed.map((c) => c.text)).toEqual(['Checks passed'])
    const failed = changes(before, [review({ checks: 'failure' })], 'robin')
    expect(failed.map((c) => c.text)).toEqual(['Checks failed'])
    const approved = changes(before, [review({ checks: 'pending', review_state: 'approved' })], 'robin')
    expect(approved.map((c) => c.text)).toEqual(['Approved'])
    const asked = changes(before, [review({ checks: 'pending', review_state: 'changes_requested' })], 'robin')
    expect(asked.map((c) => c.text)).toEqual(['Changes requested'])
  })

  it('is about my reviews only, and only about changes', () => {
    expect(changes(before, [review({ number: 13, author: 'kai', checks: 'failure' })], 'robin')).toEqual([])
    expect(changes(before, before, 'robin')).toEqual([])
    expect(changes(before, [review({ checks: 'success' })], null)).toEqual([])
  })

  it('waits longer the longer the checks run', () => {
    expect(nextDelay(null)).toBe(20_000)
    expect(nextDelay(20_000)).toBe(40_000)
    expect(nextDelay(80_000)).toBe(120_000)
    expect(nextDelay(120_000)).toBe(120_000)
  })
})

describe('the offer after a first push', () => {
  beforeEach(async () => {
    toasts.clear()
    asked.mockReset()
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'config_get') return { version: 1, active_profile: null, global: {}, profiles: [] }
      if (cmd === 'forge_repo_info') {
        return { full_name: 'acme/app', default_branch: 'develop', merge_methods: ['merge'], parent: null, id: 1 }
      }
      if (cmd === 'forge_reviews') return []
      if (cmd === 'review_branch_commits') return [{ subject: 'Add the parser', body: 'Parses.' }]
      return null
    })
    await config.load()
    forge.store.status = {
      kind: 'github',
      host: 'github.com',
      has_token: true,
      user: null,
      slug: { host: 'github.com', owner: 'acme', name: 'app' },
      error: null
    }
    forge.store.facts = {}
    forge.store.compose = null
    git.store.trunk = { name: 'main', chosen: false }
  })

  it('offers, and the offer opens the dialog filled in against the default branch', async () => {
    await offerReviewAfterPush('42-parser', true)
    const offer = toasts.items.value.find((one) => one.actions?.length)
    expect(offer?.title).toBe('Open a PR for 42-parser?')
    await toasts.act(offer!, offer!.actions![0]!)
    expect(forge.store.compose).toEqual({
      source: '42-parser',
      target: 'develop',
      title: 'Add the parser',
      body: 'Parses.\n\nCloses #42'
    })
  })

  it('is not made for a push that set no upstream, the trunk, or a branch with a review', async () => {
    await offerReviewAfterPush('42-parser', false)
    await offerReviewAfterPush('develop', true)
    await offerReviewAfterPush('main', true)
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'forge_reviews') return [review({ source_branch: '42-parser' })]
      if (cmd === 'forge_repo_info') return { default_branch: 'develop', merge_methods: [] }
      return null
    })
    await offerReviewAfterPush('42-parser', true)
    expect(toasts.items.value.filter((one) => one.actions?.length)).toHaveLength(0)
  })

  it('is not made when the setting is off', async () => {
    config.store.config!.global.offer_review_after_push = false
    await offerReviewAfterPush('42-parser', true)
    expect(toasts.items.value).toHaveLength(0)
  })
})

describe('the home tab', () => {
  const card = (over: Partial<RepoCard>): RepoCard => ({
    path: '/repos/app',
    name: 'app',
    branch: 'main',
    ahead: 0,
    behind: 0,
    dirty: 0,
    last_commit: 0,
    exists: true,
    ...over
  })

  it('lists what needs me, forge rows first, each with what can be done', () => {
    const rows = needsMe(
      [
        card({
          unpushed: [{ name: 'local-only', count: 5 }],
          trunk: 'main',
          trunk_behind: 3,
          old_stashes: 2,
          oldest_stash_days: 40,
          gone: ['old-work']
        }),
        card({ path: '/gone', name: 'gone', exists: false })
      ],
      {
        rows: [
          { kind: 'requested', host: 'github.com', repo: 'acme/app', profile: 'Work', path: '/repos/app', review: review() },
          { kind: 'failing', host: 'gitlab.com', repo: 'g/x', profile: 'Lab', path: null, review: review({ number: 4 }) }
        ],
        errors: []
      }
    )
    const kinds = rows.map((row) => row.kind)
    expect(kinds).toEqual(['requested', 'failing', 'unpushed', 'behind', 'stash', 'stale', 'gone'])
    // With the project and host it is in: a number alone names a review in
    // whichever project happens to be open.
    expect(rows[0]!.actions[0]!.act).toEqual({
      path: '/repos/app',
      action: 'review',
      review: review(),
      repo: 'acme/app',
      host: 'github.com'
    })
    // No clone here: the forge's own page.
    expect(rows[1]!.actions[0]!.url).toContain('https://')
    expect(rows[1]!.where).toBe('g/x !4')
    expect(rows[2]!.text).toBe('5 commits never pushed')
    expect(rows[2]!.actions[0]!.act).toEqual({ path: '/repos/app', action: 'push', branch: 'local-only' })
    expect(rows[3]!.actions[0]!.act).toEqual({ path: '/repos/app', action: 'pull', branch: 'main' })
    expect(rows[6]!.actions[0]!.forget).toBe(true)
  })

  it('runs a row only in the project it is about', async () => {
    const push = vi.spyOn(git, 'pushBranch')
    git.store.repo = { path: '/repos/other' } as never
    await runHomeAct({ path: '/repos/app', action: 'push', branch: 'x' })
    expect(asked.mock.calls.some(([cmd]) => cmd === 'push')).toBe(false)
    push.mockRestore()
  })

  it('opens the review page for a review row', async () => {
    asked.mockImplementation(async (cmd: string) => (cmd === 'forge_reviews' ? [review()] : null))
    git.store.repo = { path: '/repos/app' } as never
    await runHomeAct({ path: '/repos/app', action: 'review', review: review() })
    expect(useReview().store.current?.number).toBe(12)
    useReview().close()
  })

  it('does not open a review by its number in a folder whose reviews come from elsewhere', async () => {
    const status = {
      kind: 'github',
      host: 'github.com',
      slug: { host: 'github.com', owner: 'robin', name: 'app' }
    }
    asked.mockImplementation(async (cmd: string) =>
      cmd === 'forge_status' ? status : cmd === 'forge_reviews' ? [review()] : null
    )
    git.store.repo = { path: '/repos/app' } as never
    useReview().close()
    await runHomeAct({
      path: '/repos/app',
      action: 'review',
      review: review(),
      repo: 'acme/app',
      host: 'github.com'
    })
    expect(useReview().store.current).toBeNull()
    expect(asked.mock.calls.some(([cmd]) => cmd === 'forge_reviews')).toBe(false)
  })

  it('matches a row to a project by owner, name and host, port or not', () => {
    const status = { host: 'gitlab.example.com:8443', slug: { owner: 'Team', name: 'App' } }
    expect(sameProject({ repo: 'team/app', host: 'gitlab.example.com' }, status)).toBe(true)
    expect(sameProject({ repo: 'team/app', host: 'gitlab.com' }, status)).toBe(false)
    expect(sameProject({ repo: 'other/app', host: 'gitlab.example.com' }, status)).toBe(false)
  })
})

describe('the forge status', () => {
  it('drops an answer for a tab that is no longer in front', async () => {
    let answer: (value: unknown) => void = () => {}
    asked.mockImplementation(async (cmd: string) =>
      cmd === 'forge_status' ? new Promise((resolve) => (answer = resolve)) : null
    )
    forge.store.status = null
    aimAt('/repos/a')
    const reading = forge.refreshStatus()
    aimAt('/repos/b')
    answer({ kind: 'github', host: 'github.com', slug: { host: 'github.com', owner: 'a', name: 'a' } })
    await reading
    expect(forge.store.status).toBeNull()
  })
})

describe('the review badge', () => {
  const locals = [
    { name: 'fix', upstream: 'origin/arno/fix' },
    { name: 'main', upstream: 'origin/main' },
    { name: 'mine', upstream: null }
  ]

  it('finds a branch’s review by the name it has on its remote', () => {
    const reviews = [
      review({ number: 1, source_branch: 'arno/fix' }),
      review({ number: 2, source_branch: 'mine' }),
      review({
        number: 3,
        source_branch: 'main',
        source: { full_name: 'kai/app', owner: 'kai', ssh_url: '', https_url: '', is_fork: true }
      })
    ]
    expect(reviewForBranch('fix', false, reviews, locals)?.number).toBe(1)
    expect(reviewForBranch('mine', false, reviews, locals)?.number).toBe(2)
    expect(reviewForBranch('main', false, reviews, locals)).toBeNull()
    // The branch checked out follows what the forge worked out.
    const current = [...reviews, review({ number: 9, source_branch: 'other', is_current: true })]
    expect(reviewForBranch('main', true, current, locals)?.number).toBe(9)
  })

  it('says how the checks and the reviewing stand', () => {
    const failing = badgeLook(review({ checks: 'failure', review_state: 'changes_requested' }), '#')
    expect(failing.tone).toBe('failing')
    expect(failing.text).toBe('#12')
    expect(failing.title).toContain('changes requested')
    expect(badgeLook(review({ checks: 'pending' }), '!').tone).toBe('pending')
    expect(badgeLook(review({ checks: 'success', review_state: 'approved' }), '#').title).toBe(
      '#12 · checks passing · approved'
    )
  })
})

describe('after a merge', () => {
  it('tells the backend where the merged branch lived', async () => {
    asked.mockImplementation(async () => null)
    useReview().store.current = review({
      source_branch: 'main',
      source: { full_name: 'kai/app', owner: 'kai', ssh_url: '', https_url: '', is_fork: true }
    })
    await useReview().afterMerge('main', null)
    const call = asked.mock.calls.find(([cmd]) => cmd === 'review_after_merge')
    expect((call?.[1] as { from: unknown }).from).toEqual({ fork: true, repo: 'kai/app' })
    useReview().close()
  })
})
