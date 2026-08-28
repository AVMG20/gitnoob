// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import HomePane from '~/components/HomePane.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useConfig } from '~/composables/useConfig'
import { ago, busiestLabel, short, useHome, type HomeSummary } from '~/composables/useHome'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))

const asked = vi.mocked(invoke)
const home = useHome()
const config = useConfig()

function summary(over: Partial<HomeSummary> = {}): HomeSummary {
  return {
    repos: [
      { path: '/repos/gitui', name: 'gitui', branch: 'main', ahead: 0, behind: 0, dirty: 12, last_commit: 0, exists: true },
      { path: '/repos/api', name: 'api', branch: 'tickets', ahead: 6, behind: 0, dirty: 0, last_commit: 0, exists: true },
      { path: '/repos/gone', name: 'gone', branch: '', ahead: 0, behind: 0, dirty: 0, last_commit: 0, exists: false }
    ],
    stats: {
      days: Array.from({ length: 371 }, (_, at) => (at > 360 ? 2 : 0)),
      week: 14,
      previous_week: 9,
      streak: 4,
      best_streak: 11,
      busy_weekday: 4,
      busy_hour: 16,
      read: 421,
      added: 4200,
      removed: 1800,
      repos_this_week: 2,
      favourite_word: 'fix',
      favourite_count: 30
    },
    author: 'arno@example.com',
    ...over
  }
}

const show = () => mount(HomePane, { global: { components: { MidTruncate } } })

beforeEach(async () => {
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'home_summary') return summary()
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
            git_name: 'Arno Visker',
            git_email: 'arno@example.com',
            ssh_key: null,
            signing_key: null,
            signing_format: null,
            sign_commits: null,
            sign_tags: null,
            projects: [{ path: '/repos/gitui', name: 'gitui' }],
            recents: [],
            active_project: '/repos/gitui'
          }
        ]
      }
    }
    return null
  })
  await config.load()
  home.stale()
})

/**
 * The home tab. Everything on it is read in one command, and everything it
 * says is something the window could otherwise only tell you one repository at
 * a time.
 */
describe('the home tab', () => {
  it('reads the lot once and lists every project', async () => {
    const wrapper = show()
    await flushPromises()

    expect(asked.mock.calls.filter(([cmd]) => cmd === 'home_summary')).toHaveLength(1)
    expect(wrapper.findAll('.row')).toHaveLength(3)
    expect(wrapper.find('.row').text()).toContain('gitui')
  })

  it('says which ones are open rather than when they were last touched', async () => {
    const wrapper = show()
    await flushPromises()
    expect(wrapper.findAll('.row')[0]!.find('.when').text()).toBe('open')
  })

  it('turns the counts into what to do next', async () => {
    const wrapper = show()
    await flushPromises()

    const said = wrapper.findAll('.line').map((one) => one.text())
    expect(said.some((one) => one.includes('12 uncommitted changes'))).toBe(true)
    expect(said.some((one) => one.includes('6 commits not on origin'))).toBe(true)
    // A folder that has moved is worth saying out loud, not quietly dropping.
    expect(said.some((one) => one.includes('not there any more'))).toBe(true)
  })

  it('filters on every word, against the name and the path alike', async () => {
    const wrapper = show()
    await flushPromises()

    await wrapper.find('.find input').setValue('repos api')
    expect(wrapper.findAll('.row')).toHaveLength(1)
    expect(wrapper.find('.row').text()).toContain('api')
  })

  it('opens the project a row is clicked on, and never one that has moved', async () => {
    const wrapper = show()
    await flushPromises()

    await wrapper.findAll('.row')[1]!.trigger('click')
    expect(wrapper.emitted('open')?.[0]).toEqual(['/repos/api'])

    await wrapper.findAll('.row')[2]!.trigger('click')
    expect(wrapper.emitted('open')).toHaveLength(1)
  })

  it('draws a year as 53 columns of seven days', async () => {
    const wrapper = show()
    await flushPromises()

    expect(wrapper.findAll('.week')).toHaveLength(53)
    expect(wrapper.findAll('.cell')).toHaveLength(371)
    // The busiest days are the recent ones, which is where the colour is.
    expect(wrapper.findAll('.cell.l2').length).toBeGreaterThan(0)
  })

  it('does not read it all again on the way back in', async () => {
    const first = show()
    await flushPromises()
    first.unmount()

    const second = show()
    await flushPromises()
    expect(asked.mock.calls.filter(([cmd]) => cmd === 'home_summary')).toHaveLength(1)

    // Asking again is what the refresh button is for.
    await second.find('.actions .btn').trigger('click')
    await flushPromises()
    expect(asked.mock.calls.filter(([cmd]) => cmd === 'home_summary')).toHaveLength(2)
  })
})

describe('the numbers on it', () => {
  it('says when you work in words, and nothing when it cannot', () => {
    expect(busiestLabel(summary().stats)).toBe('Thu 16:00')
    expect(busiestLabel({ ...summary().stats, busy_weekday: 0 })).toBe('')
  })

  it('shortens the big numbers and leaves the small ones alone', () => {
    expect(short(940)).toBe('940')
    expect(short(4231)).toBe('4.2k')
  })

  it('says how long ago in the words the rest of the window uses', () => {
    const now = Math.floor(Date.now() / 1000)
    expect(ago(0)).toBe('')
    expect(ago(now - 120)).toBe('2m ago')
    expect(ago(now - 3 * 86400)).toBe('3 days ago')
  })
})
