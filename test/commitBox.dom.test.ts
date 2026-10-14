// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import WorkingChanges from '~/components/WorkingChanges.vue'
import RightPanel from '~/components/RightPanel.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import FileList from '~/components/FileList.vue'
import AppModal from '~/components/AppModal.vue'
import DiscardConflictsDialog from '~/components/DiscardConflictsDialog.vue'
import { WIP, useGit, type RepoInfo } from '~/composables/useGit'
import { useAi } from '~/composables/useAi'
import { useConfig } from '~/composables/useConfig'
import { resetDrafts } from '~/composables/useCommitDraft'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const ai = useAi()

let calls: { cmd: string; args: Record<string, unknown> }[] = []
/** What each command answers, by name; anything else answers null. */
let answers: Record<string, (args: Record<string, unknown>) => unknown> = {}

const repo = (head: string): RepoInfo => ({
  path: '/repo',
  name: 'repo',
  head,
  detached: false,
  state: '',
  author: '',
  author_email: ''
})

beforeEach(() => {
  localStorage.clear()
  resetDrafts()
  calls = []
  answers = {
    commit_attempt: () => ({ ok: true, output: 'done', hook: null }),
    commit_guard: () => [],
    // Git prints nothing for these when they work.
    stage: () => '',
    stage_all: () => '',
    working_status: () => git.store.status
  }
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return answers[cmd]?.(args ?? {}) ?? null
  })
  git.store.repo = null
  git.store.status = {
    staged: [{ path: 'app/app.vue', kind: 'modified' }],
    unstaged: [],
    conflicted: []
  }
  git.store.progress = null
  git.store.viewer = null
  git.store.selected = WIP
  git.store.trunk = { name: null, chosen: false }
  git.store.refs = null
  ai.store.status = { configured: false, model: null, default_commit_prompt: '' }
})

let mounted: ReturnType<typeof mount> | null = null
afterEach(() => {
  mounted?.unmount()
  mounted = null
})

async function open() {
  const wrapper = mount(
    { components: { WorkingChanges, ContextMenu }, template: '<div><WorkingChanges /><ContextMenu /></div>' },
    { global: { components: { FileList, AppModal, DiscardConflictsDialog } } }
  )
  mounted = wrapper
  await flushPromises()
  return wrapper
}

const commits = () => calls.filter((call) => call.cmd === 'commit_attempt')
const button = (wrapper: Awaited<ReturnType<typeof open>>, text: string) =>
  wrapper.findAll('button').find((one) => one.text().includes(text))

describe('one press is one commit', () => {
  it('ignores a second ⌘↵ while the first commit is still running', async () => {
    let finish: (value: unknown) => void = () => {}
    answers.commit_attempt = () => new Promise((resolve) => (finish = resolve))
    const wrapper = await open()
    const box = wrapper.find('textarea')
    await box.setValue('Add the parser')
    const press = { key: 'Enter', metaKey: true, ctrlKey: true }
    await box.trigger('keydown', press)
    await box.trigger('keydown', press)
    await wrapper.find('.btn-primary').trigger('click')
    finish({ ok: true, output: 'done', hook: null })
    await flushPromises()
    expect(commits()).toHaveLength(1)
  })
})

describe('nothing staged', () => {
  it('offers to stage everything and commit, rather than sitting disabled', async () => {
    git.store.status = {
      staged: [],
      unstaged: [
        { path: 'a.ts', kind: 'modified' },
        { path: 'new.ts', kind: 'untracked' }
      ],
      conflicted: []
    }
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Do the thing')
    const primary = wrapper.find('.btn-primary')
    expect(primary.text()).toContain('Stage all 2 and commit')
    expect(wrapper.text()).toContain('1 changed and 1 new')
    expect(primary.attributes('disabled')).toBeUndefined()

    await primary.trigger('click')
    await flushPromises()
    const order = calls.map((call) => call.cmd).filter((cmd) => cmd === 'stage_all' || cmd === 'commit_attempt')
    expect(order).toEqual(['stage_all', 'commit_attempt'])
  })
})

describe('the look over what is staged', () => {
  it('shows what it found and commits only when told to', async () => {
    answers.commit_guard = () => [
      { path: '.env', line: null, kind: 'file', what: 'An environment file', excerpt: null }
    ]
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Add config')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(commits()).toHaveLength(0)
    expect(wrapper.text()).toContain('An environment file')

    await button(wrapper, 'Commit anyway')!.trigger('click')
    await flushPromises()
    expect(commits()).toHaveLength(1)
  })

  it('is asked by default, and not when it is switched off', async () => {
    const settings = (guard: boolean) => ({
      version: 1,
      active_profile: null,
      profiles: [],
      global: { commit: { ticket_prefix: true, guard, guard_debug: false } }
    })
    answers.config_get = () => settings(false)
    await useConfig().load()
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Add config')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(calls.some((call) => call.cmd === 'commit_guard')).toBe(false)
    expect(commits()).toHaveLength(1)
    answers.config_get = () => settings(true)
    await useConfig().load()
  })
})

describe('a hook that stops the commit', () => {
  beforeEach(() => {
    answers.commit_attempt = (args) =>
      args.noVerify
        ? { ok: true, output: 'done', hook: null }
        : {
            ok: false,
            output: 'prettier: a.ts reformatted',
            hook: { hooks: ['pre-commit'], changed: ['app/app.vue'], mixed: [] }
          }
  })

  it('says what the hook said, in place, with the ways on', async () => {
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Change')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(wrapper.find('.hook-output').text()).toContain('prettier: a.ts reformatted')
    expect(button(wrapper, 'Retry')).toBeTruthy()
    expect(button(wrapper, 'Commit without hooks')).toBeTruthy()
    expect(button(wrapper, 'Stage the hook’s changes and retry')).toBeTruthy()
    // Nothing was skipped on anyone's behalf.
    expect(commits().every((call) => !call.args.noVerify)).toBe(true)
    // The message is still there to try again with.
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe('Change')
  })

  it('skips the hooks only from the button that says so', async () => {
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Change')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    await button(wrapper, 'Commit without hooks')!.trigger('click')
    await flushPromises()
    expect(commits().at(-1)?.args.noVerify).toBe(true)
    expect(wrapper.find('.hook-output').exists()).toBe(false)
  })

  it('stages just the files the hook rewrote before trying again', async () => {
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Change')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    await button(wrapper, 'Stage the hook’s changes')!.trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'stage')?.args.paths).toEqual(['app/app.vue'])
    expect(commits()).toHaveLength(2)
  })
})

describe('commit and push', () => {
  it('pushes the branch after the commit, setting its upstream when it has none', async () => {
    git.store.repo = repo('feature')
    git.store.refs = {
      locals: [{ name: 'feature', oid: 'a', is_head: true, upstream: null, ahead: 1, behind: 0 }],
      remotes: [],
      tags: [],
      stashes: []
    }
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Ship it')
    await wrapper.find('.push-btn').trigger('click')
    await flushPromises()
    const push = calls.find((call) => call.cmd === 'push')
    expect(push?.args).toMatchObject({ branch: 'feature', force: false, setUpstream: true })
    expect(calls.findIndex((c) => c.cmd === 'commit_attempt')).toBeLessThan(
      calls.findIndex((c) => c.cmd === 'push')
    )
  })

  it('never forces: a refused push after an amend is left to the toolbar', async () => {
    git.store.repo = repo('feature')
    git.store.refs = {
      locals: [
        { name: 'feature', oid: 'a', is_head: true, upstream: 'origin/feature', ahead: 1, behind: 1 }
      ],
      remotes: [],
      tags: [],
      stashes: []
    }
    answers.amend_draft = () => ({ summary: 'Old', body: '', is_pushed: true, short: 'abc1234' })
    answers.push = () => ({
      argv: [],
      ok: false,
      code: 1,
      stdout: '',
      stderr: ' ! [rejected] feature -> feature (non-fast-forward)'
    })
    const wrapper = await open()
    await wrapper.find('.amend input').setValue(true)
    await flushPromises()
    await wrapper.find('.push-btn').trigger('click')
    await flushPromises()
    const pushes = calls.filter((call) => call.cmd === 'push')
    expect(pushes).toHaveLength(1)
    expect(pushes[0]!.args.force).toBe(false)
    expect(git.store.pushBlocked?.branch).toBe('feature')
    git.store.pushBlocked = null
  })
})

describe('the model', () => {
  beforeEach(() => {
    ai.store.status = { configured: true, model: 'm', default_commit_prompt: '' }
    answers.ai_commit_message = () => ({ summary: 'ABC-1: Add the parser', body: '' })
  })

  it('finishes what was typed rather than replacing it, and can be taken back', async () => {
    const wrapper = await open()
    await wrapper.find('textarea').setValue('ABC-1: parser')
    await button(wrapper, 'Generate')!.trigger('click')
    await flushPromises()
    const asked = calls.find((call) => call.cmd === 'ai_commit_message')
    expect(asked?.args).toMatchObject({ amend: false, draft: 'ABC-1: parser' })
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe(
      'ABC-1: Add the parser'
    )

    await button(wrapper, 'Revert')!.trigger('click')
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe('ABC-1: parser')
  })

  it('describes the amended commit when amending, even with nothing staged', async () => {
    git.store.status = { staged: [], unstaged: [], conflicted: [] }
    answers.amend_draft = () => ({ summary: 'Old message', body: '', is_pushed: false, short: 'abc' })
    const wrapper = await open()
    await wrapper.find('.amend input').setValue(true)
    await flushPromises()
    const generate = button(wrapper, 'Generate')!
    expect(generate.attributes('disabled')).toBeUndefined()
    await generate.trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'ai_commit_message')?.args).toMatchObject({
      amend: true,
      draft: 'Old message'
    })
  })
})

describe('the warning about the trunk', () => {
  it('follows the repository’s own trunk rather than a list of names', async () => {
    git.store.repo = repo('trunk')
    git.store.trunk = { name: 'trunk', chosen: true }
    const wrapper = await open()
    expect(wrapper.text()).toContain('You are committing straight to')
    mounted?.unmount()

    git.store.repo = repo('main')
    const again = await open()
    expect(again.text()).not.toContain('You are committing straight to')
  })
})

describe('the ticket prefix', () => {
  it('starts an empty message with the branch’s ticket, which alone cannot be committed', async () => {
    git.store.repo = repo('feature/ABC-123-login')
    const wrapper = await open()
    const box = wrapper.find('textarea').element as HTMLTextAreaElement
    expect(box.value).toBe('ABC-123: ')
    expect(wrapper.find('.btn-primary').attributes('disabled')).toBeDefined()
  })
})

describe('the staged side of the file menu', () => {
  it('offers to unstage, not to discard what is on disk', async () => {
    const wrapper = await open()
    const row = wrapper.findAll('.row').find((one) => one.text().includes('app.vue'))!
    await row.trigger('contextmenu')
    await flushPromises()
    const labels = wrapper.findAll('.menu .item').map((one) => one.text())
    expect(labels.some((one) => one.includes('Unstage'))).toBe(true)
    expect(labels.some((one) => one.includes('Discard'))).toBe(false)
  })
})

describe('the draft', () => {
  it('survives the panel being taken down to read a commit', async () => {
    git.store.repo = repo('main')
    const wrapper = mount(RightPanel, {
      global: {
        components: { WorkingChanges, FileList, AppModal, DiscardConflictsDialog },
        stubs: { CommitDetails: true }
      }
    })
    mounted = wrapper
    await flushPromises()
    await wrapper.find('textarea').setValue('Half a thought')
    await wrapper.find('.amend input').setValue(false)

    git.store.selected = 'abc1234'
    await flushPromises()
    expect(wrapper.find('textarea').exists()).toBe(false)

    git.store.selected = WIP
    await flushPromises()
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe('Half a thought')
  })

  it('is cleared once the commit goes through', async () => {
    git.store.repo = repo('main')
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Done')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe('')
  })
})
