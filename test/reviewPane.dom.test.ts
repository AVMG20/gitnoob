// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import ReviewPane from '~/components/ReviewPane.vue'
import ReviewFilesPanel from '~/components/ReviewFilesPanel.vue'
import { useReview, type RComment, type ReviewStatus } from '~/composables/useReview'
import { useForge, type Review } from '~/composables/useForge'

/**
 * The review page, end to end against a fixture forge.
 *
 * Every backend call is answered from the tables below, and the ones that
 * change something change the tables — so posting a comment and then reading
 * the conversation back is the same round trip the real forge takes, minus
 * the network.
 */

const who = (login: string, name: string) => ({ login, name, avatar: null })
const now = '2026-08-20T10:00:00Z'

/** One remark, with the thread fields defaulted to a plain open one. */
function remark(partial: Partial<RComment> & Pick<RComment, 'id' | 'author' | 'body'>): RComment {
  return {
    created_at: now,
    updated_at: now,
    kind: 'issue',
    path: null,
    line: null,
    side: null,
    reply_to: null,
    thread: '',
    resolvable: false,
    resolved: false,
    outdated: false,
    ...partial
  }
}

let comments: RComment[] = []
let reviewState = 'open'
let draft = false
let labels = [{ name: 'enhancement', color: '#a2eeef' }]
let assignees: { login: string; name: string; avatar: string | null }[] = []
let reviewers = [who('robin', 'Robin Vale')]
let status: ReviewStatus
let resolved: Record<string, boolean> = {}
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const CURRENT: Review = {
  number: 38,
  title: 'Add the review page',
  author: 'kai',
  state: 'open',
  draft: false,
  source_branch: 'feature/review-page',
  target_branch: 'main',
  url: 'https://github.com/me/repo/pull/38',
  updated_at: now,
  is_current: false,
  head_sha: 'f'.repeat(40),
  source: null,
  warning: null
}

/** A small patch whose lines the fixtures anchor their remarks to. */
const FILE_PATCH = [
  '@@ -1,4 +1,6 @@',
  ' import { x } from "x"',
  '-const a = 1',
  '+const a = 2',
  '+const b = 3',
  ' ',
  ' export default a'
].join('\n')

const FILES = [
  {
    path: 'src/review/pane.ts',
    old_path: null,
    status: 'modified',
    additions: 2,
    deletions: 1,
    binary: false,
    patch: FILE_PATCH
  },
  {
    path: 'logo.png',
    old_path: null,
    status: 'added',
    additions: 0,
    deletions: 0,
    binary: true,
    patch: ''
  }
]

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string, args: Record<string, unknown> = {}) => {
    calls.push({ cmd, args })
    switch (cmd) {
      case 'forge_review_detail':
        return {
          number: 38,
          title: 'Add the review page',
          body: 'Reads the whole review here.\n\nSee `src/review/pane.ts`.',
          state: reviewState,
          draft,
          author: who('kai', 'Kai Moens'),
          assignees,
          reviewers,
          labels,
          milestone: null,
          source_branch: 'feature/review-page',
          target_branch: 'main',
          url: CURRENT.url,
          created_at: now,
          updated_at: now,
          comments: comments.length,
          merge_status: 'clean',
          base_sha: 'a'.repeat(40),
          head_sha: 'f'.repeat(40),
          start_sha: 'b'.repeat(40)
        }
      case 'forge_review_comments':
        return comments.map((one) =>
          one.thread ? { ...one, resolved: resolved[one.thread] ?? one.resolved } : { ...one }
        )
      case 'forge_review_files':
        return FILES
      case 'forge_review_commits':
        return [
          {
            sha: 'f'.repeat(40),
            message: 'Add the review page\n\nwith inline threads',
            author: 'Kai Moens',
            created_at: now
          }
        ]
      case 'forge_review_status':
        return status
      case 'forge_project_labels':
        return [
          { name: 'enhancement', color: '#a2eeef' },
          { name: 'bug', color: '#d73a4a' }
        ]
      case 'forge_post_comment':
        comments.push(
          remark({ id: 1000 + comments.length, author: who('robin', 'Robin Vale'), body: String(args.body) })
        )
        return null
      case 'forge_reply_comment': {
        const parent = comments.find((one) => one.id === args.parentId)
        comments.push(
          remark({
            id: 1000 + comments.length,
            author: who('robin', 'Robin Vale'),
            body: String(args.body),
            kind: parent?.kind ?? 'issue',
            path: parent?.path ?? null,
            line: parent?.line ?? null,
            side: parent?.side ?? null,
            reply_to: (parent?.reply_to ?? null) || (args.parentId as number)
          })
        )
        return null
      }
      case 'forge_add_diff_comment':
        comments.push(
          remark({
            id: 1000 + comments.length,
            author: who('robin', 'Robin Vale'),
            body: String(args.body),
            kind: 'diff',
            path: String(args.path),
            line: args.line as number,
            side: args.side as 'old' | 'new',
            thread: `thread-${args.line}`,
            resolvable: true
          })
        )
        return null
      case 'forge_resolve_thread':
        resolved[String(args.thread)] = args.resolved as boolean
        return null
      case 'forge_submit_review':
        comments.push(
          remark({
            id: 1000 + comments.length,
            author: who('robin', 'Robin Vale'),
            body: String(args.body || 'Approved.')
          })
        )
        status = {
          ...status,
          verdicts: [
            ...status.verdicts,
            {
              author: who('robin', 'Robin Vale'),
              state: args.event === 'approve' ? 'approved' : 'changes_requested',
              submitted_at: now,
              body: String(args.body ?? '')
            }
          ]
        }
        return null
      case 'forge_merge_review':
        reviewState = 'merged'
        return 'Merged'
      case 'forge_set_review_state':
        reviewState = String(args.action) === 'close' ? 'closed' : 'open'
        return null
      case 'forge_set_draft':
        draft = args.draft as boolean
        return null
      case 'forge_set_labels':
        labels = (args.labels as string[]).map((name) => ({ name, color: '' }))
        return null
      case 'forge_set_review_people':
        assignees = (args.assignees as { login: string; name: string }[]).map((one) =>
          who(one.login, one.name)
        )
        reviewers = (args.reviewers as { login: string; name: string }[]).map((one) =>
          who(one.login, one.name)
        )
        return null
      case 'forge_update_review':
        return null
      default:
        return null
    }
  })
}))

const review = useReview()
const forge = useForge()

beforeEach(() => {
  vi.clearAllMocks() // keeps per-call assertions clean; the fixture fn still runs
  calls = []
  reviewState = 'open'
  draft = false
  resolved = {}
  labels = [{ name: 'enhancement', color: '#a2eeef' }]
  assignees = []
  reviewers = [who('robin', 'Robin Vale')]
  status = {
    checks: [
      { name: 'build', state: 'success', description: '', url: 'https://ci.test/1' },
      { name: 'test', state: 'failure', description: '3 failed', url: 'https://ci.test/2' }
    ],
    checks_state: 'failure',
    verdicts: [
      { author: who('nadia', 'Nadia Petrova'), state: 'approved', submitted_at: now, body: 'ship it' }
    ],
    approvals: 1,
    approvals_required: 0,
    mergeable: true,
    merge_status: 'clean',
    conflicts: false
  }
  comments = [
    remark({ id: 11, author: who('kai', 'Kai Moens'), body: 'The **description** says it all.' }),
    remark({
      id: 12,
      author: who('nadia', 'Nadia Petrova'),
      body: 'Why two here?',
      kind: 'diff',
      path: 'src/review/pane.ts',
      line: 3,
      side: 'new',
      thread: 'thread-3',
      resolvable: true
    }),
    remark({
      id: 13,
      author: who('robin', 'Robin Vale'),
      body: 'Because `b` is next.',
      kind: 'diff',
      path: 'src/review/pane.ts',
      line: 3,
      side: 'new',
      reply_to: 12,
      thread: 'thread-3',
      resolvable: true
    })
  ]

  // The forge must look usable before any review lookup will run, and the
  // account known, so the remarks it made can be counted.
  forge.store.status = {
    kind: 'github',
    host: 'github.com',
    has_token: true,
    user: 'robin',
    slug: { host: 'github.com', owner: 'me', name: 'repo' },
    error: null
  }
  forge.store.me = { login: 'robin', id: 1, avatar: null }
  forge.store.details = {}
  forge.store.detailsFor = null
  forge.store.members = [
    { id: 1, login: 'robin', name: 'Robin Vale' },
    { id: 2, login: 'nadia', name: 'Nadia Petrova' }
  ]
  forge.store.membersFor = 'github@github.com/me/repo'

  review.close()
  Object.assign(review.store, {
    detail: null,
    comments: [],
    files: [],
    commits: [],
    status: null,
    tab: 'conversation',
    selectedPath: null,
    viewed: new Set(),
    draft: null,
    replyingTo: null,
    drafts: { talk: '', lines: {} },
    sending: false,
    acting: null,
    loadingDetail: false,
    loadingComments: false,
    loadingFiles: false,
    loadingCommits: false,
    loadingStatus: false,
    detailError: null,
    commentsError: null
  })
  localStorage.clear()
})

async function open() {
  review.show(CURRENT)
  await flushPromises()
  const wrapper = mount(ReviewPane)
  await flushPromises()
  return wrapper
}

const button = (wrapper: ReturnType<typeof mount>, text: string) =>
  wrapper.findAll('button').find((one) => one.text().includes(text))

describe('ReviewPane', () => {
  it('opens on a review and reads it whole', async () => {
    const wrapper = await open()
    expect(wrapper.find('[data-review-open]').exists()).toBe(true)
    expect(wrapper.text()).toContain('#38')
    expect(wrapper.text()).toContain('Add the review page')
    expect(wrapper.text()).toContain('feature/review-page')
    // The five reads: what it says, what was said, what it touches, how it
    // got there, and where it stands.
    for (const cmd of [
      'forge_review_detail',
      'forge_review_comments',
      'forge_review_files',
      'forge_review_commits',
      'forge_review_status'
    ]) {
      expect(calls.map((call) => call.cmd)).toContain(cmd)
    }
  })

  it('shows the conversation: description, threads, and where they stand', async () => {
    const wrapper = await open()
    expect(wrapper.find('[data-testid="conversation"]').exists()).toBe(true)
    // The description, rendered from its markdown.
    expect(wrapper.text()).toContain('Reads the whole review here.')
    expect(wrapper.html()).toContain('<code>src/review/pane.ts</code>')
    // The conversation thread, emphasis carried over.
    expect(wrapper.html()).toContain('<strong>description</strong>')
    // Both threads read here: the one on the conversation and the one on a
    // line of the diff, which is part of the conversation too.
    expect(wrapper.findAll('[data-testid="thread"]').length).toBe(2)
    expect(wrapper.text()).toContain('src/review/pane.ts:3')
  })

  it('draws the standing of the review beside it', async () => {
    const wrapper = await open()
    // The verdict somebody left is an event in the timeline…
    expect(wrapper.find('[data-testid="verdict-event"]').text()).toContain('Nadia')
    expect(wrapper.find('[data-testid="verdict-event"]').text()).toContain('approved')
    // …and the sidebar says where the checks got to.
    const sidebar = wrapper.find('[data-testid="review-sidebar"]')
    expect(sidebar.text()).toContain('1 of 2 failed')
    // The merge box leads with what is standing in the way.
    expect(wrapper.find('[data-testid="merge-box"]').text()).toContain('Checks failed')
  })

  it('posts a comment and reads it back', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="comment-input"]').setValue('Looks good overall')
    await wrapper.find('[data-testid="comment-send"]').trigger('click')
    await flushPromises()
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_post_comment', {
      number: 38,
      body: 'Looks good overall'
    })
    // The refresh brings the new remark on screen.
    expect(wrapper.text()).toContain('Looks good overall')
    expect(wrapper.findAll('[data-testid="thread"]').length).toBe(3)
  })

  it('approves through the finish modal, with any words beside it', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="review-mode-toggle"]').trigger('click')
    await flushPromises()
    await wrapper.find('textarea.summary').setValue('ship it')
    await wrapper.find('[data-testid="finish-approve"]').trigger('click')
    await flushPromises()
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_submit_review', {
      number: 38,
      event: 'approve',
      body: 'ship it',
      comments: []
    })
  })

  it('merges through a dialog that says what it will do', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="merge-button"]').trigger('click')
    await flushPromises()

    // Nothing has happened yet: the dialog is the confirmation.
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith('forge_merge_review', expect.anything())
    const dialog = wrapper.find('.modal')
    expect(dialog.text()).toContain('feature/review-page')
    expect(dialog.text()).toContain('main')

    await wrapper.find('[data-testid="merge-delete-branch"]').setValue(true)
    await wrapper.find('[data-testid="merge-confirm"]').trigger('click')
    await flushPromises()

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_merge_review', {
      number: 38,
      squash: false,
      deleteBranch: true
    })
    // Settled reviews lose their merge button.
    expect(wrapper.find('[data-testid="merge-button"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('merged')
  })

  it('squashes when asked to', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="merge-button"]').trigger('click')
    await flushPromises()
    const squash = wrapper.findAll('.modal input[type="radio"]')[1]!
    await squash.setValue()
    await wrapper.find('[data-testid="merge-confirm"]').trigger('click')
    await flushPromises()
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_merge_review', {
      number: 38,
      squash: true,
      deleteBranch: false
    })
  })

  it('settles a thread and folds it away', async () => {
    const wrapper = await open()
    const resolve = wrapper.find('[data-testid="thread-resolve"]')
    expect(resolve.exists()).toBe(true)
    await resolve.trigger('click')
    await flushPromises()

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_resolve_thread', {
      number: 38,
      thread: 'thread-3',
      resolved: true
    })
    // Settled, it folds itself away and stops counting as work left.
    expect(wrapper.find('[data-testid="thread-folded"]').exists()).toBe(true)
    expect(review.openThreads.value).toBe(0)
    expect(review.resolvedThreads.value).toBe(1)

    // And it opens again on a click, because "why was that settled" is a
    // question people ask.
    await wrapper.find('[data-testid="thread-folded"]').trigger('click')
    expect(wrapper.text()).toContain('Why two here?')
  })

  it('lists the files of the whole review, with their threads on their lines', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-files"]').trigger('click')
    await flushPromises()

    // One file on the page at a time; arriving with nothing chosen reads the
    // first, the way the graph opens one file across itself.
    expect(review.store.selectedPath).toBe('src/review/pane.ts')
    expect(wrapper.findAll('[data-testid="review-diff"]')).toHaveLength(1)

    // The line the fixture anchored its remark to: new side, line 3.
    const line = wrapper.find('.diff-line[data-line="3"][data-side="new"]')
    expect(line.exists()).toBe(true)
    expect(line.text()).toContain('const b = 3')
    // The thread and its reply stand directly under that line.
    const after = line.element.nextElementSibling!
    expect(after.querySelector('[data-testid="thread"]')).toBeTruthy()
    expect(after.textContent).toContain('Why two here?')
    // Rendered, so the code span is a tag rather than backticks.
    expect(after.textContent).toContain('Because b is next.')
    expect(after.innerHTML).toContain('<code>b</code>')

    // The patch is parsed as the same shape a local diff takes.
    expect(wrapper.find('.diff-line[data-line="2"][data-side="new"]').text()).toContain(
      'const a = 2'
    )
    expect(wrapper.find('.diff-line.del').text()).toContain('const a = 1')

    // Choosing the binary from the panel swaps the page to it.
    review.store.selectedPath = 'logo.png'
    await flushPromises()
    expect(wrapper.text()).toContain('No text diff for this file.')
  })

  it('walks the files one at a time, ticking them read', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-files"]').trigger('click')
    await flushPromises()

    await wrapper.find('[data-testid="viewed-next"]').trigger('click')
    await flushPromises()
    // The one just read is ticked, and the page has moved to the next unread.
    expect(review.store.viewed.has('src/review/pane.ts')).toBe(true)
    expect(review.store.selectedPath).toBe('logo.png')
  })

  it('walks the files from the keyboard', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-files"]').trigger('click')
    await flushPromises()
    expect(review.store.selectedPath).toBe('src/review/pane.ts')

    const press = async (key: string, chord = false) => {
      window.dispatchEvent(
        new KeyboardEvent('keydown', { key, ctrlKey: chord, bubbles: true })
      )
      await flushPromises()
    }

    // Down and up step the list; the ends hold rather than wrap.
    await press('ArrowDown')
    expect(review.store.selectedPath).toBe('logo.png')
    await press('ArrowDown')
    expect(review.store.selectedPath).toBe('logo.png')
    await press('ArrowUp')
    expect(review.store.selectedPath).toBe('src/review/pane.ts')

    // The chord reads this one and moves to the next that is not read.
    await press('Enter', true)
    expect(review.store.viewed.has('src/review/pane.ts')).toBe(true)
    expect(review.store.selectedPath).toBe('logo.png')

    // And the same keys do nothing to the conversation, where they belong to
    // whatever is being read or written there.
    review.store.tab = 'conversation'
    await flushPromises()
    await press('ArrowDown')
    expect(review.store.selectedPath).toBe('logo.png')
  })

  it('reads the file under one bar', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-files"]').trigger('click')
    await flushPromises()

    // The bar carries what the second bar used to: where you are, and the tick.
    const bar = wrapper.find('.filebar')
    expect(bar.text()).toContain('1 of 2')

    await bar.find('[data-testid="viewed-tick"]').setValue(true)
    expect(review.store.viewed.has('src/review/pane.ts')).toBe(true)

    // The path is the panel's to say, and is not repeated over the diff.
    expect(wrapper.findAll('.file-head')).toHaveLength(0)
  })

  it('writes a remark straight onto a line', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-files"]').trigger('click')
    await flushPromises()

    // The plus is there to be used, with no mode to enter first.
    const line = wrapper.find('.diff-line[data-line="5"][data-side="new"]')
    const add = line.find('.line-add')
    expect(add.exists()).toBe(true)
    await add.trigger('click')
    await flushPromises()

    const boxes = wrapper.findAll('[data-testid="comment-input"]')
    expect(boxes.length).toBeGreaterThan(0)
    await boxes[0]!.setValue('drop this')
    // The second way to send is the one that does not wait for the verdict.
    await wrapper.find('[data-testid="comment-second"]').trigger('click')
    await flushPromises()

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_add_diff_comment', {
      number: 38,
      headSha: 'f'.repeat(40),
      baseSha: 'a'.repeat(40),
      startSha: 'b'.repeat(40),
      path: 'src/review/pane.ts',
      line: 5,
      side: 'new',
      body: 'drop this'
    })
  })

  it('holds a remark back for the review, and sends it with the verdict', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-files"]').trigger('click')
    await flushPromises()

    const line = wrapper.find('.diff-line[data-line="5"][data-side="new"]')
    await line.find('.line-add').trigger('click')
    await flushPromises()

    await wrapper.find('[data-testid="comment-input"]').setValue('rename this')
    await wrapper.find('[data-testid="comment-send"]').trigger('click')
    await flushPromises()

    // Nothing has gone out: it is standing on its line, marked as pending.
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      'forge_add_diff_comment',
      expect.anything()
    )
    const held = wrapper.find('[data-testid="pending-remark"]')
    expect(held.exists()).toBe(true)
    expect(held.text()).toContain('rename this')
    expect(review.store.pending).toHaveLength(1)
    // And it is kept, so closing the page mid-review does not lose it.
    expect(localStorage.getItem('gitnoob.review-drafts')).toContain('rename this')

    // The verdict carries it.
    await wrapper.find('[data-testid="review-mode-toggle"]').trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('1 remark waiting')
    await wrapper.find('[data-testid="finish-request"]').trigger('click')
    await flushPromises()

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_submit_review', {
      number: 38,
      event: 'request_changes',
      body: '',
      comments: [
        { path: 'src/review/pane.ts', line: 5, side: 'new', body: 'rename this' }
      ]
    })
    expect(review.store.pending).toHaveLength(0)
  })

  it('takes a held-back remark back again', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-files"]').trigger('click')
    await flushPromises()

    await wrapper.find('.diff-line[data-line="5"][data-side="new"] .line-add').trigger('click')
    await flushPromises()
    await wrapper.find('[data-testid="comment-input"]').setValue('never mind')
    await wrapper.find('[data-testid="comment-send"]').trigger('click')
    await flushPromises()

    await wrapper.find('[data-testid="pending-drop"]').trigger('click')
    await flushPromises()
    expect(review.store.pending).toHaveLength(0)
    expect(wrapper.find('[data-testid="pending-remark"]').exists()).toBe(false)
  })

  it('answers a conversation comment by quoting it into the composer', async () => {
    const wrapper = await open()

    // A conversation comment is not a thread on either forge, so it is not
    // offered a reply that would post nothing.
    const talk = wrapper.findAll('[data-testid="thread"]')[0]!
    expect(talk.text()).not.toContain('Reply…')
    await talk.find('[data-testid="quote-reply"]').trigger('click')
    await flushPromises()

    const box = wrapper.find('[data-testid="comment-input"]')
      .element as HTMLTextAreaElement
    // The body, quoted, and nothing else — the same as a forge's own.
    expect(box.value.trim()).toBe('> The **description** says it all.')
    // Kept, like anything else half-written here.
    expect(review.store.drafts.talk).toContain('description')

    // Quoting the same remark again adds nothing: it is a draft, not a log.
    await talk.find('[data-testid="quote-reply"]').trigger('click')
    await flushPromises()
    expect(review.store.drafts.talk.match(/^> /gm)).toHaveLength(1)
  })

  it('answers a thread where it stands', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-files"]').trigger('click')
    await flushPromises()

    const reply = button(wrapper, 'Reply…')
    expect(reply).toBeTruthy()
    await reply!.trigger('click')
    const box = wrapper.find('[data-testid="comment-input"]')
    await box.setValue('fair enough')
    await wrapper.find('[data-testid="comment-send"]').trigger('click')
    await flushPromises()

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_reply_comment', {
      number: 38,
      parentId: 12,
      body: 'fair enough'
    })
  })

  it('shows the commits the branch carries', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-commits"]').trigger('click')
    expect(wrapper.find('[data-testid="commit-row"]').text()).toContain(
      'Add the review page'
    )
  })

  it('shows what ran against the branch, failures first', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="tab-checks"]').trigger('click')
    await flushPromises()

    const rows = wrapper.findAll('[data-testid="check-row"]')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.text()).toContain('test')
    expect(rows[0]!.text()).toContain('failed')
    expect(rows[1]!.text()).toContain('build')
  })

  it('finishes through a modal that counts what was said', async () => {
    const wrapper = await open()
    // One click on the pill is the whole gesture: the remarks are already
    // made, everywhere in the page, with no mode to enter first.
    await wrapper.find('[data-testid="review-mode-toggle"]').trigger('click')
    await flushPromises()

    // One remark in the fixture is the reader's own.
    expect(wrapper.find('[data-testid="review-count"]').text()).toBe('1')
    expect(wrapper.text()).toContain('You have already sent')
    expect(wrapper.text()).toContain('1 remark')
    // And it says what is still open before a verdict is handed down.
    expect(wrapper.text()).toContain('1 thread')

    await wrapper.find('textarea.summary').setValue('good work')
    await wrapper.find('[data-testid="finish-approve"]').trigger('click')
    await flushPromises()

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_submit_review', {
      number: 38,
      event: 'approve',
      body: 'good work',
      comments: []
    })
    // The verdict spent, the modal is gone.
    expect(wrapper.find('textarea.summary').exists()).toBe(false)
  })

  it('marks a draft ready to be read', async () => {
    draft = true
    const wrapper = await open()
    expect(wrapper.find('[data-testid="review-state"]').text()).toBe('draft')
    // A draft has nothing to merge yet; it has something to become.
    expect(wrapper.find('[data-testid="merge-button"]').exists()).toBe(false)
    await wrapper.find('[data-testid="ready-button"]').trigger('click')
    await flushPromises()
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_set_draft', { number: 38, draft: false })
    expect(wrapper.find('[data-testid="review-state"]').text()).toBe('open')
  })

  it('rewrites the title and the description in place', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="edit-description"]').trigger('click')
    await flushPromises()

    await wrapper.find('input.title-field').setValue('Read reviews in the app')
    await wrapper.find('textarea.body-field').setValue('Everything, without the browser tab.')
    await wrapper.find('[data-testid="save-description"]').trigger('click')
    await flushPromises()

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_update_review', {
      number: 38,
      title: 'Read reviews in the app',
      body: 'Everything, without the browser tab.'
    })
    expect(wrapper.find('input.title-field').exists()).toBe(false)
  })

  it('keeps a half-written remark for the next visit', async () => {
    const wrapper = await open()
    // The composer is bound to the kept text; typing is all it takes.
    await wrapper.find('[data-testid="comment-input"]').setValue('started something')
    review.saveDrafts()
    expect(localStorage.getItem('gitnoob.review-drafts')).toBeTruthy()

    // Leaving and coming back hands the text back rather than a blank box.
    review.close()
    review.show(CURRENT)
    await flushPromises()
    const again = mount(ReviewPane)
    await flushPromises()
    expect((again.find('[data-testid="comment-input"]').element as HTMLTextAreaElement).value).toBe(
      'started something'
    )
  })

  it('goes straight back to the graph, no questions asked', async () => {
    const wrapper = await open()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await flushPromises()
    expect(review.store.current).toBeNull()
    expect(wrapper.find('[data-review-open]').exists()).toBe(false)
  })

  it('closes from the back button just as plainly', async () => {
    const wrapper = await open()
    await wrapper.find('[data-testid="review-close"]').trigger('click')
    expect(review.store.current).toBeNull()
    expect(wrapper.find('[data-review-open]').exists()).toBe(false)
  })
})

describe('the review sidebar', () => {
  it('says who is on the review and what they made of it', async () => {
    const wrapper = await open()
    const sidebar = wrapper.find('[data-testid="review-sidebar"]')
    // The one asked, and the one who answered without being asked.
    expect(sidebar.text()).toContain('Robin Vale')
    expect(sidebar.text()).toContain('Nadia Petrova')
    expect(sidebar.text()).toContain('waiting')
  })

  it('hands the review to somebody', async () => {
    const wrapper = await open()
    const sidebar = wrapper.find('[data-testid="review-sidebar"]')
    await sidebar.findAll('.edit')[1]!.trigger('click')
    await flushPromises()

    // The picker offers the project's people; choosing one and saving sends
    // both lists, since the forge takes them together.
    await wrapper.find('[data-testid="save-assignees"]').trigger('click')
    await flushPromises()
    const call = calls.find((one) => one.cmd === 'forge_set_review_people')
    expect(call).toBeTruthy()
    expect(call!.args.number).toBe(38)
  })

  it('changes the labels from the list the project keeps', async () => {
    const wrapper = await open()
    const sidebar = wrapper.find('[data-testid="review-sidebar"]')
    await sidebar.findAll('.edit')[2]!.trigger('click')
    await flushPromises()

    const bug = sidebar.findAll('.label.pick').find((one) => one.text().includes('bug'))
    expect(bug).toBeTruthy()
    await bug!.trigger('click')
    await wrapper.find('[data-testid="save-labels"]').trigger('click')
    await flushPromises()

    expect(vi.mocked(invoke)).toHaveBeenCalledWith('forge_set_labels', {
      number: 38,
      labels: ['enhancement', 'bug']
    })
  })
})

describe('ReviewFilesPanel', () => {
  it("carries the review's files in the panel the commits already use", async () => {
    review.show(CURRENT)
    await flushPromises()
    const wrapper = mount(ReviewFilesPanel)
    await flushPromises()

    // The list is the app's own file list: marks, counts, remarks.
    const rows = wrapper.findAll('.row.file')
    expect(rows.length).toBeGreaterThan(0)
    expect(wrapper.text()).toContain('+2')
    expect(wrapper.text()).toContain('−1')

    // A file carries what was said about it.
    const marked = rows.find((row) => row.attributes('data-path') === 'src/review/pane.ts')
    expect(marked!.text()).toContain('2')

    // Clicking one opens the files page on it.
    await marked!.trigger('click')
    expect(review.store.tab).toBe('files')
    expect(review.store.selectedPath).toBe('src/review/pane.ts')

    // The progress line says how far the reading has gone.
    expect(wrapper.text()).toContain('0 of 2 viewed')
    review.toggleViewed('src/review/pane.ts')
    await flushPromises()
    expect(wrapper.text()).toContain('1 of 2 viewed')
  })

  it('filters down to what is left to read, and what was talked about', async () => {
    review.show(CURRENT)
    await flushPromises()
    const wrapper = mount(ReviewFilesPanel)
    await flushPromises()

    review.toggleViewed('src/review/pane.ts')
    await flushPromises()
    await wrapper.find('[data-testid="filter-unread"]').trigger('click')
    let rows = wrapper.findAll('.row.file')
    expect(rows).toHaveLength(1)
    expect(rows[0]!.attributes('data-path')).toBe('logo.png')

    await wrapper.find('[data-testid="filter-talk"]').trigger('click')
    rows = wrapper.findAll('.row.file')
    expect(rows).toHaveLength(1)
    expect(rows[0]!.attributes('data-path')).toBe('src/review/pane.ts')

    // Settled is answered: the file drops out of what is still open, and the
    // count on the chip follows it down.
    await review.resolveThread(review.diffThreads.value[0]!, true)
    await flushPromises()
    expect(wrapper.findAll('.row.file')).toHaveLength(0)
    expect(wrapper.find('[data-testid="filter-talk"]').text()).toContain('0')
  })
})
