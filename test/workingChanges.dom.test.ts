// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import WorkingChanges from '~/components/WorkingChanges.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import FileList from '~/components/FileList.vue'
import AppModal from '~/components/AppModal.vue'
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
})

async function open() {
  const wrapper = mount(Host, { global: { components: { FileList, AppModal } } })
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
})
