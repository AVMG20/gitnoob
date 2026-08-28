// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SettingsModal from '~/components/SettingsModal.vue'
import AppModal from '~/components/AppModal.vue'
import { useConfig } from '~/composables/useConfig'
import { useAi } from '~/composables/useAi'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const config = useConfig()
const ai = useAi()
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const DEFAULT = 'Line 1 is the message, and usually the whole of it.'

/** A config as the backend hands it over, with whatever prompt is stored. */
function stored(prompt: string | null) {
  return {
    version: 1,
    active_profile: null,
    global: {
      show_avatars: true,
      ai: {
        model: 'anthropic/claude-sonnet-4.5',
        max_tokens: 1500,
        reasoning: 'off',
        commit_style: 'plain',
        commit_prompt: prompt
      }
    },
    profiles: []
  }
}

beforeEach(async () => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'config_get' || cmd === 'config_set_global') {
      const patched = (args?.global ?? null) as { ai?: { commit_prompt?: string | null } } | null
      return stored(patched ? (patched.ai?.commit_prompt ?? null) : current)
    }
    if (cmd === 'ai_status') {
      return { configured: true, model: 'a/b', default_commit_prompt: DEFAULT }
    }
    if (cmd === 'ai_models') return []
    return null
  })
  current = null
  config.store.settingsSection = 'ai'
  await config.load()
  await ai.refreshStatus()
})

let current: string | null = null

async function open() {
  const wrapper = mount(SettingsModal, { global: { components: { AppModal } } })
  await flushPromises()
  return wrapper
}

const box = (wrapper: Awaited<ReturnType<typeof open>>) => wrapper.find('textarea.prompt')

describe('the commit message instructions', () => {
  it('are a box to write in, not a choice between two of somebody else’s', async () => {
    const wrapper = await open()
    expect(box(wrapper).exists()).toBe(true)
    // The dropdown it replaced is gone.
    const options = wrapper.findAll('option').map((one) => one.attributes('value'))
    expect(options).not.toContain('conventional')
  })

  it('start filled with the default the backend sent', async () => {
    const wrapper = await open()
    expect((box(wrapper).element as HTMLTextAreaElement).value).toBe(DEFAULT)
  })

  it('show what is stored when something has been written', async () => {
    current = 'Only ever write one line.'
    await config.load()
    const wrapper = await open()
    expect((box(wrapper).element as HTMLTextAreaElement).value).toBe('Only ever write one line.')
  })

  it('are saved when the box loses focus, not on every keystroke', async () => {
    const wrapper = await open()
    await box(wrapper).setValue('Write it in haiku.')
    expect(calls.some((call) => call.cmd === 'config_set_global')).toBe(false)

    await box(wrapper).trigger('blur')
    await flushPromises()
    const saved = calls.find((call) => call.cmd === 'config_set_global')
    expect((saved!.args.global as { ai: { commit_prompt: string } }).ai.commit_prompt).toBe(
      'Write it in haiku.'
    )
  })

  it('store nothing at all when the box is left at the default', async () => {
    current = 'Only ever write one line.'
    await config.load()
    const wrapper = await open()
    await box(wrapper).setValue(DEFAULT)
    await box(wrapper).trigger('blur')
    await flushPromises()

    // A copy of the default would go stale the moment the default moved.
    const saved = calls.find((call) => call.cmd === 'config_set_global')
    expect((saved!.args.global as { ai: { commit_prompt: null } }).ai.commit_prompt).toBeNull()
  })

  it('store nothing when the box is emptied, so the default comes back', async () => {
    current = 'Only ever write one line.'
    await config.load()
    const wrapper = await open()
    await box(wrapper).setValue('   ')
    await box(wrapper).trigger('blur')
    await flushPromises()
    const saved = calls.find((call) => call.cmd === 'config_set_global')
    expect((saved!.args.global as { ai: { commit_prompt: null } }).ai.commit_prompt).toBeNull()
  })

  it('save nothing when the box was opened and left alone', async () => {
    const wrapper = await open()
    await box(wrapper).trigger('blur')
    await flushPromises()
    expect(calls.some((call) => call.cmd === 'config_set_global')).toBe(false)
  })

  it('offer the default back only once something else is in the box', async () => {
    const wrapper = await open()
    expect(wrapper.find('.link').exists()).toBe(false)

    await box(wrapper).setValue('Write it in haiku.')
    await flushPromises()
    expect(wrapper.find('.link').text()).toContain('Put the default back')

    await wrapper.find('.link').trigger('click')
    await flushPromises()
    expect((box(wrapper).element as HTMLTextAreaElement).value).toBe(DEFAULT)
  })
})
