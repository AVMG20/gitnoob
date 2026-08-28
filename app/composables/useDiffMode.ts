import { reactive, watch } from 'vue'

const KEY = 'gitnoob.diffmode'
const BLAME_KEY = 'gitnoob.blame'

/**
 * How the viewer shows a changed file.
 *
 * `diff` is the traditional one — hunks, both sides, everything else left out.
 * `file` shows the file as it stands with the changes marked down the side, the
 * way an editor does, which is the view to read code in rather than to review a
 * patch in. The traditional one stays the default.
 *
 * `blame` is not a third view but a column of the file one: who last touched
 * each line, beside the lines. It is turned on from the line numbers and stays
 * on for every file and every launch after, the way an editor's gutter does.
 */
export type DiffMode = 'diff' | 'file'

export const diffMode = reactive({ mode: 'diff' as DiffMode, blame: false })

const MODES: DiffMode[] = ['diff', 'file']

try {
  const saved = localStorage.getItem(KEY)
  if (MODES.includes(saved as DiffMode)) diffMode.mode = saved as DiffMode
  // Anyone left in the old blame view lands in the file view with the column
  // open, which is the same thing on screen.
  else if (saved === 'blame') {
    diffMode.mode = 'file'
    diffMode.blame = true
  }
  if (localStorage.getItem(BLAME_KEY) === 'on') diffMode.blame = true
} catch {
  // No stored preference is not a problem.
}

watch(
  () => diffMode.mode,
  (mode) => {
    try {
      localStorage.setItem(KEY, mode)
    } catch {
      // Storage can be refused; the choice still holds for this session.
    }
  }
)

watch(
  () => diffMode.blame,
  (on) => {
    try {
      localStorage.setItem(BLAME_KEY, on ? 'on' : 'off')
    } catch {
      // Same again.
    }
  }
)
