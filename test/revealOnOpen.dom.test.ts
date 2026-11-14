// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import SearchModal from '~/components/SearchModal.vue'
import FileTree from '~/components/FileTree.vue'
import WorkingChanges from '~/components/WorkingChanges.vue'
import FileList from '~/components/FileList.vue'
import AppModal from '~/components/AppModal.vue'
import DiscardConflictsDialog from '~/components/DiscardConflictsDialog.vue'
import { useGit } from '~/composables/useGit'
import { openTab, goBack, resetTabs, switchTabs, useTabs } from '~/composables/useTabs'
import { openSearch, resetSearch } from '~/composables/useSearch'
import { resetFileTree, treeSession } from '~/composables/useFileTree'
import { resetDrafts } from '~/composables/useCommitDraft'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

const asked = vi.mocked(invoke)
const git = useGit()
const tabs = useTabs()
const entry = (path: string, dir = false) => ({
  name: path.slice(path.lastIndexOf('/') + 1),
  path,
  dir,
  ignored: false,
  link: false
})
const DISK: Record<string, unknown[]> = {
  '': [entry('app', true), entry('README.md')],
  app: [entry('app/Billing', true), entry('app/app.vue')],
  'app/Billing': [entry('app/Billing/Tax.php')]
}
let wrappers: ReturnType<typeof mount>[] = []

beforeEach(() => {
  localStorage.clear()
  resetDrafts()
  resetTabs()
  resetSearch()
  resetFileTree()
  switchTabs('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as never
  git.store.status = { staged: [], unstaged: [{ path: 'app/app.vue', kind: 'modified' }], conflicted: [] } as never
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'list_dir') return DISK[String(args?.path ?? '')] ?? []
    if (cmd === 'find_files') return [{ path: 'app/Billing/Tax.php', score: 9, indices: [] }]
    if (cmd === 'read_file') return { text: 'x\n', hash: 'h', eol: 'lf', bom: false, readonly: null, missing: false }
    return null
  })
})

afterEach(() => {
  for (const one of wrappers) one.unmount()
  wrappers = []
  git.store.repo = null
  git.store.status = null
})

const keep = <T extends ReturnType<typeof mount>>(w: T) => (wrappers.push(w), w)

async function tree() {
  const w = keep(mount(FileTree, { attachTo: document.body, global: { components: { AppModal } } }))
  await flushPromises()
  await flushPromises()
  return w
}

describe('opening a file that was looked for', () => {
  it('brings the sidebar to Files with the file revealed and marked', async () => {
    expect(treeSession().mode).toBe('branches')
    openSearch('files')
    const w = keep(mount(SearchModal, { attachTo: document.body, global: { components: { AppModal } } }))
    await flushPromises()
    await w.find('.search input[type="text"]').setValue('tax')
    await new Promise((done) => setTimeout(done, 100))
    await flushPromises()
    w.find('.search input[type="text"]').element.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
    )
    await flushPromises()
    expect(tabs.activeTab()?.path).toBe('app/Billing/Tax.php')
    const session = treeSession()
    expect(session.mode).toBe('files')
    expect(session.open).toMatchObject({ app: true, 'app/Billing': true })
    const t = await tree()
    expect(t.find('.row.on').attributes('data-path')).toBe('app/Billing/Tax.php')
  })

  it('reveals the file back and forward land on', async () => {
    openTab({ path: 'README.md', mode: 'file' })
    openTab({ path: 'app/Billing/Tax.php', mode: 'file' })
    expect(treeSession().mode).toBe('branches')
    goBack()
    expect(treeSession().mode).toBe('files')
    expect(treeSession().reveal?.path).toBe('README.md')
  })
})

describe('opening a file from the right panel', () => {
  it('leaves Branches showing', async () => {
    const w = keep(
      mount(
        { components: { WorkingChanges }, template: '<div><WorkingChanges /></div>' },
        { global: { components: { FileList, AppModal, DiscardConflictsDialog } } }
      )
    )
    await flushPromises()
    await w.findAll('.row.file').find((one) => one.text().includes('app.vue'))!.trigger('click')
    await flushPromises()
    expect(tabs.activeTab()?.path).toBe('app/app.vue')
    expect(treeSession().mode).toBe('branches')
  })

  it('still reveals it when Files is already showing', async () => {
    treeSession().mode = 'files'
    const t = await tree()
    openTab({ path: 'app/app.vue', side: 'unstaged', mode: 'diff' }, { preview: true })
    await flushPromises()
    await flushPromises()
    expect(treeSession().mode).toBe('files')
    expect(t.find('.row.on').attributes('data-path')).toBe('app/app.vue')
  })
})
