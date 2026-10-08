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

const review = (number: number, title: string) => ({
  number,
  title,
  author: 'someone',
  state: 'open',
  draft: false,
  source_branch: `branch-${number}`,
  target_branch: 'main',
  url: `https://github.com/team/api/pull/${number}`,
  updated_at: '',
  is_current: false,
  head_sha: `${number}-sha`,
  source: null,
  warning: null
})

/**
 * The list holds the fifty most recent and no more. On a busy project the one
 * somebody types the number of is further back, so the filter asks the forge
 * — but only when the list may be missing some, and only once typing pauses.
 */
describe('searching the pull requests', () => {
  let searched: string[] = []

  beforeEach(() => {
    vi.useFakeTimers()
    searched = []
    forge.store.found = []
    forge.store.foundFor = null
    forge.store.searchingFor = null
    asked.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'forge_search_reviews') {
        const query = (args as { query: string }).query
        searched.push(query)
        return query === '#812' ? [review(812, 'Fix the login crash')] : []
      }
      return null
    })
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  const type = async (wrapper: ReturnType<typeof mount>, text: string) => {
    await wrapper.find('.filter input').setValue(text)
    await vi.advanceTimersByTimeAsync(500)
    await flushPromises()
  }

  it('asks the forge for one beyond the fifty listed', async () => {
    forge.store.reviews = Array.from({ length: 50 }, (_, i) => review(1000 + i, `Change ${i}`))
    const wrapper = show()
    await type(wrapper, '#812')

    expect(searched).toEqual(['#812'])
    expect(wrapper.text()).toContain('Fix the login crash')
  })

  it('asks once typing pauses rather than on every letter', async () => {
    forge.store.reviews = Array.from({ length: 50 }, (_, i) => review(1000 + i, `Change ${i}`))
    const wrapper = show()
    const input = wrapper.find('.filter input')
    await input.setValue('#8')
    await vi.advanceTimersByTimeAsync(100)
    await input.setValue('#81')
    await vi.advanceTimersByTimeAsync(100)
    await type(wrapper, '#812')

    expect(searched).toEqual(['#812'])
  })

  it('leaves the forge alone when the list already holds everything open', async () => {
    forge.store.reviews = [review(7, 'Small change')]
    const wrapper = show()
    await type(wrapper, '#812')

    expect(searched).toEqual([])
    expect(wrapper.text()).toContain('Nothing open.')
  })
})
