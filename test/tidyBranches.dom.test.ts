// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import TidyDialog from '~/components/TidyDialog.vue'
import AppModal from '~/components/AppModal.vue'
import Toasts from '~/components/Toasts.vue'
import { useGit, type Tidy } from '~/composables/useGit'
import { useToasts } from '~/composables/useToasts'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let answers: Record<string, unknown> = {}
const mounted: ReturnType<typeof mount>[] = []

const tidy = (over: Partial<Tidy>): Tidy => ({
  name: 'x',
  oid: '1111111aaaa',
  reason: 'merged',
  into: 'origin/main',
  gone: null,
  proven: true,
  ...over
})

beforeEach(() => {
  calls = []
  answers = {}
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return answers[cmd] ?? null
  })
  useToasts().clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.tidy = [
    tidy({ name: 'done', oid: 'aaaaaaa1' }),
    tidy({ name: 'squashed', oid: 'bbbbbbb2', reason: 'squash' }),
    tidy({ name: 'closed', reason: 'gone', gone: 'origin/closed', into: null, proven: false })
  ]
})

afterEach(() => {
  while (mounted.length) mounted.pop()!.unmount()
  useToasts().clear()
})

const show = <T>(component: T) => {
  const wrapper = mount(component as never, {
    global: { components: { AppModal }, stubs: { Teleport: true } }
  })
  mounted.push(wrapper)
  return wrapper
}

describe('tidying finished branches', () => {
  it('starts with only the proven branches ticked, each saying why', () => {
    const wrapper = show(TidyDialog)
    const rows = wrapper.findAll('.item')
    expect(rows.map((row) => row.find('input').element.checked)).toEqual([true, true, false])
    expect(rows[1]!.text()).toMatch(/squash-merged into origin\/main/)
    expect(rows[2]!.text()).toMatch(/origin\/closed was deleted/)
    expect(wrapper.find('.btn-primary').text()).toBe('Delete 2 branches')
  })

  it('names each branch in full on hover, since a long one is cut short', () => {
    const wrapper = show(TidyDialog)
    const title = wrapper.findAll('.item label')[1]!.attributes('title')!
    expect(title.split('\n')[0]).toBe('squashed')
    expect(title).toMatch(/squash-merged into origin\/main/)
  })

  it('needs an explicit tick for work that is on no trunk, and warns once ticked', async () => {
    const wrapper = show(TidyDialog)
    await wrapper.findAll('.item input')[2]!.setValue(true)
    expect(wrapper.find('.warn').text()).toMatch(/on no main/)
    expect(wrapper.find('.btn-danger').text()).toBe('Delete 3 branches')
  })

  it('deletes the ticked ones at the commits it showed, and offers them back', async () => {
    answers.delete_branches = {
      removed: [
        { name: 'done', oid: 'aaaaaaa1', remote: null, merge: null },
        { name: 'squashed', oid: 'bbbbbbb2', remote: 'origin', merge: 'squashed' }
      ],
      refused: [],
      step: 7
    }
    const toasts = show(Toasts)
    const wrapper = show(TidyDialog)
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    // Each with the commit the list showed, so one that moved since is refused.
    expect(calls.find((c) => c.cmd === 'delete_branches')?.args.branches).toEqual([
      { name: 'done', oid: 'aaaaaaa1' },
      { name: 'squashed', oid: 'bbbbbbb2' }
    ])
    expect(git.store.tidy.map((one) => one.name)).toEqual(['closed'])
    expect(toasts.text()).toMatch(/Deleted 2 branches/)

    await toasts.findAll('[data-testid="toast-action"]').find((b) => b.text() === 'Undo')!.trigger('click')
    await flushPromises()
    // The notice undoes the history's own step, not a copy of it.
    expect(calls.find((c) => c.cmd === 'undo_step')?.args.id).toBe(7)
    expect(calls.some((c) => c.cmd === 'restore_branches')).toBe(false)
  })
})

describe('the fetch nobody asked for', () => {
  it('locks nothing while it runs and stays quiet when it fails', async () => {
    let finish: (value: unknown) => void = () => {}
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} })
      if (cmd === 'fetch_primary') return new Promise((resolve) => (finish = resolve))
      return null
    })
    const running = git.fetchQuietly()
    await Promise.resolve()
    expect(git.store.busy).toBe(false)
    finish({ argv: [], ok: false, code: 1, stdout: '', stderr: 'could not reach origin' })
    expect(await running).toBe(false)
    expect(useToasts().items.value).toEqual([])
    expect(git.store.log[0]?.text).toMatch(/could not reach origin/)
    expect(calls.some((c) => c.cmd === 'fetch')).toBe(false)
  })
})
