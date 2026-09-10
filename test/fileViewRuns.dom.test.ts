// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import FileView from '~/components/FileView.vue'
import type { DiffLine, FileDiff } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const ctx = (old: number, now: number, content = 'keep'): DiffLine => ({
  origin: ' ',
  old_lineno: old,
  new_lineno: now,
  content
})
const add = (now: number, content = 'new'): DiffLine => ({
  origin: '+',
  old_lineno: null,
  new_lineno: now,
  content
})
const del = (old: number, content: string): DiffLine => ({
  origin: '-',
  old_lineno: old,
  new_lineno: null,
  content
})

/** Three lines rewritten in one edit, between two lines nobody touched. */
const diff: FileDiff = {
  path: 'app/app.vue',
  binary: false,
  truncated: 0,
  hunks: [
    {
      old_start: 1,
      old_lines: 5,
      new_start: 1,
      new_lines: 5,
      header: '@@',
      lines: [
        ctx(1, 1),
        del(2, 'was two'),
        del(3, 'was three'),
        del(4, 'was four'),
        add(2, 'two'),
        add(3, 'three'),
        add(4, 'four'),
        ctx(5, 5)
      ]
    }
  ]
}

const show = () =>
  mount(FileView, {
    props: { diff, text: 'keep\ntwo\nthree\nfour\nkeep', top: 0, view: 900 }
  })

describe('a rewritten block in the file view', () => {
  it('runs one bar down the three lines, rounded only at its ends', () => {
    const wrapper = show()
    const bars = wrapper.findAll('.gutter.live')
    expect(bars).toHaveLength(3)
    expect(bars.filter((bar) => bar.classes('head'))).toHaveLength(1)
    expect(bars.filter((bar) => bar.classes('tail'))).toHaveLength(1)
    expect(bars[0]!.classes()).toContain('head')
    expect(bars[2]!.classes()).toContain('tail')
  })

  it('caps a block of added lines the same way, so every bar reads as one', () => {
    const wrapper = mount(FileView, {
      props: {
        diff: {
          path: 'app/app.vue',
          binary: false,
          truncated: 0,
          hunks: [
            {
              old_start: 1,
              old_lines: 2,
              new_start: 1,
              new_lines: 4,
              header: '@@',
              lines: [ctx(1, 1), add(2, 'two'), add(3, 'three'), ctx(2, 4)]
            }
          ]
        },
        text: 'keep\ntwo\nthree\nkeep',
        top: 0,
        view: 900
      }
    })
    const bars = wrapper.findAll('.line.added .gutter')
    expect(bars).toHaveLength(2)
    expect(bars[0]!.classes()).toContain('head')
    expect(bars[0]!.classes()).not.toContain('tail')
    expect(bars[1]!.classes()).toContain('tail')
  })

  it('says how many lines the one bar answers for', () => {
    expect(show().findAll('.gutter.live')[1]!.attributes('title')).toContain('these 3 lines')
  })

  it('shows all three old lines from a click on any one of them', async () => {
    for (const at of [0, 1, 2]) {
      const wrapper = show()
      await wrapper.findAll('.gutter.live')[at]!.trigger('click')
      const panels = wrapper.findAll('.before.was-at')
      expect(panels).toHaveLength(1)
      expect(panels[0]!.findAll('.before-line')).toHaveLength(3)
      expect(panels[0]!.text()).toContain('was two')
      expect(panels[0]!.text()).toContain('was four')
    }
  })

  it('hangs the panel from the line that was clicked, which is one on screen', async () => {
    const wrapper = show()
    await wrapper.findAll('.gutter.live')[2]!.trigger('click')
    const rows = wrapper.findAll('.lines .line')
    expect(rows[3]!.find('.before.was-at').exists()).toBe(true)
    expect(rows[1]!.find('.before.was-at').exists()).toBe(false)
  })

  it('lights the whole run while it is open, and leaves no Was label on it', async () => {
    const wrapper = show()
    await wrapper.findAll('.gutter.live')[0]!.trigger('click')
    expect(wrapper.findAll('.gutter.lit')).toHaveLength(3)
    expect(wrapper.find('.before.was-at').find('.before-head').exists()).toBe(false)
  })

  it('lights the whole run on hover of any of its lines', async () => {
    const wrapper = show()
    await wrapper.findAll('.gutter.live')[2]!.trigger('mouseenter')
    expect(wrapper.findAll('.gutter.lit')).toHaveLength(3)
    await wrapper.findAll('.gutter.live')[2]!.trigger('mouseleave')
    expect(wrapper.findAll('.gutter.lit')).toHaveLength(0)
  })

  it('closes on a second click of the same run', async () => {
    const wrapper = show()
    const bars = wrapper.findAll('.gutter.live')
    await bars[0]!.trigger('click')
    await bars[2]!.trigger('click')
    expect(wrapper.find('.before.was-at').exists()).toBe(false)
  })
})
