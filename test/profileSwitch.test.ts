import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { useConfig, type Config, type ForgeKind, type Profile } from '~/composables/useConfig'
import { aimAt, aimedAt } from '~/composables/useInvoke'
import { useForge } from '~/composables/useForge'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const config = useConfig()
const forge = useForge()

let calls: { cmd: string; args: Record<string, unknown> }[] = []
let active = 'work'

function profile(id: string, name: string, forgeKind: ForgeKind, host: string): Profile {
  return {
    id,
    name,
    forge: forgeKind,
    host,
    git_name: null,
    git_email: null,
    ssh_key: null,
    signing_key: null,
    signing_format: null,
    sign_commits: null,
    sign_tags: null,
    projects: [],
    recents: [],
    active_project: null
  }
}

/** Two accounts on one host, which is the case the caching got wrong. */
function configNow(): Config {
  return {
    version: 1,
    active_profile: active,
    global: {} as never,
    profiles: [
      profile('work', 'Work', 'gitlab', 'gitlab.example'),
      profile('home', 'Home', 'gitlab', 'gitlab.example')
    ]
  }
}

beforeEach(async () => {
  calls = []
  active = 'work'
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'profile_activate') {
      active = String((args ?? {}).id)
      return configNow()
    }
    if (cmd === 'config_get') return configNow()
    if (cmd === 'forge_secret_key') return `forge:${active}`
    if (cmd === 'secret_status') return true
    if (cmd === 'forge_status') {
      return {
        kind: 'gitlab',
        host: 'gitlab.example',
        has_token: true,
        user: null,
        slug: null,
        error: null
      }
    }
    if (cmd === 'forge_me') return { login: active, id: 1, avatar: null }
    return null
  })
  await config.load()
})

describe('switching profile', () => {
  it('stops addressing calls to the repository being left', async () => {
    // A repository is open under the profile being left, so every call is
    // stamped with it — and the backend opens whatever it is stamped with.
    aimAt('/work/api')

    await config.activateProfile('home')

    // Nothing in the switch, the activation itself included, may carry the old
    // repository: the backend clears its path and would be handed it straight
    // back, then answer about it under the new profile's account.
    expect(aimedAt()).toBeNull()
    const stamped = calls.filter((call) => '__repo' in call.args)
    expect(stamped).toEqual([])
  })

  it('asks the forge again for a second account on the same host', async () => {
    await forge.refreshStatus()
    expect(forge.store.me?.login).toBe('work')

    await config.activateProfile('home')
    await forge.refreshStatus()

    // Both profiles are GitLab on one host, so a key made of forge and host
    // alone called this the same account and kept the first one's face.
    expect(forge.store.me?.login).toBe('home')
  })
})
