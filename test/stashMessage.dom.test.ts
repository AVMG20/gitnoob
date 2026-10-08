// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import WorkingChanges from '~/components/WorkingChanges.vue'
import FileList from '~/components/FileList.vue'
import AppModal from '~/components/AppModal.vue'
import DiscardConflictsDialog from '~/components/DiscardConflictsDialog.vue'
import { useGit, type StashEntry } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()

const stash = (index: number, message: string, named: boolean): StashEntry => ({
  index,
  oid: `${index}-oid`,
  message,
  branch: 'main',
  time: 0,
  files: 1,
  named
})

const status = {
  staged: [],
  unstaged: [{ path: 'app/app.vue', kind: 'modified' }],
  conflicted: []
}

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'stash_apply' || cmd === 'stash_pop') return 'Applied'
    if (cmd === 'stash_apply_many') return { applied: ['a', 'b'], stopped: null, conflicted: [] }
    return null
  })
  git.store.status = status as never
  git.store.progress = null
  git.store.carried = null
  git.store.stashes = [
    stash(0, 'Fix the login crash', true),
    stash(1, '1234abc First', false),
    stash(2, 'Tidy the sidebar', true)
  ]
})

let mounted: ReturnType<typeof mount> | null = null

afterEach(() => {
  mounted?.unmount()
  mounted = null
})

async function open() {
  const wrapper = mount(WorkingChanges, {
    global: { components: { FileList, AppModal, DiscardConflictsDialog } }
  })
  mounted = wrapper
  await flushPromises()
  return wrapper
}

const box = (wrapper: Awaited<ReturnType<typeof open>>) =>
  wrapper.find('textarea').element as HTMLTextAreaElement

/**
 * A stash pushed from the commit box is named after the summary in it, so
 * putting it back starts the box from that name again — but only a name
 * somebody chose, and never over a message already being written.
 */
describe('putting a stash back', () => {
  it('starts the commit box from the name the stash was given', async () => {
    const wrapper = await open()
    await git.stashApply(0)
    await flushPromises()
    expect(box(wrapper).value).toBe('Fix the login crash')
  })

  it('does the same for a pop, which takes the stash off the list', async () => {
    const wrapper = await open()
    await git.stashPop(2)
    await flushPromises()
    expect(box(wrapper).value).toBe('Tidy the sidebar')
  })

  it('leaves the box alone for a stash git named after the last commit', async () => {
    const wrapper = await open()
    await git.stashApply(1)
    await flushPromises()
    expect(box(wrapper).value).toBe('')
  })

  it('keeps what was already typed', async () => {
    const wrapper = await open()
    await wrapper.find('textarea').setValue('Something else')
    await git.stashApply(0)
    await flushPromises()
    expect(box(wrapper).value).toBe('Something else')
  })

  it('picks no one name when several named stashes go on together', async () => {
    const wrapper = await open()
    await git.stashApplyMany([0, 2])
    await flushPromises()
    expect(box(wrapper).value).toBe('')
  })

  it('waits for a box that is not on screen yet', async () => {
    await git.stashApply(0)
    const wrapper = await open()
    expect(box(wrapper).value).toBe('Fix the login crash')
  })
})
