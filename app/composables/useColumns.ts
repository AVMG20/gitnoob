import { reactive, watch } from 'vue'

/**
 * The commit list's columns: how wide, and which of them are drawn.
 *
 * A view preference rather than a repository one, so it lives in the browser's
 * own storage next to the theme instead of in the config file every profile
 * shares.
 */

export type ColumnId = 'refs' | 'graph' | 'author' | 'date'

export interface Column {
  id: ColumnId
  /** What the heading says, and what the menu calls it. */
  label: string
  min: number
  max: number
}

/** The message column is not here: it takes whatever the others leave. */
export const COLUMNS: Column[] = [
  { id: 'refs', label: 'Branch / tag', min: 0, max: 420 },
  { id: 'graph', label: 'Graph', min: 24, max: 900 },
  { id: 'author', label: 'Author', min: 40, max: 400 },
  { id: 'date', label: 'Date', min: 40, max: 240 }
]

export interface ColumnState {
  width: Record<ColumnId, number | null>
  shown: Record<ColumnId, boolean>
}

/**
 * `null` means "whatever it takes". Only the graph starts that way: its width
 * follows the lanes in view, and a number here is the user overruling that.
 */
const DEFAULTS: ColumnState = {
  width: { refs: 124, graph: null, author: 130, date: 88 },
  shown: { refs: true, graph: true, author: true, date: true }
}

const KEY = 'gitnoob.columns'

function clone(state: ColumnState): ColumnState {
  return { width: { ...state.width }, shown: { ...state.shown } }
}

const state = reactive(clone(DEFAULTS))

function clamp(id: ColumnId, width: number): number {
  const column = COLUMNS.find((one) => one.id === id)
  if (!column) return width
  return Math.round(Math.min(column.max, Math.max(column.min, width)))
}

/**
 * Takes what was saved, one field at a time.
 *
 * Anything missing or out of range keeps its default, so a stored width from an
 * older version — or a hand-edited one — cannot leave a column at three pixels
 * with no way back to it.
 */
function load() {
  let saved: unknown
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return
    saved = JSON.parse(raw)
  } catch {
    return
  }
  const from = saved as Partial<ColumnState> | null
  if (!from || typeof from !== 'object') return
  for (const column of COLUMNS) {
    const width = from.width?.[column.id]
    if (typeof width === 'number' && Number.isFinite(width)) {
      state.width[column.id] = clamp(column.id, width)
    } else if (width === null) {
      state.width[column.id] = null
    }
    const shown = from.shown?.[column.id]
    if (typeof shown === 'boolean') state.shown[column.id] = shown
  }
}

load()

watch(
  state,
  () => {
    try {
      localStorage.setItem(KEY, JSON.stringify(state))
    } catch {
      // A window that cannot remember the widths still uses them this session.
    }
  },
  { deep: true }
)

export function useColumns() {
  return {
    state,
    columns: COLUMNS,
    /** The width to draw a column at, given what it would take unaided. */
    widthOf(id: ColumnId, natural: number): number {
      const set = state.width[id]
      return set === null ? natural : clamp(id, set)
    },
    setWidth(id: ColumnId, width: number) {
      state.width[id] = clamp(id, width)
    },
    toggle(id: ColumnId) {
      state.shown[id] = !state.shown[id]
    },
    /** One column back to the width it ships with. */
    resetWidth(id: ColumnId) {
      state.width[id] = DEFAULTS.width[id]
    },
    /** Back to the widths the app ships with, keeping what is shown. */
    resetWidths() {
      state.width = { ...DEFAULTS.width }
    },
    reset() {
      state.width = { ...DEFAULTS.width }
      state.shown = { ...DEFAULTS.shown }
    }
  }
}
