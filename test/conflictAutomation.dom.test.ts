// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import ConflictView from '~/components/ConflictView.vue'
import ConflictOverlay from '~/components/ConflictOverlay.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import ChangeRuler from '~/components/ChangeRuler.vue'
import Spinner from '~/components/Spinner.vue'
import { useGit, type InProgress } from '~/composables/useGit'

/**
 * What the resolver does on its own and what it keeps: trivial regions taken
 * up front, rerere's answer waiting to be accepted, lockfiles, reopening a
 * resolved file, choices that survive the window, and the way out of a pick
 * that stopped.
 */

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let files: Record<string, unknown> = {}
let resolved: string[] = []

const conflictFile = (path: string, trivial: string | null = null) => ({
  path,
  conflict_count: 1,
  stages: { base: true, ours: true, theirs: true },
  rerere: false,
  lockfile: path.endsWith('.lock'),
  blocks: [
    { kind: 'context', lines: ['a'] },
    {
      kind: 'conflict',
      index: 0,
      ours: ['fn(a, b)'],
      theirs: trivial ? ['fn(a,  b)'] : ['fn(b, a)'],
      base: ['fn(a)'],
      has_base: true,
      ours_label: 'HEAD',
      theirs_label: 'side',
      trivial
    },
    { kind: 'context', lines: ['z'] }
  ]
})

const progress = (patch: Partial<InProgress> = {}): InProgress => ({
  merging: true,
  rebasing: false,
  cherry_picking: false,
  reverting: false,
  restoring: false,
  applied_stash: null,
  prepared: null,
  returns_to: null,
  sequencer: null,
  sequence_left: 0,
  operation: 'merge abc',
  ...patch
})

beforeEach(() => {
  calls = []
  resolved = []
  files = { 'a.txt': conflictFile('a.txt') }
  try {
    localStorage.clear()
  } catch {
    // Not every environment has it; the resolver does not need it.
  }
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'conflict_read') return files[(args?.path as string) ?? '']
    if (cmd === 'conflict_preview') return 'preview'
    if (cmd === 'conflict_marked') return []
    if (cmd === 'conflict_resolved') return resolved
    return null
  })
  const git = useGit()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.progress = progress()
  git.store.resolving = null
})

async function open(conflicted = ['a.txt']) {
  useGit().store.status = { staged: [], unstaged: [], conflicted } as never
  const wrapper = mount(
    { components: { ConflictView, ContextMenu }, template: '<div><ConflictView /><ContextMenu /></div>' },
    { global: { components: { ChangeRuler, Spinner } } }
  )
  await flushPromises()
  await flushPromises()
  return wrapper
}

describe('regions that only look different', () => {
  it('takes ours up front and says so on the region', async () => {
    files = { 'a.txt': conflictFile('a.txt', 'whitespace') }
    const wrapper = await open()
    const chip = wrapper.find('[data-test="trivial-chip"]')
    expect(chip.exists()).toBe(true)
    expect(chip.text()).toContain('whitespace-only, took ours')
    // Decided, but nothing staged: the file still waits for Mark resolved.
    expect(wrapper.text()).toContain('all 1 decided')
    expect(calls.some((call) => call.cmd === 'conflict_resolve')).toBe(false)
  })

  it('leaves a real difference undecided', async () => {
    const wrapper = await open()
    expect(wrapper.find('[data-test="trivial-chip"]').exists()).toBe(false)
    expect(wrapper.text()).toContain('1 left to look at')
  })
})

describe('an answer remembered from last time', () => {
  it('waits to be accepted, and accepting stages the file as it stands', async () => {
    files = {
      'a.txt': {
        ...conflictFile('a.txt'),
        conflict_count: 0,
        rerere: true,
        blocks: [{ kind: 'context', lines: ['resolved'] }]
      }
    }
    const wrapper = await open()
    expect(wrapper.find('[data-test="rerere-chip"]').exists()).toBe(true)
    expect(wrapper.text()).toContain('Resolved the way you did last time')
    await wrapper.find('[data-test="accept-rerere"]').trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'conflict_resolve_as_is')?.args.path).toBe('a.txt')
  })
})

describe('lockfiles', () => {
  it('suggests taking a side and regenerating', async () => {
    files = { 'Cargo.lock': conflictFile('Cargo.lock') }
    const wrapper = await open(['Cargo.lock'])
    expect(wrapper.find('[data-test="lockfile-hint"]').text()).toContain('regenerate')
  })
})

describe('reopening a resolved file', () => {
  it('lists what was resolved and puts its conflict back on request', async () => {
    resolved = ['done.txt']
    files['done.txt'] = conflictFile('done.txt')
    const wrapper = await open()
    const row = wrapper.find('[data-test="resolved-file"]')
    expect(row.text()).toContain('done.txt')
    await row.find('[data-test="reopen"]').trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'conflict_reopen')?.args.path).toBe('done.txt')
  })
})

describe('choices that outlive the window', () => {
  it('come back for the same conflict in the same operation', async () => {
    const first = await open()
    // Take theirs from the head checkbox of the theirs pane.
    const theirs = first.findAll('.pane')[1]!.find('.head input.box')
    await theirs.setValue(true)
    await flushPromises()
    first.unmount()

    const again = await open()
    const box = again.findAll('.pane')[1]!.find('.head input.box').element as HTMLInputElement
    expect(box.checked).toBe(true)
    again.unmount()
  })

  it('are dropped once the operation is over', async () => {
    const first = await open()
    await first.findAll('.pane')[1]!.find('.head input.box').setValue(true)
    await flushPromises()
    first.unmount()

    // A different merge: what was chosen for the last one is gone.
    useGit().store.progress = progress({ operation: 'merge def' })
    const again = await open()
    const box = again.findAll('.pane')[1]!.find('.head input.box').element as HTMLInputElement
    expect(box.checked).toBe(false)
    again.unmount()
  })
})

describe('a pick that stopped', () => {
  it('offers skip and abort on the conflict sheet', async () => {
    const git = useGit()
    git.store.status = { staged: [], unstaged: [], conflicted: ['a.txt'] } as never
    git.store.progress = progress({
      merging: false,
      cherry_picking: true,
      sequencer: 'cherry-pick',
      sequence_left: 2,
      operation: 'sequence abc'
    })
    const wrapper = mount(ConflictOverlay, { global: { stubs: { ConflictView: true } } })
    const abort = wrapper.find('[data-test="abort-pick"]')
    expect(abort.text()).toContain('Abort cherry-pick')
    expect(wrapper.find('[data-test="skip-pick"]').exists()).toBe(true)
    await abort.trigger('click')
    await flushPromises()
    expect(calls.some((call) => call.cmd === 'sequence_abort')).toBe(true)
  })
})
