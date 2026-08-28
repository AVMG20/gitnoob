// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import FileView from '~/components/FileView.vue'
import { useContextMenu } from '~/composables/useContextMenu'
import type { BlameRun } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const now = Math.floor(Date.now() / 1000)

function run(over: Partial<BlameRun>): BlameRun {
  return {
    oid: 'a91c4e2000000000000000000000000000000000',
    short: 'a91c4e2',
    summary: 'feat: the first pass',
    author: 'Ramon Robben',
    email: 'ramon@example.com',
    time: now - 86400,
    start: 1,
    lines: 2,
    uncommitted: false,
    ...over
  }
}

const diff = { path: 'app/app.vue', binary: false, hunks: [], truncated: 0 }

const show = (runs: BlameRun[], blame = true, text: string | null = 'one\ntwo\nthree\nfour') =>
  mount(FileView, { props: { diff, text, runs, blame, top: 0, view: 900 } })

describe('the blame column in the file view', () => {
  it('draws one chip per run, not one per line', () => {
    const wrapper = show([
      run({ start: 1, lines: 2 }),
      run({ start: 3, lines: 2, author: 'A Contributor', oid: 'bbb', short: 'bbb1111' })
    ])
    expect(wrapper.findAll('button.chip')).toHaveLength(2)
    expect(wrapper.findAll('.chip.rule')).toHaveLength(2)
  })

  it('names the author on the line the run starts at', () => {
    const wrapper = show([run({ start: 1, lines: 4 })])
    expect(wrapper.find('button.chip').text()).toContain('Ramon Robben')
  })

  it('marks uncommitted lines and refuses to open a commit for them', () => {
    const wrapper = show([
      run({ start: 1, lines: 2 }),
      run({ start: 3, lines: 2, uncommitted: true, oid: '0'.repeat(40), short: '' })
    ])
    const chips = wrapper.findAll('button.chip')
    expect(chips[1]!.text()).toContain('Uncommitted')
    expect(chips[1]!.attributes('disabled')).toBeDefined()
  })

  it('fades the oldest run and leaves the newest at full strength', () => {
    const wrapper = show([
      run({ start: 1, lines: 2, time: now - 60 * 60 * 24 * 365 }),
      run({ start: 3, lines: 2, time: now, oid: 'bbb', short: 'bbb1111' })
    ])
    const chips = wrapper.findAll('button.chip')
    const older = Number(chips[0]!.attributes('style')?.match(/opacity:\s*([\d.]+)/)?.[1])
    const newer = Number(chips[1]!.attributes('style')?.match(/opacity:\s*([\d.]+)/)?.[1])
    expect(older).toBeLessThan(newer)
    expect(newer).toBeCloseTo(1)
  })

  it('draws no column at all while blame is off', () => {
    const wrapper = show([run({ start: 1, lines: 4 })], false)
    expect(wrapper.find('.chip').exists()).toBe(false)
    // The file is still there to read.
    expect(wrapper.findAll('.line').length).toBeGreaterThan(1)
  })

  it('offers the toggle on a right-click of the line numbers', async () => {
    const menu = useContextMenu()
    const wrapper = show([run({ start: 1, lines: 4 })], false)
    await wrapper.findAll('.no')[1]!.trigger('contextmenu')
    expect(menu.state.open).toBe(true)
    expect(menu.state.items[0]!.label).toBe('Show blame')
    menu.state.items[0]!.action?.()
    expect(wrapper.emitted('toggle-blame')).toHaveLength(1)
    menu.close()
  })

  it('says the column could not be read without taking the file away', () => {
    const wrapper = mount(FileView, {
      props: {
        diff,
        text: 'one\ntwo',
        runs: [],
        blame: true,
        blameError: 'No blame here',
        top: 0,
        view: 900
      }
    })
    expect(wrapper.text()).toContain('No blame here')
    expect(wrapper.findAll('.line').length).toBeGreaterThan(1)
  })
})
