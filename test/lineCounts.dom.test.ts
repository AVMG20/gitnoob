// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import WorkingChanges from '~/components/WorkingChanges.vue'
import ContextMenu from '~/components/ContextMenu.vue'
import FileList from '~/components/FileList.vue'
import AppModal from '~/components/AppModal.vue'
import DiscardConflictsDialog from '~/components/DiscardConflictsDialog.vue'
import { resetDrafts } from '~/composables/useCommitDraft'
import { useGit } from '~/composables/useGit'
import { useFileView } from '~/composables/useFileView'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const git = useGit()
const view = useFileView()

const Host = {
  components: { WorkingChanges, ContextMenu },
  template: '<div><WorkingChanges /><ContextMenu /></div>'
}

beforeEach(() => {
  localStorage.clear()
  resetDrafts()
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockResolvedValue(null)
  view.state.mode = 'path'
  git.store.progress = null
  git.store.status = {
    staged: [{ path: 'b.ts', kind: 'modified', additions: 4, deletions: 3 }],
    unstaged: [
      { path: 'a.ts', kind: 'modified', additions: 68, deletions: 74 },
      { path: 'new.ts', kind: 'untracked', additions: 911, deletions: 0 },
      { path: 'logo.png', kind: 'untracked', additions: 0, deletions: 0 }
    ],
    conflicted: []
  }
})

async function open() {
  const wrapper = mount(Host, {
    global: { components: { FileList, AppModal, DiscardConflictsDialog } }
  })
  await flushPromises()
  return wrapper
}

describe('line counts in the working tree', () => {
  it('puts each file’s added and removed lines on its row', async () => {
    const wrapper = await open()
    const row = (path: string) => wrapper.find(`.row.file[data-path="${path}"]`)

    expect(row('a.ts').find('.plus').text()).toBe('+68')
    expect(row('a.ts').find('.minus').text()).toBe('−74')
    expect(row('new.ts').find('.plus').text()).toBe('+911')
    // Nothing removed, so nothing said about it.
    expect(row('new.ts').find('.minus').exists()).toBe(false)
    // A binary file has no lines to count.
    expect(row('logo.png').find('.plus').exists()).toBe(false)
    expect(row('b.ts').find('.plus').text()).toBe('+4')
  })

  it('gives way to the stage button on a hovered row', async () => {
    const wrapper = await open()
    const row = wrapper.find('.row.file[data-path="a.ts"]')
    expect(row.find('.act').exists()).toBe(true)
    expect(row.find('.plus').classes()).toContain('yields')
    expect(row.find('.minus').classes()).toContain('yields')
  })

  it('totals each list beside its heading', async () => {
    const wrapper = await open()
    const [unstaged, staged] = wrapper.findAll('.section-title')
    expect(unstaged!.find('.num').text()).toBe('3')
    expect(unstaged!.find('.lines .plus').text()).toBe('+979')
    expect(unstaged!.find('.lines .minus').text()).toBe('−74')
    expect(staged!.find('.lines .plus').text()).toBe('+4')
    expect(staged!.find('.lines .minus').text()).toBe('−3')
  })

  it('says nothing beside a heading with no lines to total', async () => {
    git.store.status = {
      staged: [],
      unstaged: [{ path: 'logo.png', kind: 'untracked', additions: 0, deletions: 0 }],
      conflicted: []
    }
    const wrapper = await open()
    expect(wrapper.find('.section-title .lines').exists()).toBe(false)
  })
})

describe('line counts in a list without a hover button', () => {
  it('stay put on hover', async () => {
    const wrapper = mount(FileList, {
      props: { files: [{ path: 'a.ts', kind: 'modified', additions: 2, deletions: 1 }] }
    })
    expect(wrapper.find('.act').exists()).toBe(false)
    expect(wrapper.find('.plus').classes()).not.toContain('yields')
  })
})
