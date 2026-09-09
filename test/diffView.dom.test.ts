// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import DiffView from '~/components/DiffView.vue'
import type { FileDiff } from '~/composables/useGit'
import { ready } from '~/composables/useHighlight'

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
  it('says a pure move is a move, not "no changes"', () => {
    const moved: FileDiff = {
      path: 'b/f.txt',
      from: 'a/f.txt',
      binary: false,
      truncated: 0,
      hunks: []
    }
    const wrapper = mount(DiffView, {
      props: { diff: moved, side: 'staged', top: 0, view: 600, left: 0, width: 500 }
    })
    expect(wrapper.text()).toContain('Moved from a/f.txt')
    expect(wrapper.text()).not.toContain('No changes in this file')
  })

  it('still says "no changes" for an empty diff that moved nowhere', () => {
    const still: FileDiff = { path: 'a.ts', binary: false, truncated: 0, hunks: [] }
    const wrapper = mount(DiffView, {
      props: { diff: still, side: 'unstaged', top: 0, view: 600, left: 0, width: 500 }
    })
    expect(wrapper.text()).toContain('No changes in this file')
  })

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

/**
 * A `.vue` patch that changes a line inside the `<script>` block.
 *
 * This is the case the old highlighter could not reach. The `+` line was
 * coloured from the file on disk and came out as TypeScript; the `-` line is
 * not in that file at all, so it fell back to being read on its own — and on
 * its own it is template text, because the `<script>` tag that makes it
 * TypeScript is five lines above and not in what was passed.
 */
const SFC = [
  '<template>',
  '  <p>{{ label }}</p>',
  '</template>',
  '',
  '<script setup lang="ts">',
  "const label = ref('after')",
  '</script>'
].join('\n')

const sfcDiff: FileDiff = {
  path: 'app/components/Thing.vue',
  binary: false,
  truncated: 0,
  hunks: [
    {
      header: '@@ -5,3 +5,3 @@',
      lines: [
        { origin: ' ', old_lineno: 5, new_lineno: 5, content: '<script setup lang="ts">' },
        { origin: '-', old_lineno: 6, new_lineno: null, content: "const label = ref('before')" },
        { origin: '+', old_lineno: null, new_lineno: 6, content: "const label = ref('after')" },
        { origin: ' ', old_lineno: 7, new_lineno: 7, content: '</script>' }
      ]
    }
  ]
}

/** How many tokens on this row got a colour of their own. */
const colours = (html: string) => (html.match(/<span style="color:/g) ?? []).length

describe('colouring a single-file component', () => {
  it('reads a deleted line as TypeScript, not as template text', async () => {
    await ready()
    const wrapper = mount(DiffView, {
      props: { diff: sfcDiff, text: SFC, top: 0, view: 600, left: 0, width: 900 }
    })
    const rows = wrapper.findAll('.line')
    const removed = rows.find((row) => row.classes().includes('del'))
    const added = rows.find((row) => row.classes().includes('add'))
    expect(removed).toBeDefined()
    expect(added).toBeDefined()

    const removedHtml = removed!.find('.text').html()
    // The line it replaced is coloured to the same degree, which is what says
    // the old side was rebuilt and read in context rather than line by line.
    expect(colours(removedHtml)).toBe(colours(added!.find('.text').html()))
    expect(colours(removedHtml)).toBeGreaterThan(1)
    expect(removedHtml).toContain('before')
  })

  it('leaves the deleted line plain when the file on disk has moved on', async () => {
    await ready()
    // The diff says line 5 is the script tag; this file disagrees, so the
    // rebuild is of some other version and is dropped.
    const stale = SFC.replace('<script setup lang="ts">', '<script>')
    const wrapper = mount(DiffView, {
      props: { diff: sfcDiff, text: stale, top: 0, view: 600, left: 0, width: 900 }
    })
    const removed = wrapper.findAll('.line').find((row) => row.classes().includes('del'))
    expect(removed!.text()).toContain('before')
  })
})
