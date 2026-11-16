// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import ProjectTabs from '~/components/ProjectTabs.vue'
import { useConfig } from '~/composables/useConfig'
import { openTab, resetTabs, switchTabs, useTabs } from '~/composables/useTabs'
import { editKey, resetEdits, saveStates } from '~/composables/useFileEdit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), ask: vi.fn(async () => false) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({}) }))

const asked = vi.mocked(invoke)
const config = useConfig()

const show = (home = false) => mount(ProjectTabs, { props: { home } })

beforeEach(async () => {
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'config_get') {
      return {
        version: 1,
        active_profile: 'p1',
        global: {},
        profiles: [
          {
            id: 'p1',
            name: 'Work',
            forge: 'none',
            host: '',
            git_name: 'Robin Vale',
            git_email: 'robin@example.com',
            ssh_key: null,
            signing_key: null,
            signing_format: null,
            sign_commits: null,
            sign_tags: null,
            projects: [
              { path: '/repos/gitui', name: 'gitui' },
              { path: '/repos/api', name: 'api' }
            ],
            recents: [],
            active_project: '/repos/gitui'
          }
        ]
      }
    }
    return null
  })
  await config.load()
})

/**
 * The tab strip. Home sits beside the tabs rather than among them, and the two
 * have to agree about which page the window is actually on.
 */
describe('the tab strip', () => {
  it('marks the open project', () => {
    const wrapper = show()
    expect(wrapper.findAll('.tab')[0]!.classes()).toContain('on')
  })

  it('marks no tab while home is the page on screen', () => {
    const wrapper = show(true)
    expect(wrapper.findAll('.tab').some((tab) => tab.classes().includes('on'))).toBe(false)
    expect(wrapper.find('.icon').classes()).toContain('on')
  })

  it('asks for the project a tab was clicked on, home or not', async () => {
    const wrapper = show(true)
    // The project the strip would have marked: from home it is still somewhere
    // to go, and the click used to land nowhere.
    await wrapper.findAll('.tab')[0]!.trigger('click')
    expect(wrapper.emitted('open')).toEqual([['/repos/gitui']])
  })

  it('asks for home when the home button is pressed', async () => {
    const wrapper = show()
    await wrapper.find('.icon').trigger('click')
    expect(wrapper.emitted('home')).toHaveLength(1)
  })
})

/**
 * ⌘W closes what is on screen: the file tab when a file is open, the project
 * tab when the graph is.
 */
describe('closing with the keyboard', () => {
  const isMac = navigator.userAgent.includes('Mac')
  const pressW = () =>
    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'w', metaKey: isMac, ctrlKey: !isMac, bubbles: true })
    )

  it('closes the file tab on screen and leaves the project open', async () => {
    resetTabs()
    switchTabs('/repos/gitui')
    openTab({ path: 'a.php' })
    const wrapper = show()
    await flushPromises()
    pressW()
    await flushPromises()
    expect(useTabs().state.tabs).toEqual([])
    expect(asked.mock.calls.some(([cmd]) => cmd === 'project_close')).toBe(false)
    wrapper.unmount()
  })

  it('closes the project when the graph is on screen', async () => {
    resetTabs()
    switchTabs('/repos/gitui')
    const wrapper = show()
    await flushPromises()
    pressW()
    await flushPromises()
    expect(asked.mock.calls.some(([cmd]) => cmd === 'project_close')).toBe(true)
    wrapper.unmount()
  })

  it('asks before closing a project with edits that did not save, and keeps it when told to', async () => {
    resetEdits()
    saveStates[editKey('/repos/api', 'work:app/a.ts')] = 'failed'
    const w = show()
    await flushPromises()
    const api = w.findAll('.tab').find((one) => one.text().includes('api'))!
    await api.find('.close').trigger('click')
    await flushPromises()
    expect(asked.mock.calls.some(([cmd]) => cmd === 'project_close' || cmd === 'config_close_project')).toBe(false)
    expect(config.projects.value.map((one) => one.path)).toContain('/repos/api')
    resetEdits()
  })
})
