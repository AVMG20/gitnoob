// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import WorkingChanges from '~/components/WorkingChanges.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import FileList from '~/components/FileList.vue'
import AppModal from '~/components/AppModal.vue'
import DiscardConflictsDialog from '~/components/DiscardConflictsDialog.vue'
import { useGit } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const git = useGit()

/**
 * The working-tree panel, against a status with one changed file and one file
 * git has never seen. What the menu offers for each of them is the question:
 * "discard" means take the content back for one and delete for the other, and
 * only one of those can be undone.
 */
const Host = {
  components: { WorkingChanges, ContextMenu },
  template: '<div><WorkingChanges /><ContextMenu /></div>'
}

beforeEach(() => {
  calls = []
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    return null
  })
  git.store.status = {
    staged: [],
    unstaged: [
      { path: 'app/app.vue', kind: 'modified' },
      { path: 'notes.md', kind: 'untracked' }
    ],
    conflicted: []
  }
  // Nothing part-way through, unless a test says so.
  git.store.progress = null
})

async function open() {
  const wrapper = mount(Host, {
    global: { components: { FileList, AppModal, DiscardConflictsDialog } }
  })
  await flushPromises()
  return wrapper
}

/** Right-clicks a row in the unstaged list and reads the menu it opened. */
async function menuFor(wrapper: Awaited<ReturnType<typeof open>>, path: string) {
  const row = wrapper.findAll('.row').find((one) => one.text().includes(path.split('/').pop()!))
  await row!.trigger('contextmenu')
  await flushPromises()
  return wrapper.findAll('.menu .item')
}

describe('the working tree panel', () => {
  it('offers to delete a file git has never seen, rather than a dead discard', async () => {
    const wrapper = await open()
    const items = await menuFor(wrapper, 'notes.md')
    const labels = items.map((one) => one.text())
    expect(labels.some((one) => one.includes('Delete this file'))).toBe(true)
    expect(labels.some((one) => one.includes('Discard changes'))).toBe(false)

    // It asks first: nothing in git holds a copy of an untracked file.
    await items.find((one) => one.text().includes('Delete this file'))!.trigger('click')
    await flushPromises()
    expect(calls.some((call) => call.cmd === 'delete_untracked')).toBe(false)
    expect(wrapper.text()).toContain('Deleting cannot be undone')

    await wrapper
      .findAll('button')
      .find((one) => one.text().includes('Delete the file'))!
      .trigger('click')
    await flushPromises()
    const done = calls.find((call) => call.cmd === 'delete_untracked')
    expect(done?.args.paths).toEqual(['notes.md'])
  })

  it('still discards a tracked file the way it always did', async () => {
    const wrapper = await open()
    const items = await menuFor(wrapper, 'app/app.vue')
    const discard = items.find((one) => one.text().includes('Discard changes'))
    expect(discard).toBeTruthy()

    await discard!.trigger('click')
    await flushPromises()
    // Straight through: the content is still in the index or in HEAD.
    expect(calls.find((call) => call.cmd === 'discard')?.args.paths).toEqual(['app/app.vue'])
  })

  it('lists a conflicted file among the unstaged changes, marked', async () => {
    git.store.status = {
      staged: [],
      unstaged: [{ path: 'app/app.vue', kind: 'modified' }],
      conflicted: ['app/store.ts']
    }
    const wrapper = await open()

    // In the list where the file lives, not in a strip of its own.
    const row = wrapper.findAll('.row.file').find((one) => one.text().includes('store.ts'))
    expect(row).toBeTruthy()
    expect(row!.find('.name.clash').exists()).toBe(true)
    // Counted with the rest, and said again on its own.
    expect(wrapper.find('.section-title').text()).toContain('2')
    expect(wrapper.find('.clashes').text()).toContain('1')
  })

  it('opens the resolver rather than the diff when one is clicked', async () => {
    git.store.status = { staged: [], unstaged: [], conflicted: ['app/store.ts'] }
    const wrapper = await open()

    await wrapper.findAll('.row.file')[0]!.trigger('click')
    await flushPromises()
    expect(git.store.resolving).toBe('app/store.ts')
    expect(git.store.viewer).toBeFalsy()
  })

  it('offers to throw a conflict away, and asks before it does', async () => {
    git.store.status = { staged: [], unstaged: [], conflicted: ['app/store.ts'] }
    const wrapper = await open()

    const items = await menuFor(wrapper, 'app/store.ts')
    const labels = items.map((one) => one.text())
    expect(labels.some((one) => one.includes('Throw the conflict away'))).toBe(true)
    // Neither of these means anything for a file with both sides in the index.
    expect(labels.some((one) => one.includes('Stage'))).toBe(false)
    expect(labels.some((one) => one.includes('Discard changes'))).toBe(false)

    await items.find((one) => one.text().includes('Throw the conflict away'))!.trigger('click')
    await flushPromises()
    expect(calls.some((call) => call.cmd === 'conflict_discard')).toBe(false)
    // The warning is one short line now: what it costs, and nothing else.
    expect(wrapper.text()).toContain('gone for good')

    await wrapper
      .findAll('button')
      .find((one) => one.text().includes('Throw it away'))!
      .trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'conflict_discard')?.args.paths).toEqual([
      'app/store.ts'
    ])
  })

  it('does not sweep conflicts into the index with "Stage all"', async () => {
    git.store.status = {
      staged: [],
      unstaged: [{ path: 'app/app.vue', kind: 'modified' }],
      conflicted: ['app/store.ts']
    }
    const wrapper = await open()

    await wrapper
      .findAll('button')
      .find((one) => one.text() === 'Stage all')!
      .trigger('click')
    await flushPromises()

    // `git add --all` would stage the conflicted file with its markers in it
    // and call the conflict settled. Only the changed file goes on.
    expect(calls.some((call) => call.cmd === 'stage_all')).toBe(false)
    expect(calls.find((call) => call.cmd === 'stage')?.args.paths).toEqual(['app/app.vue'])
  })

  it('does not offer to discard a folder from the staged side', async () => {
    git.store.status = {
      staged: [{ path: 'app/pages/miner/lootbox.vue', kind: 'modified' }],
      unstaged: [{ path: 'app/pages/shop/cart.vue', kind: 'modified' }],
      conflicted: []
    }
    const wrapper = await open()

    const folders = wrapper.findAll('.row.dir')
    const staged = folders[folders.length - 1]!
    await staged.trigger('contextmenu')
    await flushPromises()
    const labels = wrapper.findAll('.menu .item').map((one) => one.text())
    expect(labels.some((one) => one.includes('Unstage folder'))).toBe(true)
    expect(labels.some((one) => one.includes('Discard changes in this folder'))).toBe(false)
  })

  it('still offers it on the unstaged side, where it means one thing', async () => {
    git.store.status = {
      staged: [],
      unstaged: [{ path: 'app/pages/shop/cart.vue', kind: 'modified' }],
      conflicted: []
    }
    const wrapper = await open()

    await wrapper.findAll('.row.dir')[0]!.trigger('contextmenu')
    await flushPromises()
    const discard = wrapper
      .findAll('.menu .item')
      .find((one) => one.text().includes('Discard changes in this folder'))
    expect(discard).toBeTruthy()

    await discard!.trigger('click')
    await flushPromises()
    expect(calls.find((call) => call.cmd === 'discard')?.args.paths).toEqual([
      'app/pages/shop/cart.vue'
    ])
  })

  it('does not offer to throw a conflict away while a rebase is running', async () => {
    git.store.status = { staged: [], unstaged: [], conflicted: ['app/store.ts'] }
    git.store.progress = {
      merging: false,
      rebasing: true,
      cherry_picking: false,
      reverting: false,
      restoring: false,
      applied_stash: null,
      prepared: null
    }
    const wrapper = await open()

    const items = await menuFor(wrapper, 'app/store.ts')
    const labels = items.map((one) => one.text())
    // Mid-rebase the committed side is the branch being rebased onto, so
    // "keeps what the branch had" would be the wrong way round. Abort is the
    // way out, and it lives in the bar.
    expect(labels.some((one) => one.includes('Resolve'))).toBe(true)
    expect(labels.some((one) => one.includes('Throw the conflict away'))).toBe(false)
  })
})

/**
 * Committing is the end of a read, so it goes back to the graph.
 *
 * It used to close only when the commit emptied the working tree, which meant
 * committing half of it left the last file staring back at you over the list
 * you were about to pick the next one from.
 */
describe('committing from the panel', () => {
  it('goes back to the main view, with changes still left over', async () => {
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} })
      if (cmd === 'commit') return 'Committed 1 file'
      return null
    })
    git.store.status = {
      staged: [{ path: 'app/app.vue', kind: 'modified' }],
      unstaged: [{ path: 'notes.md', kind: 'untracked' }],
      conflicted: []
    }
    git.store.viewer = { path: 'app/app.vue', side: 'staged' }

    const wrapper = await open()
    await wrapper.find('textarea').setValue('Add the parser')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(calls.some((call) => call.cmd === 'commit')).toBe(true)
    expect(git.store.viewer).toBeNull()
  })

  it('stays where it is when git refused the commit', async () => {
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} })
      if (cmd === 'commit') throw new Error('the hook said no')
      return null
    })
    git.store.status = {
      staged: [{ path: 'app/app.vue', kind: 'modified' }],
      unstaged: [],
      conflicted: []
    }
    git.store.viewer = { path: 'app/app.vue', side: 'staged' }

    const wrapper = await open()
    await wrapper.find('textarea').setValue('Add the parser')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(git.store.viewer).toEqual({ path: 'app/app.vue', side: 'staged' })
  })
})
