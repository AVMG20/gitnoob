// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import DiffViewer from '~/components/DiffViewer.vue'
import DiffView from '~/components/DiffView.vue'
import { useGit, type FileDiff, type WorkingStatus } from '~/composables/useGit'
import { diffMode } from '~/composables/useDiffMode'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()

/** A diff with `count` one-line hunks, far enough apart to be separate. */
function hunks(count: number): FileDiff {
  return {
    path: 'a.txt',
    binary: false,
    truncated: 0,
    hunks: Array.from({ length: count }, (_, at) => ({
      header: `@@ -${at * 20 + 1},3 +${at * 20 + 1},4 @@`,
      lines: [
        { origin: ' ', old_lineno: at * 20 + 1, new_lineno: at * 20 + 1, content: 'x' },
        { origin: '+', old_lineno: null, new_lineno: at * 20 + 2, content: `added ${at}` },
        { origin: ' ', old_lineno: at * 20 + 2, new_lineno: at * 20 + 3, content: 'y' }
      ]
    }))
  }
}

const entry = (path: string) => ({ path, kind: 'modified' })
const status = (staged: string[], unstaged: string[]): WorkingStatus => ({
  staged: staged.map(entry),
  unstaged: unstaged.map(entry),
  conflicted: []
})

let left = 2
let calls: string[] = []

beforeEach(() => {
  left = 2
  calls = []
  diffMode.mode = 'diff'
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    calls.push(cmd)
    if (cmd === 'working_file_diff') return hunks(left)
    if (cmd === 'file_text') return 'x\n'
    if (cmd === 'apply_hunk') {
      left -= 1
      return 'Staged one hunk of a.txt'
    }
    return null
  })
  git.store.status = status([], ['a.txt', 'b.txt'])
  git.store.viewer = { path: 'a.txt', side: 'unstaged' }
})

async function open() {
  const wrapper = mount(DiffViewer, { global: { components: { DiffView } } })
  await flushPromises()
  return wrapper
}

/** The stage button on the hunk heading drawn in the list (not the pinned copy). */
function stageButton(wrapper: Awaited<ReturnType<typeof open>>) {
  return wrapper
    .findAll('.hunk-head:not(.pin) .hunk-btn')
    .find((one) => one.text().includes('Stage'))!
}

describe('staging one hunk at a time', () => {
  it('re-reads only the working tree, not the whole repository', async () => {
    const wrapper = await open()
    git.store.repo = { path: '/r', name: 'r', head: 'main', detached: false, state: '', author: '', author_email: '' }
    calls = []
    await stageButton(wrapper).trigger('click')
    await flushPromises()
    expect(calls).toContain('apply_hunk')
    expect(calls).toContain('working_status')
    expect(calls).not.toContain('commit_graph')
    expect(calls).not.toContain('ref_tree')
    git.store.repo = null
    wrapper.unmount()
  })

  it('keeps the file open on the hunk that is left', async () => {
    const wrapper = await open()
    await stageButton(wrapper).trigger('click')
    await flushPromises()
    expect(git.store.viewer).toEqual({ path: 'a.txt', side: 'unstaged' })
    // No "Loading" blank in between: the page was reloaded in place.
    expect(wrapper.text()).not.toContain('Loading diff')
    expect(wrapper.text()).toContain('added 0')
    wrapper.unmount()
  })

  it('moves on to the next file once the last hunk has gone', async () => {
    left = 1
    const wrapper = await open()
    await stageButton(wrapper).trigger('click')
    await flushPromises()
    expect(git.store.viewer).toEqual({ path: 'b.txt', side: 'unstaged' })
    wrapper.unmount()
  })
})

describe('the viewer header on the staged side', () => {
  it('offers no discard, which would only ever touch the unstaged copy', async () => {
    git.store.status = status(['a.txt'], [])
    git.store.viewer = { path: 'a.txt', side: 'staged' }
    const wrapper = await open()
    const labels = wrapper.findAll('header .btn').map((one) => one.text())
    expect(labels.some((one) => one.includes('Unstage file'))).toBe(true)
    expect(labels.some((one) => one.includes('Discard'))).toBe(false)
    wrapper.unmount()
  })

  it('still offers it beside the unstaged copy', async () => {
    const wrapper = await open()
    const labels = wrapper.findAll('header .btn').map((one) => one.text())
    expect(labels.some((one) => one.includes('Discard'))).toBe(true)
    wrapper.unmount()
  })
})
