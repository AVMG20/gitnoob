// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import DiffViewer from '~/components/DiffViewer.vue'
import { useGit, type WorkingStatus } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)

const DIFF = {
  path: 'app/app.vue',
  binary: false,
  truncated: 0,
  hunks: [
    {
      header: '@@ -1,1 +1,1 @@',
      lines: [{ origin: '+', old_lineno: null, new_lineno: 1, content: 'one' }]
    }
  ]
}

const entry = (path: string) => ({ path, status: 'modified', staged: false })

const status = (staged: string[], unstaged: string[]): WorkingStatus => ({
  staged: staged.map(entry) as WorkingStatus['staged'],
  unstaged: unstaged.map(entry) as WorkingStatus['unstaged'],
  conflicted: []
})

const git = useGit()

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'working_file_diff' || cmd === 'commit_file_diff') return DIFF
    if (cmd === 'file_text') return 'one\n'
    return null
  })
  git.store.status = status([], ['app/app.vue'])
  git.store.viewer = { path: 'app/app.vue', side: 'unstaged' }
})

/**
 * The file viewer over the working tree. What it does when the file it is
 * showing stops being a changed file at all is the question here: a commit
 * empties both lists at once, and the viewer used to stay open on "No changes
 * in this file" over the list the user now wants back.
 */
describe('the file viewer', () => {
  it('closes itself once a commit has left nothing to look at', async () => {
    const wrapper = mount(DiffViewer)
    await flushPromises()
    expect(git.store.viewer).not.toBeNull()

    git.store.status = status([], [])
    await flushPromises()
    expect(git.store.viewer).toBeNull()
    wrapper.unmount()
  })

  it('stays open while other files are still changed', async () => {
    const wrapper = mount(DiffViewer)
    await flushPromises()

    git.store.status = status([], ['app/other.vue'])
    await flushPromises()
    expect(git.store.viewer).not.toBeNull()
    wrapper.unmount()
  })

  it('leaves a commit file alone whatever the working tree does', async () => {
    git.store.viewer = { path: 'app/app.vue', commit: 'abc1234' }
    const wrapper = mount(DiffViewer)
    await flushPromises()

    git.store.status = status([], [])
    await flushPromises()
    expect(git.store.viewer).not.toBeNull()
    wrapper.unmount()
  })
})
