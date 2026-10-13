// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import ConflictOverlay from '~/components/ConflictOverlay.vue'
import { useGit, type InProgress } from '~/composables/useGit'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const git = useGit()

const merging = (returnsTo: string | null): InProgress => ({
  merging: true,
  rebasing: false,
  cherry_picking: false,
  reverting: false,
  restoring: false,
  applied_stash: null,
  prepared: "Merge branch 'feature' into staging",
  returns_to: returnsTo
})

const show = () => mount(ConflictOverlay, { global: { stubs: { ConflictView: true } } })

beforeEach(() => {
  git.store.status = { staged: [], unstaged: [], conflicted: ['a.txt'] } as never
})

describe('the conflict sheet', () => {
  it('says where you go back to when the merge was started for another branch', () => {
    git.store.progress = merging('feature')
    const note = show().find('[data-test="returns-to"]')
    expect(note.exists()).toBe(true)
    expect(note.text()).toContain('back to feature')
  })

  it('says nothing about going anywhere for an ordinary merge', () => {
    git.store.progress = merging(null)
    expect(show().find('[data-test="returns-to"]').exists()).toBe(false)
  })
})
