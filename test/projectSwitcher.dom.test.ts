// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import ProjectSwitcher from '~/components/ProjectSwitcher.vue'
import { useConfig, type Config } from '~/composables/useConfig'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const config = useConfig()

let calls: { cmd: string; args: Record<string, unknown> }[] = []

function configWith(projects: string[], recents: string[]): Config {
  const entry = (path: string) => ({ path, name: path.split('/').pop() ?? path })
  return {
    version: 1,
    active_profile: 'p1',
    global: {} as never,
    profiles: [
      {
        id: 'p1',
        name: 'Personal',
        forge: 'none',
        host: '',
        git_name: null,
        git_email: null,
        ssh_key: null,
        signing_key: null,
        signing_format: null,
        sign_commits: null,
        sign_tags: null,
        projects: projects.map(entry),
        recents: recents.map(entry),
        active_project: projects[0] ?? null
      }
    ]
  }
}

beforeEach(async () => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return configWith(['/work/api'], ['/work/api', '/home/site'])
  })
  await config.load()
})

const show = () => mount(ProjectSwitcher, { global: { stubs: { Teleport: true } } })

describe('the repository switcher', () => {
  it('lists the open tabs first, then what was opened before them', () => {
    const wrapper = show()
    const rows = wrapper.findAll('.row')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.text()).toContain('api')
    expect(rows[0]!.find('.badge').text()).toBe('open')
    expect(rows[1]!.text()).toContain('site')
    expect(rows[1]!.find('.badge').exists()).toBe(false)
  })

  it('filters on every word, against the name and the path alike', async () => {
    const wrapper = show()
    await wrapper.find('input').setValue('home site')
    expect(wrapper.findAll('.row')).toHaveLength(1)
    expect(wrapper.find('.row').text()).toContain('site')

    await wrapper.find('input').setValue('nothing here')
    expect(wrapper.findAll('.row')).toHaveLength(0)
    expect(wrapper.find('.none').text()).toContain('Nothing matches')
  })

  it('opens what the arrows land on, and closes behind itself', async () => {
    const wrapper = show()
    await wrapper.find('input').trigger('keydown', { key: 'ArrowDown' })
    await wrapper.find('input').trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('open')?.[0]).toEqual(['/home/site'])
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  it('wraps round the ends rather than stopping at them', async () => {
    const wrapper = show()
    await wrapper.find('input').trigger('keydown', { key: 'ArrowUp' })
    await wrapper.find('input').trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('open')?.[0]).toEqual(['/home/site'])
  })

  it('closes on Escape without opening anything', async () => {
    const wrapper = show()
    await wrapper.find('input').trigger('keydown', { key: 'Escape' })
    expect(wrapper.emitted('open')).toBeFalsy()
    expect(wrapper.emitted('close')).toBeTruthy()
  })

  it('forgets a past repository without opening it', async () => {
    const wrapper = show()
    await wrapper.findAll('.row')[1]!.find('.drop').trigger('click')
    await flushPromises()
    expect(calls.find((c) => c.cmd === 'project_forget')?.args).toEqual({ path: '/home/site' })
    expect(wrapper.emitted('open')).toBeFalsy()
  })

  it('offers no way to forget a tab that is open', () => {
    const wrapper = show()
    expect(wrapper.findAll('.row')[0]!.find('.drop').exists()).toBe(false)
  })
})
