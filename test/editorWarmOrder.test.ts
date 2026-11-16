import { describe, expect, it } from 'vitest'
import { EditorState } from '@codemirror/state'
import {
  WARM_MAX,
  acquire,
  clearWarm,
  isWarm,
  release,
  type Held,
  type Holder
} from '~/composables/editorBuffers'

const REPO = '/repos/shop'

const holder = (): Holder => ({ lend: () => null, reclaim: () => undefined })

const held = (path: string): Held =>
  ({
    path,
    state: EditorState.create({ doc: 'x' }),
    scroll: null,
    base: { text: 'x', hash: 'h', shape: null },
    reason: null,
    editorconfig: null
  }) as unknown as Held

/** Opens a tab's editor and closes it again, which parks its state. */
function visit(tab: string) {
  const one = holder()
  acquire(REPO, tab, one)
  release(REPO, tab, one, held(tab))
}

describe('the warm tabs', () => {
  it('drops the one left longest ago, not the one opened first', () => {
    clearWarm(REPO)
    // Opened first and kept open all afternoon while others come and go.
    const first = holder()
    acquire(REPO, 'first', first)
    for (let at = 0; at < WARM_MAX; at++) visit(`tab${at}`)
    // Left last: it is the most recent now.
    release(REPO, 'first', first, held('first'))
    visit('one-more')
    expect(isWarm('first', REPO)).toBe(true)
    expect(isWarm('tab0', REPO)).toBe(false)
    clearWarm(null)
  })
})
