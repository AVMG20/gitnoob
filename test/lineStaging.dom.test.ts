// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import DiffView from '~/components/DiffView.vue'
import type { FileDiff } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

/**
 * One hunk with two changes in it: a replaced line near the top and another
 * further down. Picking one of them is the whole point of the feature.
 */
const diff: FileDiff = {
  path: 'a.txt',
  binary: false,
  truncated: 0,
  hunks: [
    {
      header: '@@ -1,4 +1,4 @@',
      lines: [
        { origin: ' ', old_lineno: 1, new_lineno: 1, content: 'one' },
        { origin: '-', old_lineno: 2, new_lineno: null, content: 'two' },
        { origin: '+', old_lineno: null, new_lineno: 2, content: 'TWO' },
        { origin: ' ', old_lineno: 3, new_lineno: 3, content: 'three' },
        { origin: '-', old_lineno: 4, new_lineno: null, content: 'four' },
        { origin: '+', old_lineno: null, new_lineno: 4, content: 'FOUR' }
      ]
    }
  ]
}

const show = (side: 'staged' | 'unstaged' | null = 'unstaged') =>
  mount(DiffView, { props: { diff, text: null, side, top: 0, view: 900, left: 0, width: 900 } })

/** The rows that are changes, in the order they are drawn. */
const changed = (wrapper: ReturnType<typeof show>) =>
  wrapper.findAll('.line').filter((line) => line.classes('pickable'))

const stageButton = (wrapper: ReturnType<typeof show>) =>
  wrapper.findAll('.hunk-btn').find((b) => b.text().startsWith('Stage'))!

describe('picking lines out of a hunk', () => {
  it('marks only the changed lines as pickable', () => {
    const wrapper = show()
    expect(changed(wrapper)).toHaveLength(4)
    // The context lines are not.
    expect(wrapper.findAll('.line.ctx.pickable')).toHaveLength(0)
  })

  it('picks nothing at all until a gutter is pressed', () => {
    const wrapper = show()
    expect(wrapper.findAll('.line.picked')).toHaveLength(0)
    expect(stageButton(wrapper).text()).toBe('Stage hunk')
  })

  it('picks a line when its gutter is pressed, and says how many', async () => {
    const wrapper = show()
    await changed(wrapper)[1]!.find('.no').trigger('mousedown')
    expect(wrapper.findAll('.line.picked')).toHaveLength(1)
    expect(stageButton(wrapper).text()).toBe('Stage 1 line')

    await changed(wrapper)[0]!.find('.sign').trigger('mousedown')
    expect(stageButton(wrapper).text()).toBe('Stage 2 lines')
  })

  it('leaves the code itself selectable as text', async () => {
    const wrapper = show()
    await changed(wrapper)[0]!.find('.text').trigger('mousedown')
    expect(wrapper.findAll('.line.picked')).toHaveLength(0)
  })

  it('sends only the picked lines, named by their numbers', async () => {
    const wrapper = show()
    // The `+TWO` line, which is line 2 of the new file.
    await changed(wrapper)[1]!.find('.no').trigger('mousedown')
    await stageButton(wrapper).trigger('click')

    expect(wrapper.emitted('hunk')?.[0]).toEqual([0, 'stage', { added: [2], removed: [] }])
  })

  it('names a removal by the line it used to be', async () => {
    const wrapper = show()
    // The `-two` line, which was line 2 of the old file.
    await changed(wrapper)[0]!.find('.no').trigger('mousedown')
    await stageButton(wrapper).trigger('click')
    expect(wrapper.emitted('hunk')?.[0]).toEqual([0, 'stage', { added: [], removed: [2] }])
  })

  it('asks for the whole hunk when nothing is picked', async () => {
    const wrapper = show()
    await stageButton(wrapper).trigger('click')
    expect(wrapper.emitted('hunk')?.[0]).toEqual([0, 'stage', undefined])
  })

  it('takes a line back out when its gutter is pressed again', async () => {
    const wrapper = show()
    await changed(wrapper)[1]!.find('.no').trigger('mousedown')
    await changed(wrapper)[1]!.find('.no').trigger('mousedown')
    expect(wrapper.findAll('.line.picked')).toHaveLength(0)
  })

  it('picks a run by dragging across it', async () => {
    const wrapper = show()
    await changed(wrapper)[0]!.find('.no').trigger('mousedown')
    await changed(wrapper)[1]!.trigger('mouseenter')
    await changed(wrapper)[2]!.trigger('mouseenter')
    expect(wrapper.findAll('.line.picked')).toHaveLength(3)
  })

  it('reaches back to the last one pressed with shift', async () => {
    const wrapper = show()
    await changed(wrapper)[0]!.find('.no').trigger('mousedown')
    await changed(wrapper)[3]!.find('.no').trigger('mousedown', { shiftKey: true })
    expect(wrapper.findAll('.line.picked')).toHaveLength(4)
  })

  it('offers to discard only what is picked, and to clear the picks', async () => {
    const wrapper = show()
    await changed(wrapper)[1]!.find('.no').trigger('mousedown')
    const discard = wrapper.findAll('.hunk-btn').find((b) => b.text().includes('Discard'))!
    expect(discard.text()).toBe('Discard lines')

    await wrapper.findAll('.hunk-btn').find((b) => b.text() === 'Clear')!.trigger('click')
    expect(wrapper.findAll('.line.picked')).toHaveLength(0)
    expect(stageButton(wrapper).text()).toBe('Stage hunk')
  })

  it('says unstage on the staged side', async () => {
    const wrapper = show('staged')
    await changed(wrapper)[1]!.find('.no').trigger('mousedown')
    expect(wrapper.findAll('.hunk-btn').some((b) => b.text() === 'Unstage 1 line')).toBe(true)
    // Nothing is discarded from the index.
    expect(wrapper.findAll('.hunk-btn').some((b) => b.text().includes('Discard'))).toBe(false)
  })

  it('picks nothing at all when a commit is being read rather than the working tree', () => {
    const wrapper = show(null)
    expect(wrapper.findAll('.line.pickable')).toHaveLength(0)
  })

  it('forgets the picks when the file is reloaded', async () => {
    const wrapper = show()
    await changed(wrapper)[1]!.find('.no').trigger('mousedown')
    expect(wrapper.findAll('.line.picked')).toHaveLength(1)

    await wrapper.setProps({ diff: { ...diff, hunks: [...diff.hunks] } })
    expect(wrapper.findAll('.line.picked')).toHaveLength(0)
  })
})
