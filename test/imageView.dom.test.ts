// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import DiffViewer from '~/components/DiffViewer.vue'
import FileView from '~/components/FileView.vue'
import FileEditor from '~/components/FileEditor.vue'
import { diffMode } from '~/composables/useDiffMode'
import { useGit } from '~/composables/useGit'
import { resetEdits } from '~/composables/useFileEdit'
import { resetTabs, switchTabs } from '~/composables/useTabs'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => true) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []
/** What each side holds: by source. */
let sides: Record<string, { mime: string | null; size: number; missing?: boolean; text?: string }> = {}
let made = 0
const revoked: string[] = []
let wrapper: ReturnType<typeof mount> | null = null

beforeEach(() => {
  resetTabs()
  resetEdits()
  switchTabs('/repo')
  calls = []
  made = 0
  revoked.length = 0
  sides = {
    work: { mime: 'image/png', size: 2048 },
    index: { mime: 'image/png', size: 1024 },
    head: { mime: 'image/png', size: 1024 }
  }
  URL.createObjectURL = vi.fn(() => `blob:test/${++made}`)
  URL.revokeObjectURL = vi.fn((url: string) => void revoked.push(url))
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'image_read') {
      const side = sides[String(args?.source)]
      if (!side || side.missing) return { mime: null, size: 0, too_large: false, lfs: null, missing: true, data: null }
      return { mime: side.mime, size: side.size, too_large: false, lfs: null, missing: false, data: btoa(side.text ?? 'img') }
    }
    if (cmd === 'working_file_diff') return { path: 'x', binary: true, truncated: 0, hunks: [] }
    if (cmd === 'read_file') {
      return { text: '<svg xmlns="http://www.w3.org/2000/svg"/>\n', hash: 'h', eol: 'lf', bom: false, readonly: null, missing: false }
    }
    if (cmd === 'file_text') return '<svg xmlns="http://www.w3.org/2000/svg"/>\n'
    if (cmd === 'config_get') return { global: {}, profiles: [] }
    return null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  diffMode.mode = 'diff'
  git.store.status = null
  git.store.viewer = null
})

async function show(path: string, mode: 'file' | 'diff', kind = 'modified') {
  git.store.status = { staged: [], unstaged: [{ path, kind }], conflicted: [] } as never
  git.store.viewer = { path, side: 'unstaged' } as never
  diffMode.mode = mode
  wrapper = mount(DiffViewer, {
    attachTo: document.body,
    global: { components: { FileView, FileEditor }, stubs: { Spinner: true, ChangeRuler: true } }
  })
  await flushPromises()
  await flushPromises()
  return wrapper
}

/** happy-dom loads no pictures: says it did, at a size. */
async function loaded(img: Element, width: number, height: number) {
  Object.defineProperty(img, 'naturalWidth', { value: width })
  Object.defineProperty(img, 'naturalHeight', { value: height })
  img.dispatchEvent(new Event('load'))
  await flushPromises()
}

describe('images in the viewer', () => {
  it('draws an image from a blob URL, with its size in the header', async () => {
    const w = await show('img/logo.png', 'file')
    const img = w.find('.image-view img')
    expect(img.attributes('src')).toMatch(/^blob:/)
    expect(calls.find((one) => one.cmd === 'image_read')!.args).toMatchObject({ path: 'img/logo.png', source: 'work' })
    // Not read as text, and no editor.
    expect(calls.some((one) => one.cmd === 'file_text' || one.cmd === 'read_file')).toBe(false)
    expect(w.find('.cm-editor').exists()).toBe(false)
    await loaded(img.element, 640, 480)
    const facts = w.find('[data-fact="image"]').text()
    expect(facts).toContain('640 × 480')
    expect(facts).toContain('2.0 KB')
    expect(facts).toContain('PNG')
    expect(w.find('.stats').exists()).toBe(false)
    // The zoom buttons work.
    await w.find('.image-view .tool[title^="Actual size"]').trigger('click')
    expect(w.find('.image-view .pct').text()).toBe('100%')
    const url = img.attributes('src')!
    w.unmount()
    wrapper = null
    expect(revoked).toContain(url)
  })

  it('shows an SVG as an image, with a Source switch to its text in the editor', async () => {
    sides.work = { mime: 'image/svg+xml', size: 40 }
    const w = await show('icons/mark.svg', 'file')
    expect(w.find('.image-view img').exists()).toBe(true)
    // Never inline markup: the picture is an <img>, and no <svg> from the file is in the page.
    expect(w.find('.image-view svg:not(.lucide)').exists()).toBe(false)
    await w.find('[data-act="svg-source"]').trigger('click')
    await flushPromises()
    await flushPromises()
    expect(w.find('.image-view').exists()).toBe(false)
    expect(w.find('.cm-editor').exists()).toBe(true)
    expect(w.find('[data-act="svg-source"]').text()).toBe('Image')
  })

  it('compares a changed image side by side, the index against the disk', async () => {
    const w = await show('img/logo.png', 'diff')
    const figures = w.findAll('.image-diff .side')
    expect(figures).toHaveLength(2)
    expect(figures.every((one) => one.find('img').exists())).toBe(true)
    const sources = calls.filter((one) => one.cmd === 'image_read').map((one) => one.args.source)
    expect(sources).toEqual(expect.arrayContaining(['index', 'work']))
    await loaded(figures[1]!.find('img').element, 32, 16)
    expect(figures[1]!.text()).toContain('32 × 16')
    expect(figures[1]!.text()).toContain('2.0 KB')
    // Whole-file buttons only: there are no hunks.
    const labels = w.findAll('.bar .btn').map((one) => one.text())
    expect(labels).toEqual(expect.arrayContaining(['Discard', 'Stage file']))
    expect(w.find('.hunk').exists()).toBe(false)
  })

  it('shows only the one side of a new image', async () => {
    sides.index = { mime: null, size: 0, missing: true }
    const w = await show('img/new.png', 'diff', 'untracked')
    const figures = w.findAll('.image-diff .side')
    expect(figures[0]!.find('img').exists()).toBe(false)
    expect(figures[0]!.text()).toContain('Not there before')
    expect(figures[1]!.find('img').exists()).toBe(true)
  })

  it("shows a commit's image, and its change against the parent", async () => {
    sides.commit = { mime: 'image/png', size: 10 }
    sides.parent = { mime: 'image/png', size: 9 }
    git.store.viewer = { path: 'img/logo.png', commit: 'abc1234' } as never
    diffMode.mode = 'diff'
    wrapper = mount(DiffViewer, {
      attachTo: document.body,
      global: { components: { FileView, FileEditor }, stubs: { Spinner: true, ChangeRuler: true } }
    })
    await flushPromises()
    await flushPromises()
    const asks = calls.filter((one) => one.cmd === 'image_read').map((one) => one.args)
    expect(asks).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ source: 'parent', commit: 'abc1234' }),
        expect.objectContaining({ source: 'commit', commit: 'abc1234' })
      ])
    )
    diffMode.mode = 'file'
    await flushPromises()
    expect(wrapper.find('.image-view img').exists()).toBe(true)
  })
})
