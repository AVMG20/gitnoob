import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { useGit } from '~/composables/useGit'
import { useFileView } from '~/composables/useFileView'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const store = git.store

/** The status the panel shows, as it stands after the stage has landed. */
function afterStaging(path: string) {
  const left = (store.status?.unstaged ?? []).filter((one) => one.path !== path)
  return {
    staged: [...(store.status?.staged ?? []), { path, kind: 'modified' }],
    unstaged: left,
    conflicted: store.status?.conflicted ?? []
  }
}

/** The paths that still have something unstaged once they leave the index. */
let unstagesTo: string[] = []

beforeEach(() => {
  unstagesTo = ['app/two.vue']
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'stage') return 'Staged 1 file'
    if (cmd === 'unstage') {
      // What the refresh after the write would find: the file has moved to the
      // other list. Modelled here because the store's own refresh needs a
      // repository open, and these tests are about the viewer.
      const moved = (args?.paths ?? []) as string[]
      store.status = {
        staged: (store.status?.staged ?? []).filter((one) => !moved.includes(one.path)),
        unstaged: [
          ...(store.status?.unstaged ?? []),
          ...moved
            .filter((path) => unstagesTo.includes(path))
            .map((path) => ({ path, kind: 'modified' }))
        ],
        conflicted: store.status?.conflicted ?? []
      }
      return 'Unstaged 1 file'
    }
    if (cmd === 'discard') return 'Discarded 1 file'
    // The refresh a write runs; only the status matters here.
    if (cmd === 'working_status') return afterStaging(String((args ?? {}).__staged ?? ''))
    return null
  })
  useFileView().expandAll()
  store.repo = null
  store.viewer = null
  store.status = {
    staged: [],
    unstaged: [
      { path: 'app/one.vue', kind: 'modified' },
      { path: 'app/two.vue', kind: 'modified' },
      { path: 'app/three.vue', kind: 'modified' }
    ],
    conflicted: []
  }
})

/**
 * Reviewing a change is reading a file, staging it, reading the next one. The
 * file just staged is not where the viewer is pointing any more, so the panel
 * walks on rather than sitting on a page that has nothing left on it.
 */
describe('staging the file being read', () => {
  it('opens the next one down', async () => {
    store.viewer = { path: 'app/two.vue', side: 'unstaged' }
    await git.stage(['app/two.vue'])
    expect(store.viewer).toEqual({ path: 'app/three.vue', side: 'unstaged' })
  })

  it('goes back up when the last one was staged', async () => {
    store.viewer = { path: 'app/three.vue', side: 'unstaged' }
    await git.stage(['app/three.vue'])
    expect(store.viewer).toEqual({ path: 'app/two.vue', side: 'unstaged' })
  })

  it('closes when that was the only file left to read', async () => {
    store.status = { staged: [], unstaged: [{ path: 'app/one.vue', kind: 'modified' }], conflicted: [] }
    store.viewer = { path: 'app/one.vue', side: 'unstaged' }
    await git.stage(['app/one.vue'])
    expect(store.viewer).toBeNull()
  })

  it('steps over a conflicted file, which opens the resolver rather than the viewer', async () => {
    store.status = {
      staged: [],
      unstaged: [
        { path: 'app/one.vue', kind: 'modified' },
        { path: 'app/clash.vue', kind: 'modified' },
        { path: 'app/three.vue', kind: 'modified' }
      ],
      conflicted: ['app/clash.vue']
    }
    store.viewer = { path: 'app/one.vue', side: 'unstaged' }
    await git.stage(['app/one.vue'])
    expect(store.viewer).toEqual({ path: 'app/three.vue', side: 'unstaged' })
  })

  it('leaves the page alone when what was staged is not what is open', async () => {
    store.viewer = { path: 'app/one.vue', side: 'unstaged' }
    await git.stage(['app/three.vue'])
    expect(store.viewer).toEqual({ path: 'app/one.vue', side: 'unstaged' })
  })

  it('leaves the page alone when a whole folder is staged at once', async () => {
    store.viewer = { path: 'app/one.vue', side: 'unstaged' }
    await git.stage(['app/one.vue', 'app/two.vue'])
    expect(store.viewer).toEqual({ path: 'app/one.vue', side: 'unstaged' })
  })

  it('stays put when git refused the stage', async () => {
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'stage') throw new Error('it went wrong')
      return null
    })
    store.viewer = { path: 'app/two.vue', side: 'unstaged' }
    await git.stage(['app/two.vue'])
    expect(store.viewer).toEqual({ path: 'app/two.vue', side: 'unstaged' })
  })
})

/**
 * The other half of the same idea: a file that moves out from under the page
 * being read takes the page with it.
 */
describe('unstaging the file being read', () => {
  it('follows it to the unstaged side rather than sitting on an empty page', async () => {
    store.status = {
      staged: [{ path: 'app/two.vue', kind: 'modified' }],
      unstaged: [{ path: 'app/two.vue', kind: 'modified' }],
      conflicted: []
    }
    store.viewer = { path: 'app/two.vue', side: 'staged' }

    await git.unstage(['app/two.vue'])

    expect(store.viewer).toEqual({ path: 'app/two.vue', side: 'unstaged' })
  })

  it('closes when the file has nothing left on either side', async () => {
    unstagesTo = []
    store.status = {
      staged: [{ path: 'app/two.vue', kind: 'modified' }],
      unstaged: [],
      conflicted: []
    }
    store.viewer = { path: 'app/two.vue', side: 'staged' }

    await git.unstage(['app/two.vue'])

    expect(store.viewer).toBeNull()
  })

  it('leaves the page alone when what was unstaged is not what is open', async () => {
    unstagesTo = ['app/one.vue']
    store.status = {
      staged: [
        { path: 'app/one.vue', kind: 'modified' },
        { path: 'app/two.vue', kind: 'modified' }
      ],
      unstaged: [{ path: 'app/one.vue', kind: 'modified' }],
      conflicted: []
    }
    store.viewer = { path: 'app/two.vue', side: 'staged' }

    await git.unstage(['app/one.vue'])

    expect(store.viewer).toEqual({ path: 'app/two.vue', side: 'staged' })
  })
})

describe('discarding the file being read', () => {
  it('shuts the page, because there is nothing left to show on it', async () => {
    store.viewer = { path: 'app/two.vue', side: 'unstaged' }
    await git.discard(['app/two.vue'])
    expect(store.viewer).toBeNull()
  })

  it('leaves a commit page alone — a commit keeps its diff whatever the tree does', async () => {
    store.viewer = { path: 'app/two.vue', commit: 'a'.repeat(40) }
    await git.discard(['app/two.vue'])
    expect(store.viewer).toEqual({ path: 'app/two.vue', commit: 'a'.repeat(40) })
  })
})

/**
 * The bulk gestures. Staging the lot is not stepping through a review, so the
 * file being read follows itself across rather than jumping somewhere new.
 */
describe('staging more than one file at a time', () => {
  it('keeps the open file on screen, on the side it moved to', async () => {
    store.status = {
      staged: [],
      unstaged: [
        { path: 'app/one.vue', kind: 'modified' },
        { path: 'app/two.vue', kind: 'modified' }
      ],
      conflicted: []
    }
    store.viewer = { path: 'app/one.vue', side: 'unstaged' }

    asked.mockImplementation(async (cmd: string) => {
      if (cmd !== 'stage') return null
      store.status = {
        staged: [
          { path: 'app/one.vue', kind: 'modified' },
          { path: 'app/two.vue', kind: 'modified' }
        ],
        unstaged: [],
        conflicted: []
      }
      return 'Staged 2 files'
    })

    await git.stage(['app/one.vue', 'app/two.vue'])

    expect(store.viewer).toEqual({ path: 'app/one.vue', side: 'staged' })
  })

  it('does the same for Stage all', async () => {
    store.status = {
      staged: [],
      unstaged: [{ path: 'app/one.vue', kind: 'modified' }],
      conflicted: []
    }
    store.viewer = { path: 'app/one.vue', side: 'unstaged' }

    asked.mockImplementation(async (cmd: string) => {
      if (cmd !== 'stage_all') return null
      store.status = {
        staged: [{ path: 'app/one.vue', kind: 'modified' }],
        unstaged: [],
        conflicted: []
      }
      return 'Staged everything'
    })

    await git.stageAll()

    expect(store.viewer).toEqual({ path: 'app/one.vue', side: 'staged' })
  })
})
