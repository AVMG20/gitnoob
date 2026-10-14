// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import WorkingChanges from '~/components/WorkingChanges.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import FileList from '~/components/FileList.vue'
import AppModal from '~/components/AppModal.vue'
import DiscardConflictsDialog from '~/components/DiscardConflictsDialog.vue'
import { WIP, useGit, type RepoInfo } from '~/composables/useGit'
import { useAi } from '~/composables/useAi'
import { draftKey, resetDrafts, useCommitDraft } from '~/composables/useCommitDraft'
import { aimAt } from '~/composables/useInvoke'
import { useConfig } from '~/composables/useConfig'

/**
 * The commit box across a tab switch: everything that comes back after an
 * await belongs to the repository and branch it was asked in.
 */

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const asked = vi.mocked(invoke)
const git = useGit()
const ai = useAi()
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let answers: Record<string, (args: Record<string, unknown>) => unknown> = {}

const repo = (path: string, head: string): RepoInfo =>
  ({ path, name: path, head, detached: false, state: '', author: '', author_email: '' }) as never
const refsFor = (head: string) => ({
  locals: [{ name: head, oid: 'a', is_head: true, upstream: null, ahead: 1, behind: 0 }],
  remotes: [],
  tags: [],
  stashes: []
})
const drafts = useCommitDraft(() => ({ repo: null, branch: null }))
const draftOf = (path: string, head: string) => drafts.read(draftKey(path, head))

beforeEach(() => {
  localStorage.clear()
  resetDrafts()
  calls = []
  answers = {
    commit_attempt: () => ({ ok: true, output: 'done', hook: null }),
    commit_guard: () => [],
    stage: () => '',
    unstage: () => '',
    stage_all: () => '',
    working_status: () => git.store.status
  }
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return answers[cmd]?.(args ?? {}) ?? null
  })
  git.store.status = {
    staged: [{ path: 'package.json', kind: 'modified' }],
    unstaged: [],
    conflicted: []
  } as never
  git.store.progress = null
  git.store.viewer = null
  git.store.selected = WIP
  git.store.trunk = { name: null, chosen: false } as never
  ai.store.status = { configured: false, model: null, default_commit_prompt: '' } as never
})

function switchTo(path: string, head: string) {
  aimAt(path)
  git.store.repo = repo(path, head)
  git.store.refs = refsFor(head) as never
}

async function open() {
  const wrapper = mount(
    {
      components: { WorkingChanges, ContextMenu },
      template: '<div><WorkingChanges /><ContextMenu /></div>'
    },
    { global: { components: { FileList, AppModal, DiscardConflictsDialog } } }
  )
  await flushPromises()
  return wrapper
}

const button = (wrapper: Awaited<ReturnType<typeof open>>, text: string) =>
  wrapper.findAll('button').find((one) => one.text().includes(text))
const box = (wrapper: Awaited<ReturnType<typeof open>>) =>
  (wrapper.find('textarea').element as HTMLTextAreaElement).value

describe('the commit box across a tab switch', () => {
  it('takes the hook strip down, so its buttons cannot act on the other repository', async () => {
    answers.commit_attempt = () => ({
      ok: false,
      output: 'lint failed',
      hook: { hooks: ['pre-commit'], changed: ['package.json'], mixed: [] }
    })
    switchTo('/A', 'main')
    const wrapper = await open()
    await wrapper.find('textarea').setValue('A message')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(button(wrapper, 'Stage the hook')).toBeTruthy()

    switchTo('/B', 'feature')
    await flushPromises()
    expect(button(wrapper, 'Stage the hook')).toBeUndefined()
    expect(button(wrapper, 'Commit without hooks')).toBeUndefined()
    expect(calls.filter((c) => c.cmd === 'commit_attempt')).toHaveLength(1)
  })

  it('does not push or clear the other tab after a slow commit', async () => {
    let finish: (value: unknown) => void = () => {}
    answers.commit_attempt = () => new Promise((resolve) => (finish = resolve))
    switchTo('/B', 'feature')
    const wrapper = await open()
    await wrapper.find('textarea').setValue('B draft, keep me')
    switchTo('/A', 'main')
    await flushPromises()
    await wrapper.find('textarea').setValue('A message')
    await wrapper.find('.push-btn').trigger('click')
    await flushPromises()

    switchTo('/B', 'feature')
    await flushPromises()
    const before = calls.length
    finish({ ok: true, output: 'done', hook: null })
    await flushPromises()

    expect(box(wrapper)).toBe('B draft, keep me')
    expect(calls.slice(before).some((c) => c.cmd.includes('push'))).toBe(false)
    expect(draftOf('/A', 'main').message).toBe('')
  })

  it('keeps a model answer for one tab out of another', async () => {
    ai.store.status = { configured: true, model: 'm', default_commit_prompt: '' } as never
    let finish: (value: unknown) => void = () => {}
    answers.ai_commit_message = () => new Promise((resolve) => (finish = resolve))
    switchTo('/A', 'main')
    const wrapper = await open()
    await button(wrapper, 'Generate')!.trigger('click')
    await flushPromises()
    switchTo('/B', 'feature')
    await flushPromises()
    finish({ summary: 'Describe A changes', body: '' })
    await flushPromises()
    expect(box(wrapper)).toBe('')
    expect(draftOf('/A', 'main').message).toBe('')
  })

  it('puts the amend message into the draft it was asked for', async () => {
    let finish: (value: unknown) => void = () => {}
    answers.amend_draft = () => new Promise((resolve) => (finish = resolve))
    switchTo('/A', 'main')
    const wrapper = await open()
    await wrapper.find('.amend input').setValue(true)
    switchTo('/B', 'feature')
    await flushPromises()
    finish({ summary: 'Head of A', body: '', is_pushed: false, short: 'a' })
    await flushPromises()
    expect(box(wrapper)).toBe('')
    expect(draftOf('/A', 'main').message).toBe('Head of A')
  })
})

describe('stage everything, then commit', () => {
  it('asks the backend to refuse if something got staged unseen, and Go back unstages only what it staged', async () => {
    useConfig().settings.value = {
      ...(useConfig().settings.value ?? {}),
      commit: { guard: true }
    } as never
    git.store.status = {
      staged: [],
      unstaged: [
        { path: 'a.txt', kind: 'modified' },
        { path: 'new.txt', kind: 'untracked' }
      ],
      conflicted: []
    } as never
    answers.commit_guard = () => [{ what: 'A key', path: 'a.txt', line: 1, excerpt: 'sk-' }]
    switchTo('/A', 'main')
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Everything')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(calls.find((c) => c.cmd === 'stage_all')?.args.expectStaged).toBe(false)
    await button(wrapper, 'Go back')!.trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'unstage')?.args.paths).toEqual(['a.txt', 'new.txt'])
    expect(calls.some((c) => c.cmd === 'commit_attempt')).toBe(false)
  })
})
