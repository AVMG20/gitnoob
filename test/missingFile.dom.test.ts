// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import DiffViewer from '~/components/DiffViewer.vue'
import FileView from '~/components/FileView.vue'
import DiffView from '~/components/DiffView.vue'
import ChangeRuler from '~/components/ChangeRuler.vue'
import FileEditor from '~/components/FileEditor.vue'
import { useGit } from '~/composables/useGit'
import { diffMode } from '~/composables/useDiffMode'
import { goBack, openTab, resetTabs, switchTabs, useTabs } from '~/composables/useTabs'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const asked = vi.mocked(invoke)
const git = useGit()
const tabs = useTabs()
let wrapper: ReturnType<typeof mount> | null = null

beforeEach(() => {
  resetTabs()
  switchTabs('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] }
  diffMode.mode = 'file'
  diffMode.blame = false
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (args?.path === 'app/Old.php') {
      if (cmd === 'read_file') {
        return { text: '', hash: '', eol: 'lf', bom: false, readonly: null, missing: true }
      }
      throw new Error('No such file: app/Old.php')
    }
    if (cmd === 'read_file') {
      return { text: 'x\n', hash: 'h', eol: 'lf', bom: false, readonly: null, missing: false }
    }
    if (cmd === 'file_text') return 'x\n'
    return null
  })
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  git.store.repo = null
})

describe('a tab for a file that is gone', () => {
  it('says the file is gone, rather than showing a read error, when Back reopens it', async () => {
    openTab({ path: 'app/Old.php', mode: 'file' })
    openTab({ path: 'app/New.php', mode: 'file' })
    wrapper = mount(DiffViewer, {
      attachTo: document.body,
      global: { components: { FileView, DiffView, ChangeRuler, FileEditor } }
    })
    await flushPromises()

    // Deleted and committed elsewhere: no status entry, nothing on disk.
    goBack()
    await flushPromises()
    await flushPromises()
    expect(tabs.activeTab()?.path).toBe('app/Old.php')
    const panel = wrapper.find('.missing')
    expect(panel.exists()).toBe(true)
    expect(panel.text()).toContain('not in the work tree any more')
    expect(wrapper.text()).not.toContain('No such file')
    expect(wrapper.find('.cm-editor').exists()).toBe(false)

    await panel.findAll('button').find((one) => one.text() === 'Close the tab')!.trigger('click')
    expect(tabs.state.tabs.some((one) => one.path === 'app/Old.php')).toBe(false)
  })
})
