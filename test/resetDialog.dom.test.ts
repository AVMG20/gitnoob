// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import AppModal from '~/components/AppModal.vue'
import ResetDialog from '~/components/ResetDialog.vue'
import { useGit, type ResetPreview } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const preview = (over: Partial<ResetPreview> = {}): ResetPreview => ({
  target: 'a'.repeat(40),
  short: '015dbe4',
  summary: 'Auto-save ticket message drafts per agent',
  branch: 'tickets',
  dropped: [
    {
      oid: 'b'.repeat(40),
      short: 'bbbbbbb',
      summary: 'Half a refactor',
      author: 'Robin Vale',
      time: Math.floor(Date.now() / 1000) - 600
    } as never
  ],
  diverges: false,
  staged_files: 2,
  unstaged_files: 50,
  ...over
})

function answering(given: ResetPreview) {
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'reset_preview') return given
    if (cmd === 'reset') return 'Moved tickets to 015dbe4'
    return null
  })
}

const show = () =>
  mount(ResetDialog, { props: { oid: 'a'.repeat(40) }, global: { components: { AppModal } } })

beforeEach(() => {
  calls = []
  asked.mockReset()
  answering(preview())
  git.store.log = []
})

/**
 * The reset question, which is only ever about a hard one: soft and mixed run
 * on the click that asked for them.
 */
describe('the reset dialog', () => {
  it('asks about a hard reset without offering the other two', async () => {
    const wrapper = await show()
    await flushPromises()

    expect(wrapper.findAll('.mode')).toHaveLength(0)
    expect(wrapper.text()).not.toContain('Soft')
    expect(wrapper.text()).not.toContain('Mixed')
    expect(wrapper.find('.btn-danger').text()).toBe('Hard reset')
  })

  it('says what is at stake once, not twice', async () => {
    const wrapper = show()
    await flushPromises()

    // The tick is the warning; a banner saying the same thing above it was the
    // sentence people stopped reading.
    const said = wrapper.text().match(/uncommitted/g) ?? []
    expect(said).toHaveLength(1)
    expect(wrapper.find('.ack').text()).toContain('Throw away 52 uncommitted changes')
  })

  it('will not reset until the work on disk has been acknowledged', async () => {
    const wrapper = show()
    await flushPromises()

    expect(wrapper.find('.btn-danger').attributes('disabled')).toBeDefined()
    await wrapper.find('input[type="checkbox"]').setValue(true)
    expect(wrapper.find('.btn-danger').attributes('disabled')).toBeUndefined()

    await wrapper.find('.btn-danger').trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'reset')?.args).toEqual({
      oid: 'a'.repeat(40),
      mode: 'hard'
    })
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('asks nothing extra when there is no uncommitted work to lose', async () => {
    answering(preview({ staged_files: 0, unstaged_files: 0 }))
    const wrapper = show()
    await flushPromises()

    expect(wrapper.find('.ack').exists()).toBe(false)
    expect(wrapper.find('.btn-danger').attributes('disabled')).toBeUndefined()
  })

  it('names the commits that would leave the branch, and that undo brings them back', async () => {
    const wrapper = show()
    await flushPromises()

    expect(wrapper.find('.block-head').text()).toContain('1 commit leaves tickets')
    expect(wrapper.find('.block-head').text()).toContain('Undo brings them back')
    expect(wrapper.findAll('.commits li')).toHaveLength(1)
  })

  it('says when the branch would move sideways rather than back', async () => {
    answering(preview({ diverges: true }))
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.note').text()).toContain('not on tickets')
  })
})
