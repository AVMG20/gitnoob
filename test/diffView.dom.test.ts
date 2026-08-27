// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import DiffView from '~/components/DiffView.vue'
import type { FileDiff } from '~/composables/useGit'

/**
 * A patch with one hunk and one very long line, which is the case the heading
 * used to get wrong: the heading was as wide as the file, so the buttons on it
 * were at the end of that width rather than at the edge of the window.
 */
const diff: FileDiff = {
  path: 'src/app.ts',
  binary: false,
  truncated: 0,
  hunks: [
    {
      header: '@@ -1,2 +1,2 @@',
      lines: [
        { origin: ' ', old_lineno: 1, new_lineno: 1, content: 'x'.repeat(400) },
        { origin: '-', old_lineno: 2, new_lineno: null, content: 'gone' },
        { origin: '+', old_lineno: null, new_lineno: 2, content: 'here' },
        { origin: '\\', old_lineno: null, new_lineno: null, content: 'No newline at end of file' }
      ]
    }
  ]
}

const head = (wrapper: ReturnType<typeof mount>) =>
  wrapper.find('.hunk-head').attributes('style') ?? ''

describe('the diff view', () => {
  it('draws a hunk heading the width of the box, not of the file', () => {
    const wrapper = mount(DiffView, {
      props: { diff, side: 'unstaged', top: 0, view: 600, left: 0, width: 500 }
    })
    expect(head(wrapper)).toContain('width: 500px')
  })

  it('carries the heading along when the patch is scrolled sideways', async () => {
    const wrapper = mount(DiffView, {
      props: { diff, side: 'unstaged', top: 0, view: 600, left: 0, width: 500 }
    })
    await wrapper.setProps({ left: 320 })
    // The buttons sit at the right of the heading, so a heading that follows
    // the scroll is a pair of buttons that stay where they can be clicked.
    expect(head(wrapper)).toContain('translateX(320px)')
  })

  it('keeps the no-newline remark off both sides of the numbering', () => {
    const wrapper = mount(DiffView, {
      props: { diff, side: 'unstaged', top: 0, view: 600 }
    })
    const remark = wrapper.findAll('.line').find((one) => one.classes('eof'))
    expect(remark).toBeTruthy()
    expect(remark!.findAll('.no').every((one) => one.text() === '')).toBe(true)
    expect(remark!.find('.text').text()).toBe('No newline at end of file')
  })
})
