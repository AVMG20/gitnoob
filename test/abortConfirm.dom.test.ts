// @vitest-environment happy-dom
import { flushPromises } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { ask } from '@tauri-apps/plugin-dialog'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => false) }))

const asked = vi.mocked(invoke)
const asking = vi.mocked(ask)
const git = useGit()

beforeEach(() => {
  asked.mockClear()
  asking.mockClear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
})

describe('aborting a merge', () => {
  it('asks first when resolved files would be thrown away, and stops on no', async () => {
    git.store.status = { staged: [], unstaged: [], conflicted: ['a.txt', 'b.txt'] } as never
    await flushPromises()
    git.store.status = { staged: [{ path: 'a.txt', kind: 'modified' }], unstaged: [], conflicted: ['b.txt'] } as never
    await flushPromises()
    await git.abortMerge()
    expect(asking).toHaveBeenCalledOnce()
    expect(String(asking.mock.calls[0]![0])).toContain('1 file you resolved')
    expect(asked.mock.calls.some(([cmd]) => cmd === 'abort_merge')).toBe(false)
  })

  it('just aborts when nothing has been resolved yet', async () => {
    git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
    await flushPromises()
    // Merged cleanly where it could, which is staged, and stopped on b.txt.
    git.store.status = { staged: [{ path: 'c.txt', kind: 'modified' }], unstaged: [], conflicted: ['b.txt'] } as never
    await flushPromises()
    await git.abortMerge()
    expect(asking).not.toHaveBeenCalled()
    expect(asked.mock.calls.some(([cmd]) => cmd === 'abort_merge')).toBe(true)
  })
})
