// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import DiffViewer from '~/components/DiffViewer.vue'
import FileView from '~/components/FileView.vue'
import { diffMode } from '~/composables/useDiffMode'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()

let calls: { cmd: string; args: Record<string, unknown> }[] = []

const RUNS = [
  {
    oid: 'a91c4e2000000000000000000000000000000000',
    short: 'a91c4e2',
    summary: 'feat: the first pass',
    author: 'Ramon Robben',
    email: 'ramon@example.com',
    time: Math.floor(Date.now() / 1000) - 86400,
    start: 1,
    lines: 2,
    uncommitted: false
  }
]

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'file_text') return 'one\ntwo'
    if (cmd === 'working_file_diff') {
      return { path: 'app/app.vue', binary: false, truncated: 0, hunks: [] }
    }
    if (cmd === 'blame_file') return RUNS
    return null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [{ path: 'app/app.vue' }], conflicted: [] } as never
  git.store.viewer = { path: 'app/app.vue', side: 'unstaged' } as never
  diffMode.mode = 'file'
  diffMode.blame = false
})

let open: ReturnType<typeof mount> | null = null

const show = async () => {
  // The viewer reaches for its two views by name, which is Nuxt's doing and
  // not this environment's; the file one is the point of these tests.
  open = mount(DiffViewer, {
    global: { components: { FileView }, stubs: { Spinner: true, ChangeRuler: true } }
  })
  await flushPromises()
  return open
}

afterEach(() => {
  open?.unmount()
  open = null
  diffMode.mode = 'diff'
  diffMode.blame = false
})

const blamed = () => calls.filter((call) => call.cmd === 'blame_file')

describe('the blame toggle in the bar', () => {
  it('walks the patch and the file, and offers no third view', async () => {
    const wrapper = await show()
    const segs = wrapper.findAll('.seg')
    expect(segs.map((seg) => seg.text())).toEqual(['Diff', 'File'])
  })

  it('asks for no blame until the column is opened', async () => {
    await show()
    expect(blamed()).toHaveLength(0)
  })

  it('reads the blame when the button opens the column, and draws it', async () => {
    const wrapper = await show()
    const toggle = wrapper.findAll('.bar .btn')[0]!
    await toggle.trigger('click')
    await flushPromises()
    expect(diffMode.blame).toBe(true)
    expect(blamed()).toHaveLength(1)
    expect(wrapper.find('button.chip').text()).toContain('Ramon Robben')
  })

  it('takes the column away again, and the file stays', async () => {
    const wrapper = await show()
    const toggle = wrapper.findAll('.bar .btn')[0]!
    await toggle.trigger('click')
    await flushPromises()
    await toggle.trigger('click')
    await flushPromises()
    expect(wrapper.find('.chip').exists()).toBe(false)
    expect(wrapper.findAll('.line').length).toBeGreaterThan(1)
  })

  it('is not offered while the patch is what is on screen', async () => {
    diffMode.mode = 'diff'
    const wrapper = await show()
    const titles = wrapper.findAll('.bar .btn').map((btn) => btn.attributes('title'))
    expect(titles.some((title) => title?.includes('who last touched'))).toBe(false)
  })
})
