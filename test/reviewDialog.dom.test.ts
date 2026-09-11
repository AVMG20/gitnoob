// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import AppModal from '~/components/AppModal.vue'
import PeoplePicker from '~/components/PeoplePicker.vue'
import ReviewDialog from '~/components/ReviewDialog.vue'
import SearchSelect from '~/components/SearchSelect.vue'
import { useForge } from '~/composables/useForge'
import { useGit, type LocalBranch } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const forge = useForge()

const branch = (name: string, is_head: boolean): LocalBranch => ({
  name,
  oid: name.padEnd(40, '0'),
  is_head,
  upstream: `origin/${name}`,
  ahead: 0,
  behind: 0
})

const show = () =>
  mount(ReviewDialog, { global: { components: { AppModal, SearchSelect, PeoplePicker } } })

/** Types into the description and the title, the way a person fills the form. */
async function fill(wrapper: ReturnType<typeof show>, title: string, body: string) {
  await wrapper.find('input[type="text"]').setValue(title)
  await wrapper.find('textarea').setValue(body)
}

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    if (cmd === 'forge_create_review') return { number: 7, title: 'x', url: 'https://x', warning: null }
    if (cmd === 'forge_members') return []
    if (cmd === 'ai_status') return { configured: false, model: null, default_commit_prompt: '' }
    return null
  })
  git.store.repo = { path: '/repo', name: 'repo', head: 'tickets', detached: false } as never
  git.store.refs = {
    locals: [branch('tickets', true), branch('main', false)],
    remotes: [],
    tags: [],
    stashes: []
  }
  git.store.rows = []
  git.store.log = []
  forge.store.status = { kind: 'github', has_token: true, slug: 'me/repo' } as never
  forge.store.draft = null
  forge.store.draftFor = null
})

/**
 * The half-written review survives the dialog closing by ✕ or Escape, and
 * only goes when it is cancelled, created, or handed to the forge — one stray
 * click used to cost the whole description.
 */
describe('the new review dialog', () => {
  it('brings back what was typed after closing with ✕', async () => {
    const first = show()
    await flushPromises()
    await fill(first, 'Add the parser', 'It parses things.')
    await first.find('.head .btn').trigger('click')
    expect(first.emitted('close')).toHaveLength(1)
    first.unmount()

    const again = show()
    await flushPromises()
    expect((again.find('input[type="text"]').element as HTMLInputElement).value).toBe('Add the parser')
    expect((again.find('textarea').element as HTMLTextAreaElement).value).toBe('It parses things.')
    again.unmount()
  })

  it('brings it back after Escape too', async () => {
    const first = show()
    await flushPromises()
    await fill(first, 'Add the parser', 'It parses things.')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await flushPromises()
    expect(first.emitted('close')).toHaveLength(1)
    first.unmount()

    const again = show()
    await flushPromises()
    expect((again.find('textarea').element as HTMLTextAreaElement).value).toBe('It parses things.')
    again.unmount()
  })

  it('throws it away on Cancel', async () => {
    const first = show()
    await flushPromises()
    await fill(first, 'Add the parser', 'It parses things.')
    const cancel = first.findAll('.footer .btn').find((b) => b.text() === 'Cancel')!
    await cancel.trigger('click')
    expect(first.emitted('close')).toHaveLength(1)
    first.unmount()

    const again = show()
    await flushPromises()
    expect((again.find('textarea').element as HTMLTextAreaElement).value).toBe('')
    again.unmount()
  })

  it('throws it away once the review is created', async () => {
    const first = show()
    await flushPromises()
    await fill(first, 'Add the parser', 'It parses things.')
    const create = first.findAll('.footer .btn').find((b) => b.text() === 'Create')!
    await create.trigger('click')
    await flushPromises()
    expect(asked.mock.calls.some(([cmd]) => cmd === 'forge_create_review')).toBe(true)
    expect(first.emitted('close')).toHaveLength(1)
    expect(forge.store.draft).toBeNull()
    first.unmount()
  })

  it('does not hand another repository the draft', async () => {
    const first = show()
    await flushPromises()
    await fill(first, 'Add the parser', 'It parses things.')
    await first.find('.head .btn').trigger('click')
    first.unmount()

    git.store.repo = { path: '/other', name: 'other', head: 'tickets', detached: false } as never
    const again = show()
    await flushPromises()
    expect((again.find('textarea').element as HTMLTextAreaElement).value).toBe('')
    again.unmount()
  })

  it('keeps nothing when nothing was written', async () => {
    const first = show()
    await flushPromises()
    await first.find('.head .btn').trigger('click')
    first.unmount()
    expect(forge.store.draft).toBeNull()
  })
})
