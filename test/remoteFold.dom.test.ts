// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SideBar from '~/components/SideBar.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()

const remote = (remote: string, name: string) => ({ remote, name, oid: `${remote}-${name}` })

const Host = {
  components: { SideBar, ContextMenu },
  template: '<div><SideBar /><ContextMenu /></div>'
}

let wrapper: ReturnType<typeof mount> | null = null

async function show() {
  wrapper = mount(Host, { global: { components: { MidTruncate }, stubs: { Teleport: true } } })
  await flushPromises()
  return wrapper
}

const heading = (w: ReturnType<typeof mount>, name: string) =>
  w.findAll('.remote-name').find((one) => one.text().startsWith(name))!

/** The remote branches drawn, as `remote/name`, in any order. */
const drawn = (w: ReturnType<typeof mount>) =>
  w
    .findAll('.row[title]')
    .map((one) => one.attributes('title'))
    .filter((title) => title?.startsWith('origin/') || title?.startsWith('pal/'))
    .sort()

beforeEach(() => {
  asked.mockReset()
  asked.mockImplementation(async () => null)
  localStorage.clear()
  git.store.repo = { path: '/repo', name: 'repo', head: 'main', detached: false } as never
  git.store.status = { staged: [], unstaged: [], conflicted: [] } as never
  git.store.stashes = []
  git.store.refs = {
    locals: [],
    remotes: [remote('origin', 'main'), remote('origin', 'bugfix/one'), remote('pal', 'main')],
    tags: [],
    stashes: []
  } as never
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('folding a remote', () => {
  it('hides its branches on a click and shows them on the next', async () => {
    const w = await show()
    expect(drawn(w)).toEqual(['origin/bugfix/one', 'origin/main', 'pal/main'])

    await heading(w, 'origin').trigger('click')
    expect(drawn(w)).toEqual(['pal/main'])
    expect(w.find('.row.folder').exists()).toBe(false)
    // Folded, it still says how many it holds.
    expect(heading(w, 'origin').text()).toContain('2')

    await heading(w, 'origin').trigger('click')
    expect(drawn(w)).toEqual(['origin/bugfix/one', 'origin/main', 'pal/main'])
  })

  it('folds one remote and leaves the others alone', async () => {
    const w = await show()
    await heading(w, 'pal').trigger('click')
    expect(drawn(w)).toEqual(['origin/bugfix/one', 'origin/main'])
  })

  it('is remembered the next time the sidebar is drawn', async () => {
    const w = await show()
    await heading(w, 'origin').trigger('click')
    w.unmount()
    wrapper = null

    const again = await show()
    expect(drawn(again)).toEqual(['pal/main'])
  })

  it('opens while the filter is searching, so a match is never hidden', async () => {
    const w = await show()
    await heading(w, 'origin').trigger('click')
    await w.find('input').setValue('main')
    await flushPromises()
    expect(drawn(w)).toEqual(['origin/main', 'pal/main'])
  })
})
