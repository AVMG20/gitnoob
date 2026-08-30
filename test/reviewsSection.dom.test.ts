// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SideBar from '~/components/SideBar.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useGit, type LocalBranch } from '~/composables/useGit'
import { useForge, type ForgeStatus } from '~/composables/useForge'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const forge = useForge()

const branch = (name: string, isHead: boolean): LocalBranch => ({
  name,
  oid: `${name}-oid`,
  is_head: isHead,
  upstream: `origin/${name}`,
  ahead: 0,
  behind: 0
})

function status(over: Partial<ForgeStatus> = {}): ForgeStatus {
  return {
    kind: 'github',
    host: 'github.com',
    has_token: true,
    user: 'someone',
    slug: { host: 'github.com', owner: 'team', name: 'api' },
    error: null,
    ...over
  }
}

let open: ReturnType<typeof mount> | null = null

const show = () => {
  open = mount(SideBar, { global: { components: { MidTruncate }, stubs: { Teleport: true } } })
  return open
}

/** The heading of the pull requests section, wherever it has ended up. */
const heading = (wrapper: ReturnType<typeof mount>) =>
  wrapper.findAll('.section-title').find((one) => one.text().includes('Pull requests'))

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async () => null)
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.stashes = []
  git.store.refs = { locals: [branch('main', true)], remotes: [], tags: [], stashes: [] }
  forge.store.reviews = []
  forge.store.error = null
  forge.store.status = status()
})

afterEach(() => {
  open?.unmount()
  open = null
})

/**
 * The section is the whole width of the pane and three rows tall before it has
 * said anything. On a profile that cannot fetch a single review it was three
 * rows spent telling you so, in the one pane that never has enough of them.
 */
describe('the pull requests section', () => {
  it('is there for a profile signed in to a forge', async () => {
    const wrapper = show()
    await flushPromises()
    expect(heading(wrapper)).toBeTruthy()
  })

  it('is gone when the profile has no token', async () => {
    forge.store.status = status({ has_token: false })
    const wrapper = show()
    await flushPromises()
    expect(heading(wrapper)).toBeUndefined()
  })

  it('is gone when the profile names no forge at all', async () => {
    forge.store.status = status({ kind: 'none', has_token: false, slug: null })
    const wrapper = show()
    await flushPromises()
    expect(heading(wrapper)).toBeUndefined()
  })

  /**
   * A token with nothing to point it at is the repository being the odd one
   * out, not the profile — and that is worth a line, because the answer is
   * about this clone rather than about Settings.
   */
  it('stays, and says so, when the token has no remote on that forge to read', async () => {
    forge.store.status = status({ slug: null })
    const wrapper = show()
    await flushPromises()

    expect(heading(wrapper)).toBeTruthy()
    expect(wrapper.text()).toContain('No remote on this forge to read.')
  })
})
