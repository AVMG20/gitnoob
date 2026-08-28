// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import DiffViewer from '~/components/DiffViewer.vue'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()

let calls: { cmd: string; args: Record<string, unknown> }[] = []
let fileText = ''

const POINTER = [
  'version https://git-lfs.github.com/spec/v1',
  'oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393',
  'size 12582912',
  ''
].join('\n')

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'file_text') return fileText
    if (cmd === 'working_file_diff') {
      return { path: 'art/logo.psd', binary: false, truncated: 0, hunks: [] }
    }
    return null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.lfs = { in_use: true, installed: true }
  git.store.viewer = { path: 'art/logo.psd', side: 'unstaged' } as never
})

/**
 * One viewer at a time. It registers window listeners and reads a shared
 * store, so a test that leaves the last one mounted is a test that runs
 * against two of them.
 */
let open: ReturnType<typeof mount> | null = null

const show = async () => {
  open = mount(DiffViewer, { global: { stubs: { Spinner: true, ChangeRuler: true } } })
  await flushPromises()
  return open
}

afterEach(() => {
  open?.unmount()
  open = null
})

describe('an LFS file in the viewer', () => {
  it('says what the file is instead of drawing the pointer as its contents', async () => {
    fileText = POINTER
    const wrapper = await show()
    const panel = wrapper.find('.lfs')
    expect(panel.exists()).toBe(true)
    expect(panel.text()).toContain('Stored with Git LFS')
    expect(panel.text()).toContain('12 MB')
    expect(panel.text()).toContain('sha256:4d7a2146')
    // None of the ordinary views are drawn over the top of it.
    expect(wrapper.find('.file').exists()).toBe(false)
    expect(wrapper.find('.blame').exists()).toBe(false)
  })

  it('fetches just that file when asked', async () => {
    fileText = POINTER
    const wrapper = await show()
    await wrapper.find('.lfs .btn-primary').trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'lfs_pull')?.args).toEqual({ path: 'art/logo.psd' })
  })

  it('offers nothing to press when git-lfs is not installed', async () => {
    fileText = POINTER
    git.store.lfs = { in_use: true, installed: false }
    const wrapper = await show()
    expect(wrapper.find('.lfs .btn-primary').exists()).toBe(false)
    expect(wrapper.find('.lfs').text()).toContain('not installed')
  })

  it('leaves an ordinary file to the ordinary views', async () => {
    fileText = 'const x = 1\n'
    const wrapper = await show()
    expect(wrapper.find('.lfs').exists()).toBe(false)
  })
})
