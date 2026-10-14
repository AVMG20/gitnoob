// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import TitleBar from '~/components/TitleBar.vue'
import AppModal from '~/components/AppModal.vue'
import { useGit } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let answers: Record<string, unknown> = {}
let open: ReturnType<typeof mount> | null = null

const branch = (over: Record<string, unknown>) => ({
  name: 'feature',
  oid: 'f1',
  is_head: true,
  upstream: 'origin/feature',
  ahead: 0,
  behind: 0,
  gone: null,
  ...over
})

beforeEach(() => {
  calls = []
  answers = {}
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return answers[cmd] ?? null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'feature', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.progress = null
  git.store.inside = []
  git.store.stashes = []
  git.store.selected = '__working__'
  git.store.pushBlocked = null
  git.store.pushConfirm = null
  git.store.pushGone = null
  git.store.history = { undo: [], redo: [] }
  git.store.trunk = { name: 'main', chosen: false }
  git.store.refs = {
    locals: [branch({}), branch({ name: 'main', oid: 'm1', is_head: false, upstream: 'origin/main' })],
    remotes: [],
    tags: [],
    stashes: []
  } as never
})

afterEach(() => {
  open?.unmount()
  open = null
})

const show = () => {
  open = mount(TitleBar, {
    global: {
      components: { AppModal },
      stubs: { ProfileMenu: true, HistoryMenu: true, BranchDialog: true, Teleport: true }
    }
  })
  return open
}

const strip = (wrapper: ReturnType<typeof show>) =>
  wrapper.find('.banner.push').text().replace(/\s+/g, ' ')

describe('a push the remote refused', () => {
  it('offers a pull first when somebody else pushed', async () => {
    git.store.pushBlocked = { remote: 'origin', branch: 'feature', upstream: 'origin/feature', message: '' }
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.banner.push .btn.tiny').text()).toBe('Pull with rebase')
  })

  it('still leads with a pull right after a rebase, unless the backend says only copies would go', async () => {
    // The last step being a rebase is no proof: a colleague may have pushed
    // to the branch meanwhile, and their commits are what force would drop.
    git.store.history = {
      undo: [
        {
          id: 1,
          label: 'Rebase onto origin/main',
          kind: 'rebase',
          branch: 'feature',
          before: 'a',
          after: 'b',
          mode: 'hard',
          destructive: true,
          at: 0
        }
      ],
      redo: []
    }
    git.store.pushBlocked = { remote: 'origin', branch: 'feature', upstream: 'origin/feature', message: '' }
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.banner.push .btn.tiny').text()).toBe('Pull with rebase')
  })

  it('never forces without a lease when the preview could not read the remote', async () => {
    git.store.pushBlocked = { remote: 'origin', branch: 'feature', upstream: 'origin/feature', message: '' }
    answers.push_preview = null
    const wrapper = show()
    await flushPromises()
    await wrapper.findAll('.banner.push button').find((b) => b.text() === 'Force push…')!.trigger('click')
    await flushPromises()
    const yes = wrapper.findAll('.banner.push button').find((b) => b.text().startsWith('Yes, force push'))!
    expect(yes.attributes('disabled')).toBeDefined()
    expect(strip(wrapper)).toMatch(/nothing to lease/)
    await yes.trigger('click')
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'push')).toBe(false)
  })

  it('forces with the lease the preview read', async () => {
    git.store.pushBlocked = { remote: 'origin', branch: 'feature', upstream: 'origin/feature', message: '' }
    answers.push_preview = { upstream_oid: 'abc123', will_orphan: [] }
    const wrapper = show()
    await flushPromises()
    await wrapper.findAll('.banner.push button').find((b) => b.text() === 'Force push…')!.trigger('click')
    await flushPromises()
    await wrapper.findAll('.banner.push button').find((b) => b.text().startsWith('Yes, force push'))!.trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'push')?.args).toMatchObject({ force: true, lease: 'abc123' })
  })

  it('offers force-with-lease first when only rewritten copies would go', async () => {
    git.store.pushBlocked = {
      remote: 'origin',
      branch: 'feature',
      upstream: 'origin/feature',
      message: '',
      rewritten: true
    }
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.banner.push .btn.tiny').text()).toBe('Force push with lease…')
  })
})

describe('a branch whose upstream was deleted', () => {
  it('asks rather than pushing it back, and offers to tidy it when it landed', async () => {
    git.store.refs!.locals[0] = branch({ upstream: null, gone: 'origin/feature' }) as never
    answers.delete_branch_preview = { trunk_holds: true, against: 'origin/main', landed: 'squash' }
    const wrapper = show()
    const push = wrapper.findAll('.actions .btn').find((b) => b.text().startsWith('Push'))!
    expect(push.attributes('title')).toMatch(/was deleted on the remote/)
    await push.trigger('click')
    await flushPromises()

    expect(calls.some((c) => c.cmd === 'push')).toBe(false)
    const text = wrapper.find('.gone-strip').text().replace(/\s+/g, ' ')
    expect(text).toMatch(/origin\/feature was deleted on the remote/)
    expect(text).toMatch(/Switch to main and delete feature/)
    expect(text).toMatch(/Push to recreate origin\/feature/)
  })

  it('pushes it back only when asked to, to the name it had', async () => {
    git.store.refs!.locals[0] = branch({ upstream: null, gone: 'origin/feature' }) as never
    answers.delete_branch_preview = { trunk_holds: false, against: 'main' }
    const wrapper = show()
    await wrapper.findAll('.actions .btn').find((b) => b.text().startsWith('Push'))!.trigger('click')
    await flushPromises()
    const recreate = wrapper.findAll('.gone-strip button').find((b) => b.text().startsWith('Push to recreate'))!
    await recreate.trigger('click')
    await flushPromises()
    const push = calls.find((c) => c.cmd === 'push')
    expect(push?.args).toMatchObject({ remoteName: 'origin', branch: 'feature', setUpstream: false })
  })
})

describe('pushing straight to main', () => {
  it('asks first when settings say so, and pushes on "Push anyway"', async () => {
    const config = useConfig()
    const before = config.settings.value
    config.store.config = {
      ...(config.store.config ?? {}),
      global: { ...(before ?? {}), warn_push_to_trunk: true }
    } as never
    git.store.refs!.locals = [
      branch({ name: 'main', is_head: true, upstream: 'origin/main' }),
      branch({ is_head: false })
    ] as never
    git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
    const wrapper = show()
    await wrapper.findAll('.actions .btn').find((b) => b.text().startsWith('Push'))!.trigger('click')
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'push')).toBe(false)
    expect(wrapper.find('.confirm-strip').text()).toMatch(/main is the main branch/)

    await wrapper.findAll('.confirm-strip button').find((b) => b.text() === 'Push anyway')!.trigger('click')
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'push')).toBe(true)
  })
})

describe('every push goes through the same checks', () => {
  it('asks about a deleted upstream for a push that did not come from the toolbar', async () => {
    git.store.refs!.locals[0] = branch({ upstream: null, gone: 'origin/feature' }) as never
    answers.delete_branch_preview = { trunk_holds: false, against: 'main' }
    const wrapper = show()
    // What commit and push, a home row and the review dialog now call.
    const pushed = await git.requestPush('feature', true)
    await flushPromises()
    expect(pushed).toBeNull()
    expect(calls.some((c) => c.cmd === 'push')).toBe(false)
    expect(wrapper.find('.gone-strip').text()).toMatch(/origin\/feature was deleted on the remote/)
  })

  it('names where the push really lands', async () => {
    git.store.refs!.locals[0] = branch({ upstream: 'origin/main', push_to: 'origin/feature' }) as never
    const wrapper = show()
    const push = wrapper.findAll('.actions .btn').find((b) => b.text().startsWith('Push'))!
    expect(push.attributes('title')).toBe('Push to origin/feature')
  })

  it('does not push into another tab when the check outlives the tab', async () => {
    const config = useConfig()
    const before = config.settings.value
    config.store.config = {
      ...(config.store.config ?? {}),
      global: { ...(before ?? {}), warn_push_to_trunk: true }
    } as never
    const { aimAt } = await import('~/composables/useInvoke')
    aimAt('/repo')
    let answer: (value: boolean) => void = () => {}
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} })
      if (cmd === 'forge_branch_protected') return new Promise((done) => (answer = done))
      return answers[cmd] ?? null
    })
    const { useForge } = await import('~/composables/useForge')
    const protectedSpy = vi
      .spyOn(useForge(), 'isProtected')
      .mockImplementation(() => new Promise((done) => (answer = done as never)))
    const going = git.requestPush('feature', false)
    aimAt('/other')
    answer(false)
    expect(await going).toBeNull()
    expect(calls.some((c) => c.cmd === 'push')).toBe(false)
    protectedSpy.mockRestore()
    aimAt(null)
  })
})
