// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import ConflictView from '~/components/ConflictView.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import ChangeRuler from '~/components/ChangeRuler.vue'
import Spinner from '~/components/Spinner.vue'
import { useGit, type Resolution } from '~/composables/useGit'

/**
 * The conflict resolver against a fixture file.
 *
 * The file is deliberately lopsided — our side is shorter in the first region
 * and longer in the second — because that is the case the panes used to get
 * wrong: the two sides of one conflict ended up on different parts of the
 * screen. Every backend call is answered here, and the resolutions the view
 * sends are recorded, so what a click means can be asserted on rather than
 * guessed at from the pixels.
 */

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const FILE = {
  path: 'src/app.ts',
  conflict_count: 2,
  stages: { base: true, ours: true, theirs: true },
  blocks: [
    { kind: 'context', lines: ['import fs from "fs"', ''] },
    {
      kind: 'conflict',
      index: 0,
      ours: ['const port = 3000'],
      theirs: ['const port = 8080', 'const host = "0.0.0.0"'],
      base: [],
      has_base: false,
      ours_label: 'HEAD',
      theirs_label: 'feature/ports'
    },
    { kind: 'context', lines: ['', 'start(port)'] },
    {
      kind: 'conflict',
      index: 1,
      ours: ['log("ours")', 'log("again")'],
      theirs: ['log("theirs")'],
      base: [],
      has_base: false,
      ours_label: 'HEAD',
      theirs_label: 'feature/ports'
    },
    { kind: 'context', lines: ['export {}'] }
  ]
}

let calls: { cmd: string; args: Record<string, unknown> }[] = []

const asked = vi.mocked(invoke)

/** The same walk the Rust side does, so the result pane can be asserted on. */
function preview(chosen: Resolution[]) {
  const out: string[] = []
  for (const block of FILE.blocks) {
    if (block.kind === 'context') {
      out.push(...(block.lines as string[]))
      continue
    }
    const choice = chosen[block.index as number]
    if (choice?.custom) {
      out.push(...choice.custom)
      continue
    }
    const sides = choice?.ours_first ?? true ? ['ours', 'theirs'] : ['theirs', 'ours']
    for (const side of sides) {
      const take = side === 'ours' ? choice?.take_ours ?? true : choice?.take_theirs ?? false
      if (take) out.push(...((block as Record<string, string[]>)[side] ?? []))
    }
  }
  return out.join('\n')
}

function answer(cmd: string, args: Record<string, unknown>) {
  if (cmd === 'conflict_read') return FILE
  if (cmd === 'conflict_preview') return preview((args.choices ?? []) as Resolution[])
  return null
}

const choices = () => {
  const last = [...calls].reverse().find((call) => call.cmd === 'conflict_preview')
  return (last?.args.choices ?? []) as {
    take_ours: boolean
    take_theirs: boolean
    ours_first: boolean
    custom: string[] | null
  }[]
}

/** The resolver and the menu renderer, which is how the app mounts the two. */
const Host = {
  components: { ConflictView, ContextMenu },
  template: '<div><ConflictView /><ContextMenu /></div>'
}

async function open() {
  const git = useGit()
  git.store.status = { staged: [], unstaged: [], conflicted: ['src/app.ts'] }
  const wrapper = mount(Host, { global: { components: { ChangeRuler, Spinner } } })
  await flushPromises()
  await flushPromises()
  return wrapper
}

/** Opens one of the bulk menus and picks the row whose label starts with `label`. */
async function pick(
  wrapper: Awaited<ReturnType<typeof open>>,
  opener: string,
  label: string
) {
  await wrapper.findAll('button').find((one) => one.text().includes(opener))!.trigger('click')
  await flushPromises()
  const row = wrapper.findAll('.menu .item').find((one) => one.text().includes(label))
  await row!.trigger('click')
  await flushPromises()
}

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return answer(cmd, args ?? {})
  })
  const git = useGit()
  git.store.status = null
  git.store.resolving = null
})

describe('the conflict resolver', () => {
  it('shows both sides of every region, lined up row for row', async () => {
    const wrapper = await open()
    const panes = wrapper.findAll('.pane')
    expect(panes).toHaveLength(2)

    // One head per region in each pane, so a region is answerable from either.
    expect(panes[0]!.findAll('.head')).toHaveLength(2)
    expect(panes[1]!.findAll('.head')).toHaveLength(2)

    // The sides are different lengths, and the shorter one is padded rather
    // than closed up: two holes, one in each pane.
    expect(panes[0]!.findAll('.filler')).toHaveLength(1)
    expect(panes[1]!.findAll('.filler')).toHaveLength(1)

    // Both panes draw the same number of rows, which is what makes one scroll
    // position mean the same place in each.
    expect(panes[0]!.findAll('.row:not(.gauge)')).toHaveLength(
      panes[1]!.findAll('.row:not(.gauge)').length
    )
  })

  it('numbers each side as that side of the file', async () => {
    const wrapper = await open()
    const nums = (pane: number) =>
      wrapper
        .findAll('.pane')[pane]!
        .findAll('.row:not(.gauge) .no')
        .map((cell) => cell.text())
    // Ours: two lines of context, one of the region, two more, two, one.
    expect(nums(0)).toEqual(['1', '2', '3', '', '4', '5', '6', '7', '8'])
    // Theirs runs one line longer through the first region and one shorter
    // through the second, and says so.
    expect(nums(1)).toEqual(['1', '2', '3', '4', '5', '6', '7', '', '8'])
  })

  it('starts on our side and says nothing has been decided yet', async () => {
    const wrapper = await open()
    expect(wrapper.text()).toContain('2 conflicts')
    expect(wrapper.text()).toContain('2 left to look at')
    expect(choices()).toEqual([
      { take_ours: true, take_theirs: false, ours_first: true, custom: null },
      { take_ours: true, take_theirs: false, ours_first: true, custom: null }
    ])
  })

  it('takes a whole side from the head checkbox', async () => {
    const wrapper = await open()
    const theirs = wrapper.findAll('.pane')[1]!
    await theirs.findAll('.head input')[0]!.trigger('change')
    await flushPromises()

    // Both sides now, and the region counts as decided.
    expect(choices()[0]).toEqual({
      take_ours: true,
      take_theirs: true,
      ours_first: true,
      custom: null
    })
    expect(wrapper.text()).toContain('1 left to look at')
  })

  it('picks single lines, and sends them spelled out', async () => {
    const wrapper = await open()
    const theirs = wrapper.findAll('.pane')[1]!
    // The second line of their first region only: a mix no checkbox can say.
    await theirs.findAll('.line-box')[1]!.trigger('change')
    await flushPromises()

    expect(choices()[0]!.custom).toEqual(['const port = 3000', 'const host = "0.0.0.0"'])
    expect(wrapper.text()).toContain('picked line by line')
  })

  it('drops a region when neither side is wanted', async () => {
    const wrapper = await open()
    const ours = wrapper.findAll('.pane')[0]!
    await ours.findAll('.head input')[0]!.trigger('change')
    await flushPromises()

    expect(choices()[0]).toEqual({
      take_ours: false,
      take_theirs: false,
      ours_first: true,
      custom: null
    })
    expect(wrapper.text()).toContain('1 dropped')
  })

  it('walks the regions with the arrow keys', async () => {
    const wrapper = await open()
    expect(wrapper.find('.place').text()).toContain('1 of 2')

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }))
    await flushPromises()
    expect(wrapper.find('.place').text()).toContain('2 of 2')

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp' }))
    await flushPromises()
    expect(wrapper.find('.place').text()).toContain('1 of 2')
    wrapper.unmount()
  })

  it('marks every region on the strip, and what has been decided about it', async () => {
    const wrapper = await open()
    const marks = wrapper.findAll('.panes .ruler .mark')
    expect(marks).toHaveLength(2)
    expect(marks.every((mark) => mark.classes('open'))).toBe(true)

    await wrapper.findAll('.pane')[1]!.findAll('.head input')[0]!.trigger('change')
    await flushPromises()
    expect(wrapper.findAll('.panes .ruler .mark')[0]!.classes()).toContain('settled')
  })

  it('says which lines of the result answered a conflict, and from which side', async () => {
    const wrapper = await open()
    const out = () => wrapper.find('.out-body')
    // Our side by default: one line from the first region, two from the second.
    expect(out().findAll('.from-ours')).toHaveLength(3)
    expect(out().findAll('.from-theirs')).toHaveLength(0)
    // And the strip beside it has a bar per region, not per line.
    expect(wrapper.findAll('.output .ruler .mark')).toHaveLength(2)

    await pick(wrapper, 'Every conflict', 'Take theirs in every conflict')
    expect(out().findAll('.from-ours')).toHaveLength(0)
    expect(out().findAll('.from-theirs')).toHaveLength(3)

    // A line-by-line pick still says which side each line came from.
    await wrapper.findAll('.pane')[0]!.findAll('.line-box')[0]!.trigger('change')
    await flushPromises()
    expect(out().findAll('.from-ours')).toHaveLength(1)
    expect(out().findAll('.from-theirs')).toHaveLength(3)
  })

  it('answers every conflict at once from the menu', async () => {
    const wrapper = await open()
    await pick(wrapper, 'Every conflict', 'Take theirs in every conflict')
    expect(choices()).toEqual([
      { take_ours: false, take_theirs: true, ours_first: true, custom: null },
      { take_ours: false, take_theirs: true, ours_first: true, custom: null }
    ])
    expect(wrapper.text()).toContain('all 2 decided')
  })

  it('writes the file the preview showed, then closes when nothing is left', async () => {
    const wrapper = await open()
    const git = useGit()
    await wrapper.findAll('button').find((one) => one.text() === 'Mark resolved')!.trigger('click')
    await flushPromises()

    const wrote = calls.find((call) => call.cmd === 'conflict_resolve')
    expect(wrote?.args.path).toBe('src/app.ts')
    expect(wrote?.args.choices).toEqual(choices())
    // The status still lists it — the watcher has not run — but the view has
    // let go of it, which is what closes the resolver.
    expect(git.store.resolving).toBe(null)
  })
})
