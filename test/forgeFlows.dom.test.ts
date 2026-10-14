// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import AppModal from '~/components/AppModal.vue'
import PeoplePicker from '~/components/PeoplePicker.vue'
import SearchSelect from '~/components/SearchSelect.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import ReviewDialog from '~/components/ReviewDialog.vue'
import ReviewMergeDialog from '~/components/ReviewMergeDialog.vue'
import ReviewSidebar from '~/components/ReviewSidebar.vue'
import ReviewBadge from '~/components/ReviewBadge.vue'
import HomePane from '~/components/HomePane.vue'
import Toasts from '~/components/Toasts.vue'
import { useForge, type Review, type ReviewDetail } from '~/composables/useForge'
import { useGit, type LocalBranch } from '~/composables/useGit'
import { useReview } from '~/composables/useReview'
import { useToasts } from '~/composables/useToasts'
import { useConfig } from '~/composables/useConfig'
import { useHome } from '~/composables/useHome'
import { aimAt } from '~/composables/useInvoke'
import { checkProfile } from '~/composables/useForgeNudges'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))

const asked = vi.mocked(invoke)
const forge = useForge()
const git = useGit()
const review = useReview()
const toasts = useToasts()
const config = useConfig()
const home = useHome()

/** What each command answers; a function is called with the arguments. */
let answers: Record<string, unknown> = {}

const person = { login: 'kai', name: 'Kai', avatar: null }

const CURRENT: Review = {
  number: 38,
  title: 'The review',
  author: 'robin',
  state: 'open',
  draft: false,
  source_branch: 'feature',
  target_branch: 'main',
  url: 'https://github.com/acme/app/pull/38',
  updated_at: '',
  is_current: true,
  head_sha: 'f'.repeat(40),
  source: { full_name: 'acme/app', owner: 'acme', ssh_url: '', https_url: '', is_fork: false },
  warning: null
}

const DETAIL: ReviewDetail = {
  number: 38,
  title: 'The review',
  body: '',
  state: 'open',
  draft: false,
  author: { login: 'robin', name: 'Robin', avatar: null },
  assignees: [],
  reviewers: [person],
  labels: [],
  milestone: null,
  source_branch: 'feature',
  target_branch: 'main',
  url: CURRENT.url,
  created_at: '',
  updated_at: '',
  comments: 0,
  merge_status: null,
  base_sha: 'b'.repeat(40),
  head_sha: 'f'.repeat(40),
  start_sha: 'c'.repeat(40)
}

const branch = (name: string, is_head: boolean, upstream: string | null = `origin/${name}`): LocalBranch => ({
  name,
  oid: name.padEnd(40, '0'),
  is_head,
  upstream,
  ahead: 0,
  behind: 0
})

beforeEach(() => {
  answers = {
    forge_review_detail: DETAIL,
    forge_reviews: [],
    forge_review_status: null,
    forge_review_comments: [],
    forge_review_files: [],
    forge_review_commits: [],
    forge_members: [],
    ai_status: { configured: false, model: null, default_commit_prompt: '' },
    forge_repo_info: { full_name: 'acme/app', default_branch: 'main', merge_methods: ['merge', 'squash', 'rebase'], parent: null, id: 1 }
  }
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    const answer = answers[cmd]
    if (typeof answer === 'function') return (answer as (a?: Record<string, unknown>) => unknown)(args)
    return answer ?? null
  })
  aimAt(null)
  toasts.clear()
  forge.store.status = {
    kind: 'github',
    host: 'github.com',
    has_token: true,
    user: null,
    slug: { host: 'github.com', owner: 'acme', name: 'app' },
    error: null,
    remote: 'origin'
  }
  forge.store.facts = {}
  forge.store.details = {}
  forge.store.detailsFor = null
  forge.store.reviews = []
  forge.store.reviewsFor = null
  forge.store.error = null
  forge.store.compose = null
  forge.store.draft = null
  forge.store.draftFor = null
  forge.store.me = { login: 'robin', id: 1, avatar: null }
  git.store.repo = { path: '/repo', name: 'repo', head: 'feature', detached: false } as never
  git.store.refs = {
    locals: [branch('feature', true), branch('aaa-first', false), branch('develop', false)],
    remotes: [],
    tags: [],
    stashes: []
  }
  git.store.rows = []
  git.store.trunk = { name: 'develop', chosen: true }
  review.close()
  Object.assign(review.store, {
    current: CURRENT,
    detail: { ...DETAIL },
    status: null,
    commits: [],
    acting: null
  })
})

const modal = { global: { components: { AppModal, SearchSelect, PeoplePicker } } }

describe('merging', () => {
  it('offers only the ways the project allows, and defaults to one of them', async () => {
    answers.forge_repo_info = { default_branch: 'main', merge_methods: ['squash'], parent: null, id: 1 }
    const wrapper = mount(ReviewMergeDialog)
    await flushPromises()
    const ways = wrapper.findAll('input[type="radio"]')
    expect(ways).toHaveLength(1)
    expect((ways[0]!.element as HTMLInputElement).checked).toBe(true)
    await wrapper.find('[data-testid="merge-confirm"]').trigger('click')
    await flushPromises()
    expect(asked).toHaveBeenCalledWith('forge_merge_review', {
      number: 38,
      method: 'squash',
      deleteBranch: false,
      sha: 'f'.repeat(40)
    })
  })

  it('can rebase', async () => {
    const wrapper = mount(ReviewMergeDialog)
    await flushPromises()
    await wrapper.find('[data-testid="merge-method-rebase"]').setValue()
    await wrapper.find('[data-testid="merge-confirm"]').trigger('click')
    await flushPromises()
    expect(asked).toHaveBeenCalledWith(
      'forge_merge_review',
      expect.objectContaining({ method: 'rebase' })
    )
  })

  it('stays open with the reason when the forge refuses', async () => {
    answers.forge_merge_review = () => {
      throw new Error('405: Merge commits are not allowed on this repository')
    }
    const wrapper = mount(ReviewMergeDialog)
    await flushPromises()
    await wrapper.find('[data-testid="merge-confirm"]').trigger('click')
    await flushPromises()
    expect(wrapper.emitted('close')).toBeUndefined()
    expect(wrapper.find('[data-testid="merge-error"]').text()).toContain('not allowed')
  })

  it('reads the whole review again when its head moved', async () => {
    answers.forge_merge_review = () => {
      throw new Error('New commits were pushed to this branch since this page was loaded')
    }
    const wrapper = mount(ReviewMergeDialog)
    await flushPromises()
    asked.mockClear()
    await wrapper.find('[data-testid="merge-confirm"]').trigger('click')
    await flushPromises()
    const commands = asked.mock.calls.map(([cmd]) => cmd)
    expect(commands).toContain('forge_review_commits')
    expect(commands).toContain('forge_review_files')
  })

  it('tidies up afterwards and offers the branch, with a way back', async () => {
    answers.forge_merge_review = 'Merged'
    answers.review_after_merge = {
      message: 'Switched to main, which is up to date',
      switched_to: 'main',
      branch: 'feature',
      sha: 'e'.repeat(40),
      extra: 0
    }
    answers.review_delete_merged = 'Deleted feature (it was at eeeeeee)'
    answers.review_restore_branch = 'Restored feature'
    const wrapper = mount(ReviewMergeDialog)
    const corner = mount(Toasts)
    await flushPromises()
    await wrapper.find('[data-testid="merge-confirm"]').trigger('click')
    await flushPromises()
    expect(wrapper.emitted('close')).toHaveLength(1)
    expect(asked).toHaveBeenCalledWith('review_after_merge', {
      branch: 'feature',
      head: 'f'.repeat(40),
      remote: 'origin',
      from: expect.anything()
    })
    // Offered, not done.
    expect(asked.mock.calls.some(([cmd]) => cmd === 'review_delete_merged')).toBe(false)
    expect(corner.text()).toContain('Delete the branch feature?')
    await corner.find('[data-testid="toast-action"]').trigger('click')
    await flushPromises()
    expect(asked).toHaveBeenCalledWith('review_delete_merged', { name: 'feature', sha: 'e'.repeat(40) })
    expect(corner.text()).toContain('Deleted feature')
    await corner.find('[data-testid="toast-action"]').trigger('click')
    await flushPromises()
    expect(asked).toHaveBeenCalledWith('review_restore_branch', { name: 'feature', sha: 'e'.repeat(40) })
    corner.unmount()
  })
})

describe('answers that arrive late', () => {
  it('drops a list of reviews read for another tab', async () => {
    let release: (value: Review[]) => void = () => {}
    answers.forge_reviews = () => new Promise<Review[]>((resolve) => (release = resolve))
    aimAt('/tab-a')
    const pending = forge.loadReviews()
    aimAt('/tab-b')
    release([CURRENT])
    await pending
    expect(forge.store.reviews).toEqual([])
    aimAt(null)
  })

  it('does not write one review’s conversation into the next one opened', async () => {
    let release: (value: unknown) => void = () => {}
    answers.forge_review_comments = () => new Promise((resolve) => (release = resolve))
    const refreshing = review.refreshConversation()
    await flushPromises()
    review.store.current = { ...CURRENT, number: 39 }
    review.store.comments = []
    release([{ id: 1, body: 'for 38' }])
    await refreshing
    expect(review.store.comments).toEqual([])
  })
})

describe('the new review dialog', () => {
  it('aims at the forge’s default branch, then the trunk, never just the first name', async () => {
    answers.forge_repo_info = { default_branch: 'develop', merge_methods: [], parent: null, id: 1 }
    const wrapper = mount(ReviewDialog, modal)
    await flushPromises()
    const target = wrapper.findAll('.end').at(1)!
    expect(target.text()).toContain('develop')
    wrapper.unmount()

    answers.forge_repo_info = null
    forge.store.facts = {}
    const again = mount(ReviewDialog, modal)
    await flushPromises()
    expect(again.findAll('.end').at(1)!.text()).toContain('develop')
    again.unmount()
  })

  it('names the review the way GitHub does', async () => {
    answers.forge_create_review = { ...CURRENT, number: 7, warning: null }
    const wrapper = mount(ReviewDialog, modal)
    await flushPromises()
    await wrapper.find('input[type="text"]').setValue('Add it')
    const create = wrapper.findAll('.footer .btn').find((b) => b.text() === 'Create')!
    await create.trigger('click')
    await flushPromises()
    expect(git.store.log.some((line) => line.text.includes('Opened #7'))).toBe(true)
    wrapper.unmount()
  })

  it('starts from what a push asked for', async () => {
    forge.store.compose = { source: 'feature', target: 'develop', title: 'From the push', body: 'Closes #4' }
    const wrapper = mount(ReviewDialog, modal)
    await flushPromises()
    expect((wrapper.find('input[type="text"]').element as HTMLInputElement).value).toBe('From the push')
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe('Closes #4')
    wrapper.unmount()
  })
})

describe('asking again', () => {
  it('re-requests a review from somebody who has reviewed', async () => {
    review.store.status = {
      checks: [],
      checks_state: 'none',
      verdicts: [{ author: person, state: 'changes_requested', submitted_at: '', body: '' }],
      approvals: 0,
      approvals_required: 0,
      mergeable: null,
      merge_status: null,
      conflicts: false
    }
    const wrapper = mount(ReviewSidebar)
    await flushPromises()
    await wrapper.find('[data-testid="rerequest"]').trigger('click')
    await flushPromises()
    expect(asked).toHaveBeenCalledWith('forge_rerequest_review', { number: 38, login: 'kai' })
  })
})

describe('the branch badge', () => {
  it('opens the review it stands for', async () => {
    review.close()
    const wrapper = mount(ReviewBadge, { props: { review: { ...CURRENT, checks: 'pending' } } })
    expect(wrapper.text()).toContain('#38')
    expect(wrapper.classes()).toContain('pending')
    await wrapper.trigger('click')
    expect(review.store.current?.number).toBe(38)
  })
})

describe('the home tab', () => {
  it('forgets a project whose folder has gone instead of trying to open it', async () => {
    answers.config_get = {
      version: 1,
      active_profile: 'p1',
      global: {},
      profiles: [
        {
          id: 'p1',
          name: 'Work',
          forge: 'none',
          host: '',
          git_name: null,
          git_email: null,
          ssh_key: null,
          signing_key: null,
          signing_format: null,
          sign_commits: null,
          sign_tags: null,
          projects: [],
          recents: [{ path: '/gone', name: 'gone' }],
          active_project: null
        }
      ]
    }
    answers.project_forget = answers.config_get
    answers.home_summary = {
      repos: [
        { path: '/gone', name: 'gone', branch: '', ahead: 0, behind: 0, dirty: 0, last_commit: 0, exists: false }
      ],
      stats: null,
      author: null,
      uncounted: ['other']
    }
    await config.load()
    home.store.summary = null
    home.stale()
    const wrapper = mount(HomePane, { global: { components: { MidTruncate } } })
    await flushPromises()
    expect(wrapper.find('[data-testid="uncounted"]').exists()).toBe(false)
    await wrapper.find('[data-testid="waiting-gone"]').trigger('click')
    await flushPromises()
    expect(wrapper.emitted('open')).toBeUndefined()
    expect(asked).toHaveBeenCalledWith('project_forget', { path: '/gone' })
  })

  it('pushes never-pushed work through the project, not from home', async () => {
    answers.home_summary = {
      repos: [
        {
          path: '/repos/app',
          name: 'app',
          branch: 'main',
          ahead: 0,
          behind: 0,
          dirty: 0,
          last_commit: 0,
          exists: true,
          unpushed: [{ name: 'local-only', count: 5 }]
        }
      ],
      stats: null,
      author: null
    }
    home.store.summary = null
    home.stale()
    const wrapper = mount(HomePane, { global: { components: { MidTruncate } } })
    await flushPromises()
    expect(wrapper.text()).toContain('5 commits never pushed')
    expect(wrapper.text()).not.toContain('everything is committed and pushed')
    await wrapper.find('[data-testid="waiting-unpushed"]').trigger('click')
    expect(wrapper.emitted('act')?.[0]).toEqual([
      { path: '/repos/app', action: 'push', branch: 'local-only' }
    ])
    expect(asked.mock.calls.some(([cmd]) => cmd === 'push')).toBe(false)
  })
})

describe('the profile a repository belongs to', () => {
  it('asks before using another profile for a repository', async () => {
    answers.config_get = { version: 1, active_profile: null, global: { profile_match: 'ask' }, profiles: [] }
    await config.load()
    answers.profile_match = {
      host: 'gitlab.com',
      repo: 'acme/app',
      using: { id: 'gh', name: 'Personal' },
      bound: false,
      fits: false,
      best: [{ id: 'gl', name: 'Work' }]
    }
    answers.repo_profile_set = answers.config_get
    answers.forge_status = forge.store.status
    const corner = mount(Toasts)
    await checkProfile()
    await flushPromises()
    expect(corner.text()).toContain('Use the Work profile here?')
    expect(asked.mock.calls.some(([cmd]) => cmd === 'repo_profile_set')).toBe(false)
    await corner.find('[data-testid="toast-action"]').trigger('click')
    await flushPromises()
    expect(asked).toHaveBeenCalledWith('repo_profile_set', { path: '/repo', id: 'gl' })
    corner.unmount()
  })

  it('asks before replacing the address a repository commits with', async () => {
    answers.profile_match = { host: null, repo: null, using: null, bound: false, fits: true, best: [] }
    answers.identity_check = {
      local_name: 'Robin',
      local_email: 'robin@work.example',
      profile: 'Personal',
      name: 'Robin',
      email: 'robin@home.example',
      fits: true,
      differs: true
    }
    const corner = mount(Toasts)
    await checkProfile()
    await flushPromises()
    expect(corner.text()).toContain('Commit here as robin@home.example?')
    expect(asked.mock.calls.some(([cmd, args]) => cmd === 'apply_identity' && (args as { force?: boolean })?.force)).toBe(false)
    await corner.find('[data-testid="toast-action"]').trigger('click')
    await flushPromises()
    expect(asked).toHaveBeenCalledWith('apply_identity', { force: true })
    corner.unmount()
  })
})
