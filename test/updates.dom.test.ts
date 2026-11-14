// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { useUpdates } from '~/composables/useUpdates'
import { resetEdits, setSaveState } from '~/composables/useFileEdit'

vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn(async () => '0.4.9') }))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn() }))
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => false) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }))

const asked = vi.mocked(check)
const updates = useUpdates()

function offer(version: string) {
  asked.mockResolvedValue({ version, body: null, date: undefined } as never)
}

beforeEach(() => {
  vi.useFakeTimers()
  asked.mockReset()
  updates.store.stage = 'idle'
  updates.store.checked = null
  updates.store.dismissed = null
})

afterEach(() => {
  updates.stopWatching()
  vi.useRealTimers()
})

describe('looking for a newer release', () => {
  it('offers what it finds', async () => {
    offer('0.5.0')
    await updates.checkForUpdate(true)
    expect(updates.store.stage).toBe('available')
    expect(updates.store.version).toBe('0.5.0')
  })

  it('keeps quiet about a version the user already said not now to', async () => {
    offer('0.5.0')
    await updates.checkForUpdate(true)
    updates.dismiss()
    expect(updates.store.stage).toBe('idle')

    await updates.checkForUpdate(true)
    expect(updates.store.stage).toBe('idle')

    offer('0.5.1')
    await updates.checkForUpdate(true)
    expect(updates.store.stage).toBe('available')
  })

  it('still answers the button in settings about a dismissed version', async () => {
    offer('0.5.0')
    await updates.checkForUpdate(true)
    updates.dismiss()
    await updates.checkForUpdate()
    expect(updates.store.stage).toBe('available')
  })

  it('asks again on a schedule, and on focus only once the answer is stale', async () => {
    asked.mockResolvedValue(null)
    updates.watchForUpdates()
    await vi.advanceTimersByTimeAsync(1)
    expect(asked).toHaveBeenCalledTimes(1)

    window.dispatchEvent(new Event('focus'))
    expect(asked).toHaveBeenCalledTimes(1)

    await vi.advanceTimersByTimeAsync(61 * 60_000)
    window.dispatchEvent(new Event('focus'))
    await vi.advanceTimersByTimeAsync(1)
    expect(asked).toHaveBeenCalledTimes(2)

    await vi.advanceTimersByTimeAsync(4 * 60 * 60_000)
    expect(asked).toHaveBeenCalledTimes(3)

    updates.stopWatching()
    await vi.advanceTimersByTimeAsync(8 * 60 * 60_000)
    expect(asked).toHaveBeenCalledTimes(3)
  })
})

describe('installing', () => {
  it('asks again before the restart about edits made while it downloaded', async () => {
    resetEdits()
    vi.mocked(relaunch).mockClear()
    asked.mockResolvedValue({
      version: '0.5.0',
      body: null,
      date: undefined,
      downloadAndInstall: async () => {
        // Typed into a file whose save is then refused, mid-download.
        setSaveState(null, 'work:a.php', 'conflict')
      }
    } as never)
    await updates.checkForUpdate(true)
    vi.useRealTimers()
    await updates.install()
    expect(updates.store.stage).toBe('ready')
    expect(relaunch).not.toHaveBeenCalled()
    resetEdits()
  })
})
