import { reactive, watch } from 'vue'

/**
 * Colours put on folders in the Files tree, from the folder's right-click menu.
 *
 * A view preference, so it lives in the browser's own storage like the
 * columns and the theme, kept per repository by its path on disk.
 */

export interface FolderColor {
  id: string
  label: string
  /** The hue; mixed with the theme's text colour where it is drawn. */
  hex: string
}

export const FOLDER_COLORS: FolderColor[] = [
  { id: 'red', label: 'Red', hex: '#ef4444' },
  { id: 'orange', label: 'Orange', hex: '#f97316' },
  { id: 'yellow', label: 'Yellow', hex: '#eab308' },
  { id: 'green', label: 'Green', hex: '#22c55e' },
  { id: 'teal', label: 'Teal', hex: '#14b8a6' },
  { id: 'blue', label: 'Blue', hex: '#3b82f6' },
  { id: 'purple', label: 'Purple', hex: '#a855f7' },
  { id: 'pink', label: 'Pink', hex: '#ec4899' }
]

const KEY = 'gitnoob.folderColors'

/** Colour ids by folder path, by repository path. */
const state = reactive<Record<string, Record<string, string>>>({})

function load() {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}') as unknown
    if (!saved || typeof saved !== 'object') return
    const known = new Set(FOLDER_COLORS.map((one) => one.id))
    for (const [repo, dirs] of Object.entries(saved as Record<string, unknown>)) {
      if (!dirs || typeof dirs !== 'object') continue
      const kept: Record<string, string> = {}
      for (const [dir, color] of Object.entries(dirs as Record<string, unknown>)) {
        if (typeof color === 'string' && known.has(color)) kept[dir] = color
      }
      if (Object.keys(kept).length) state[repo] = kept
    }
  } catch {
    // Nothing saved, or storage that cannot be read: no colours.
  }
}

load()

watch(
  state,
  () => {
    try {
      localStorage.setItem(KEY, JSON.stringify(state))
    } catch {
      // Storage full or blocked: the colours last for this session.
    }
  },
  { deep: true }
)

/** The colour of a folder, or null when it has none. */
export function folderColor(repo: string | undefined, dir: string): FolderColor | null {
  if (!repo) return null
  const id = state[repo]?.[dir]
  return id ? (FOLDER_COLORS.find((one) => one.id === id) ?? null) : null
}

/** Colours a folder, or clears its colour with null. */
export function setFolderColor(repo: string | undefined, dir: string, id: string | null) {
  if (!repo) return
  if (id === null) {
    const dirs = state[repo]
    if (!dirs) return
    delete dirs[dir]
    if (!Object.keys(dirs).length) delete state[repo]
    return
  }
  ;(state[repo] ??= {})[dir] = id
}

/** Forgets every colour. For tests. */
export function resetFolderColors() {
  for (const repo of Object.keys(state)) delete state[repo]
}
