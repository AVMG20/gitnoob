// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import RewriteDialog from '~/components/RewriteDialog.vue'
import AppModal from '~/components/AppModal.vue'
import { useRewrite } from '~/composables/useRewrite'

/**
 * The question asked before a fixup, a split, a new message or an absorb:
 * what it would do is on screen before the button that does it, a pushed
 * commit needs a yes of its own, and the absorb plan is shown hunk by hunk.
 */

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
let calls: { cmd: string; args: Record<string, unknown> }[] = []
let check: Record<string, unknown> = {}

const PLAN = {
  fingerprint: 'abc 123',
  refusal: null,
  kept_files: ['new.txt'],
  hunks: [
    {
      path: 'f.txt',
      header: '@@ -2,1 +2,1 @@',
      preview: ['-line 2', '+line two'],
      target: { oid: 'a'.repeat(40), short: 'aaaaaaa', summary: 'Add the parser' },
      why: null
    },
    {
      path: 'f.txt',
      header: '@@ -5,1 +5,1 @@',
      preview: ['-line 5', '+line five'],
      target: null,
      why: 'Those lines come from commits already pushed or older than this branch'
    }
  ]
}

beforeEach(() => {
  calls = []
  check = {
    oid: 'b'.repeat(40),
    short: 'bbbbbbb',
    summary: 'Add b',
    message: 'Add b\n\nBecause.',
    pushed: false,
    is_head: false,
    above: 2,
    staged: 1,
    refusal: null
  }
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'rewrite_check') return check
    if (cmd === 'absorb_plan') return PLAN
    if (cmd === 'rebase_progress') return null
    if (cmd.startsWith('fixup') || cmd === 'absorb' || cmd.startsWith('split') || cmd.startsWith('reword'))
      return 'done'
    return null
  })
})

async function show(dialog: Parameters<ReturnType<typeof useRewrite>['open']>[0]) {
  useRewrite().open(dialog)
  const wrapper = mount(RewriteDialog, { global: { components: { AppModal } } })
  await flushPromises()
  return wrapper
}

describe('the rewrite dialog', () => {
  it('folds the staged changes into the commit once asked', async () => {
    const wrapper = await show({ kind: 'fixup', oid: 'b'.repeat(40) })
    expect(wrapper.text()).toContain('Add b')
    expect(wrapper.text()).toContain('2 commits sit above it')
    await wrapper.find('[data-test="apply"]').trigger('click')
    await flushPromises()
    const call = calls.find((one) => one.cmd === 'fixup_commit')
    expect(call?.args).toMatchObject({ fold: true, allowPushed: false })
  })

  it('wants its own yes before rewriting a pushed commit', async () => {
    check.pushed = true
    const wrapper = await show({ kind: 'split', oid: 'b'.repeat(40) })
    const apply = wrapper.find('[data-test="apply"]')
    expect((apply.element as HTMLButtonElement).disabled).toBe(true)
    await wrapper.find('[data-test="pushed"] input').setValue(true)
    expect((apply.element as HTMLButtonElement).disabled).toBe(false)
    await apply.trigger('click')
    await flushPromises()
    expect(calls.find((one) => one.cmd === 'split_start')?.args.allowPushed).toBe(true)
  })

  it('says that abandoning a split keeps what was changed during it', async () => {
    const wrapper = await show({ kind: 'split', oid: 'b'.repeat(40) })
    expect(wrapper.text()).toMatch(/anything you changed on the way is kept in the stash/)
    expect(wrapper.text()).not.toMatch(/Abort puts everything back/)
  })

  it('shows a refusal instead of a button that would fail', async () => {
    check.refusal = 'There is a merge commit in the way'
    const wrapper = await show({ kind: 'reword', oid: 'b'.repeat(40) })
    expect(wrapper.find('[data-test="refusal"]').text()).toContain('merge commit')
    expect((wrapper.find('[data-test="apply"]').element as HTMLButtonElement).disabled).toBe(true)
  })

  it('starts the message box from the message the commit has', async () => {
    const wrapper = await show({ kind: 'reword', oid: 'b'.repeat(40) })
    const box = wrapper.find('[data-test="message"]').element as HTMLTextAreaElement
    expect(box.value).toBe('Add b\n\nBecause.')
  })

  it('lays out the absorb plan hunk by hunk before doing anything', async () => {
    const wrapper = await show({ kind: 'absorb' })
    const plan = wrapper.find('[data-test="plan"]')
    expect(plan.text()).toContain('Add the parser')
    expect(plan.text()).toContain('stays staged')
    expect(plan.text()).toContain('already pushed')
    expect(wrapper.text()).toContain('new.txt')
    expect(calls.some((one) => one.cmd === 'absorb')).toBe(false)

    await wrapper.find('[data-test="fold"]').setValue(false)
    await wrapper.find('[data-test="apply"]').trigger('click')
    await flushPromises()
    expect(calls.find((one) => one.cmd === 'absorb')?.args).toEqual({
      fingerprint: 'abc 123',
      fold: false
    })
  })
})
