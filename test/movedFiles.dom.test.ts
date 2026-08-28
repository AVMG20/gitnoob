// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import WorkingChanges from '~/components/WorkingChanges.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import FileList from '~/components/FileList.vue'
import AppModal from '~/components/AppModal.vue'
import DiscardConflictsDialog from '~/components/DiscardConflictsDialog.vue'
import MidTruncate from '~/components/MidTruncate.vue'
import { useGit } from '~/composables/useGit'
import { useFileView } from '~/composables/useFileView'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const view = useFileView()
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const FROM = 'tests/Feature/Filament/CreateTicketFormTest.php'
const TO = 'tests/Feature/Filament/Tickets/CreateTicketFormTest.php'

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
  // The path list rather than the tree, so a row's text is the whole path and
  // the assertions are about the path rather than about the folding.
  view.state.mode = 'path'
  git.store.progress = null
  // A file moved into a subfolder and edited afterwards: the move is staged,
  // the edit is not, and both belong to the name the file has now.
  git.store.status = {
    staged: [{ path: TO, from: FROM, kind: 'renamed' }],
    unstaged: [{ path: TO, from: null, kind: 'modified' }],
    conflicted: []
  }
})

async function open() {
  const wrapper = mount(Host, {
    global: { components: { FileList, AppModal, DiscardConflictsDialog, MidTruncate } }
  })
  await flushPromises()
  return wrapper
}

const rowFor = (wrapper: Awaited<ReturnType<typeof open>>, path: string) =>
  wrapper.findAll('.row.file').filter((one) => one.attributes('data-path') === path)

/** The old location a row names, cut in the middle but read back whole. */
const origin = (wrapper: Awaited<ReturnType<typeof open>>) => wrapper.find('.moved .mid').text()

describe('a file that was moved', () => {
  it('is listed where it now is, never under the name it lost', async () => {
    const wrapper = await open()
    expect(wrapper.findAll('.row.file').map((one) => one.attributes('data-path'))).toEqual([TO, TO])
    expect(wrapper.html()).not.toContain(`data-path="${FROM}"`)
  })

  it('says where it came from, the way a move is read', async () => {
    const wrapper = await open()
    const staged = rowFor(wrapper, TO).at(-1)!
    // The name did not change, only the folder, so only the folder is named.
    expect(staged.find('.moved-word').text()).toBe('moved from')
    expect(staged.find('.moved .mid').text()).toBe('tests/Feature/Filament')
    expect(staged.attributes('title')).toBe(`${FROM} → ${TO}`)
  })

  it('names the whole old path when the file was renamed as well as moved', async () => {
    git.store.status = {
      staged: [{ path: TO, from: 'tests/Feature/OldName.php', kind: 'renamed' }],
      unstaged: [],
      conflicted: []
    }
    const wrapper = await open()
    expect(origin(wrapper)).toBe('tests/Feature/OldName.php')
  })

  it('says so plainly for a file that came from the top of the repository', async () => {
    git.store.status = {
      staged: [{ path: TO, from: 'CreateTicketFormTest.php', kind: 'renamed' }],
      unstaged: [],
      conflicted: []
    }
    const wrapper = await open()
    expect(origin(wrapper)).toBe('the top of the repository')
  })

  it('says nothing about a move on a file that stayed where it was', async () => {
    git.store.status = {
      staged: [],
      unstaged: [{ path: 'app/app.vue', kind: 'modified' }],
      conflicted: []
    }
    const wrapper = await open()
    expect(wrapper.find('.moved').exists()).toBe(false)
    expect(wrapper.find('.row.file').attributes('title')).toBe('app/app.vue')
  })

  it('stages the edit under the name the file has now', async () => {
    const wrapper = await open()
    const unstaged = rowFor(wrapper, TO)[0]!
    await unstaged.trigger('contextmenu')
    await flushPromises()
    const stage = wrapper
      .findAll('.menu .item')
      .find((one) => one.text().startsWith('Stage'))!
    await stage.trigger('click')
    await flushPromises()

    // The old name is gone from disk; asking git to add it is the error the
    // window used to show — "pathspec ... did not match any files".
    expect(calls.find((call) => call.cmd === 'stage')?.args).toEqual({ paths: [TO] })
  })

  it('opens the diff for the name the file has now', async () => {
    const wrapper = await open()
    await rowFor(wrapper, TO)[0]!.trigger('click')
    await flushPromises()
    expect(git.store.viewer?.path).toBe(TO)
  })
})
