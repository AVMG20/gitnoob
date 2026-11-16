import { describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { useConfig } from '~/composables/useConfig'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)

describe('two settings changed in quick succession', () => {
  it('both survive: the second starts from what the first left', async () => {
    let stored: Record<string, unknown> = { version: 1, active_profile: null, global: { ai: {} }, profiles: [] }
    const sent: Record<string, unknown>[] = []
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'config_get') return stored
      if (cmd === 'config_set_global') {
        sent.push(args!.global as Record<string, unknown>)
        await new Promise((resolve) => setTimeout(resolve, 10))
        stored = { ...stored, global: args!.global }
        return stored
      }
      return null
    })
    const config = useConfig()
    await config.load()
    const one = config.updateGlobal((now) => ({ ...now, show_avatars: false }))
    const two = config.updateGlobal((now) => ({ ...now, check_updates: false }))
    await Promise.all([one, two])
    expect(sent[1]).toMatchObject({ show_avatars: false, check_updates: false })
    expect(config.settings.value).toMatchObject({ show_avatars: false, check_updates: false })
  })
})
