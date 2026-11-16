// @vitest-environment happy-dom
import { flushPromises } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()

const detail = (oid: string) => ({ oid, short: oid.slice(0, 7), summary: oid, files: [] })

beforeEach(() => {
  asked.mockReset()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
})

describe('selecting commits one after another', () => {
  it('keeps the detail of the last one picked when an earlier answer comes back late', async () => {
    const answers = new Map<string, (value: unknown) => void>()
    asked.mockImplementation(
      (cmd: string, args?: Record<string, unknown>) =>
        new Promise((resolve) => {
          if (cmd === 'commit_detail') answers.set(String(args?.oid), resolve)
          else resolve(null)
        }) as never
    )

    const first = git.select('aaaaaaa')
    const second = git.select('bbbbbbb')
    answers.get('bbbbbbb')!(detail('bbbbbbb'))
    await second
    answers.get('aaaaaaa')!(detail('aaaaaaa'))
    await first
    await flushPromises()

    expect(git.store.selected).toBe('bbbbbbb')
    expect(git.store.detail?.oid).toBe('bbbbbbb')
  })
})

describe('opening two repositories one after another', () => {
  it('ends on the one asked for last, whichever answer comes back last', async () => {
    const answers = new Map<string, (value: unknown) => void>()
    asked.mockImplementation(
      (cmd: string, args?: Record<string, unknown>) =>
        new Promise((resolve) => {
          if (cmd === 'open_repo') answers.set(String(args?.path), resolve)
          else resolve(null)
        }) as never
    )
    const info = (path: string) => ({ path, name: path.split('/').pop(), head: 'main', detached: false })
    const first = git.openRepo('/repos/a')
    const second = git.openRepo('/repos/b')
    answers.get('/repos/b')!(info('/repos/b'))
    await flushPromises()
    answers.get('/repos/a')!(info('/repos/a'))
    expect(await first).toBe(false)
    await flushPromises()
    expect(git.store.repo?.path).toBe('/repos/b')
    void second
  })
})
