// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SettingsModal from '~/components/SettingsModal.vue'
import AppModal from '~/components/AppModal.vue'
import { useConfig } from '~/composables/useConfig'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const config = useConfig()

const PROFILE = {
  id: 'p1',
  name: 'Work',
  forge: 'github',
  host: 'github.com',
  projects: [],
  active_project: null
}

describe('saving a profile', () => {
  it('keeps the form open and says why when the token cannot be stored', async () => {
    const stored = { version: 1, active_profile: 'p1', global: { ai: {} }, profiles: [PROFILE] }
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'config_get' || cmd === 'profile_save') return stored
      if (cmd === 'secret_set') throw 'The keychain would not open'
      return null
    })
    await config.load()
    config.store.settingsSection = 'profiles'
    const wrapper = mount(SettingsModal, { global: { components: { AppModal } } })
    await flushPromises()
    await wrapper.findAll('button').find((one) => one.text() === 'Edit')!.trigger('click')
    await flushPromises()
    await wrapper.find('input[type="password"]').setValue('ghp_secret')
    await wrapper.findAll('button').find((one) => one.text().includes('Save profile'))!.trigger('click')
    await flushPromises()

    expect(useGit().store.log[0]?.text).toMatch(/keychain would not open/)
    // Still open, token and all, so nothing typed is lost.
    expect(wrapper.findAll('button').some((one) => one.text().includes('Save profile'))).toBe(true)
    wrapper.unmount()
  })
})
