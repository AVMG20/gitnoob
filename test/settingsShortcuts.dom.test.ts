// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SettingsModal from '~/components/SettingsModal.vue'
import AppModal from '~/components/AppModal.vue'
import { useConfig } from '~/composables/useConfig'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const config = useConfig()

describe('the shortcuts page', () => {
  it('lists a shortcut with no key by default, ready to be given one', async () => {
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'config_get') return { version: 1, active_profile: null, global: { ai: {} }, profiles: [] }
      return null
    })
    await config.load()
    config.store.settingsSection = 'shortcuts'
    const wrapper = mount(SettingsModal, { global: { components: { AppModal } } })
    await flushPromises()
    const row = wrapper.findAll('.keys-row').find((one) => one.text().includes('Next match in the file'))
    expect(row).toBeTruthy()
    expect(row!.find('kbd').text()).toBe('no key')
    expect(row!.find('.rebind button').text()).toBe('Change')
    wrapper.unmount()
  })
})
