// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import HomePane from '~/components/HomePane.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useConfig } from '~/composables/useConfig'
import { ago, short, useHome, type HomeSummary } from '~/composables/useHome'

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
      read: 421,
      added: 4200,
      removed: 1800,
      repos_this_week: 2,
      favourite_word: 'fix',
      favourite_count: 30
    },
    author: 'robin@example.com',
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
            git_name: 'Robin Vale',
            git_email: 'robin@example.com',
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
  // The store outlives a mount, so a test that wants the first paint has to
  // start from one: `stale` only says the answer is old, not that there is none.
  home.store.summary = null
  home.store.error = null
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

    // Asking again is what the refresh button is for. Named rather than taken
    // as the first button on the page: settings and the profile sit above it.
    await second.find('[title="Read everything again"]').trigger('click')
    await flushPromises()
    expect(asked.mock.calls.filter(([cmd]) => cmd === 'home_summary')).toHaveLength(2)
  })
})

/**
 * Everything on the page comes out of one command, and the page used to spend
 * the wait saying things that were not true yet — no repositories, no year,
 * nothing waiting — and then grow by a screenful in a single frame. Drawn in
 * outline it arrives at close to its finished height and fills in.
 */
describe('the first paint', () => {
  it('draws the lists in outline before the read lands', () => {
    // Deliberately not flushed: this is the paint that happens while the one
    // command behind the page is still out.
    const wrapper = show()

    expect(wrapper.findAll('.ghost-row')).toHaveLength(6)
    expect(wrapper.findAll('.ghost-line')).toHaveLength(3)
    // None of the words it cannot know yet.
    expect(wrapper.text()).not.toContain('0 repositories')
    expect(wrapper.text()).not.toContain('Nothing waiting')
    expect(wrapper.text()).not.toContain('Nothing opened yet')
  })

  /**
   * The band is the tallest thing above the two lists. Drawn only once the
   * days arrived, it appeared on top of whatever you were about to click.
   */
  it('holds the year open as a grid of quiet days', () => {
    const wrapper = show()

    expect(wrapper.findAll('.week')).toHaveLength(53)
    expect(wrapper.findAll('.cell')).toHaveLength(371)
    expect(wrapper.findAll('.cell.l1, .cell.l2, .cell.l3, .cell.l4')).toHaveLength(0)
  })

  it('has nothing to say about a day in an outline', async () => {
    const wrapper = show()

    await wrapper.findAll('.cell')[0]!.trigger('mouseenter')
    expect(wrapper.find('.bubble').exists()).toBe(false)
  })

  it('puts the real thing in its place once it lands', async () => {
    const wrapper = show()
    await flushPromises()

    expect(wrapper.findAll('.ghost-row')).toHaveLength(0)
    expect(wrapper.findAll('.ghost-line')).toHaveLength(0)
    expect(wrapper.findAll('.row')).toHaveLength(3)
    expect(wrapper.text()).toContain('3 repositories')
  })

  /**
   * Coming back to the tab has last minute's answer on screen already.
   * Replacing something true with a row of grey bars to say it is being
   * checked again is a step backwards.
   */
  it('leaves what is on screen alone while it is being read again', async () => {
    const wrapper = show()
    await flushPromises()

    // Held open, so the refresh is still out while the page is looked at.
    let settle = () => {}
    asked.mockImplementationOnce(async () => {
      await new Promise<void>((resolve) => {
        settle = resolve
      })
      return summary()
    })

    await wrapper.find('[title="Read everything again"]').trigger('click')
    expect(home.store.loading).toBe(true)
    expect(wrapper.findAll('.ghost-row')).toHaveLength(0)
    expect(wrapper.findAll('.row')).toHaveLength(3)

    settle()
    await flushPromises()
    expect(wrapper.findAll('.row')).toHaveLength(3)
  })
})

describe('taking one off the list', () => {
  it('offers a cross on the ones that are not open, and none on the ones that are', async () => {
    const wrapper = show()
    await flushPromises()

    // The first row is the open tab; the other two are only recents.
    expect(wrapper.findAll('.row')[0]!.find('.drop').exists()).toBe(false)
    expect(wrapper.findAll('.row')[1]!.find('.drop').exists()).toBe(true)
  })

  it('forgets it without asking, and without re-reading everything', async () => {
    const wrapper = show()
    await flushPromises()
    const before = asked.mock.calls.filter(([cmd]) => cmd === 'home_summary').length

    await wrapper.findAll('.row')[1]!.find('.drop').trigger('click')
    await flushPromises()

    expect(asked.mock.calls.some(([cmd, args]) => cmd === 'project_forget' && (args as { path: string }).path === '/repos/api')).toBe(true)
    // Gone from the list on the spot, and no second read of every repository.
    expect(wrapper.findAll('.row')).toHaveLength(2)
    expect(asked.mock.calls.filter(([cmd]) => cmd === 'home_summary')).toHaveLength(before)
    // A cross is not a click on the row: nothing was opened.
    expect(wrapper.emitted('open')).toBeUndefined()
  })
})

describe('keeping the numbers honest', () => {
  it('reads again after something has been committed, but not before', async () => {
    const { useGit } = await import('~/composables/useGit')
    const git = useGit()

    const first = mount(HomePane, { global: { components: { MidTruncate } } })
    await flushPromises()
    first.unmount()
    expect(asked.mock.calls.filter(([cmd]) => cmd === 'home_summary')).toHaveLength(1)

    // Any write to any repository is the moment the counts stopped being true.
    git.store.repo = null
    await git.stage(['a.ts'])

    mount(HomePane, { global: { components: { MidTruncate } } })
    await flushPromises()
    expect(asked.mock.calls.filter(([cmd]) => cmd === 'home_summary')).toHaveLength(2)
  })
})

describe('the numbers on it', () => {
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
