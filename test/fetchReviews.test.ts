import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { useGit } from '~/composables/useGit'
import { useForge } from '~/composables/useForge'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const forge = useForge()

let calls: string[] = []
let hasToken = true

beforeEach(() => {
  calls = []
  hasToken = true
  forge.store.reviews = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string) => {
    calls.push(cmd)
    if (cmd === 'fetch') {
      return { argv: ['git', 'fetch'], ok: true, code: 0, stdout: '', stderr: '' }
    }
    if (cmd === 'forge_status') {
      return {
        kind: 'gitlab',
        host: 'gitlab.example',
        has_token: hasToken,
        user: null,
        slug: { host: 'gitlab.example', owner: 'team', name: 'api' },
        error: null
      }
    }
    if (cmd === 'forge_me') return { login: 'someone', id: 1, avatar: null }
    if (cmd === 'forge_reviews') return [{ number: 7 }]
    return null
  })
})

/**
 * A fetch is asking what moved on the other side. On a profile signed in to a
 * forge, a review opened or merged elsewhere is part of that answer, so the
 * list is read again with the refs rather than waiting for the next open.
 */
describe('fetching', () => {
  it('reads the reviews again when the profile is signed in to a forge', async () => {
    await forge.refreshStatus()
    await git.fetch()
    // Not awaited by `fetch`, so let the lookup it started land.
    await Promise.resolve()
    await Promise.resolve()

    expect(calls).toContain('forge_reviews')
    expect(forge.store.reviews).toHaveLength(1)
  })

  it('leaves the forge alone when there is no token', async () => {
    hasToken = false
    await forge.refreshStatus()
    await git.fetch()
    await Promise.resolve()

    expect(calls).not.toContain('forge_reviews')
  })
})
