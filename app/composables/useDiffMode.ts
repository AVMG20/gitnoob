import { reactive, watch } from 'vue'

const KEY = 'gitnoob.diffmode'

/**
 * How the viewer shows a changed file.
 *
 * `diff` is the traditional one — hunks, both sides, everything else left out.
 * `file` shows the file as it stands with the changes marked down the side, the
 * way an editor does, which is the view to read code in rather than to review a
 * patch in. `blame` is the same file with the commit that last touched each
 * line beside it. The traditional one stays the default.
 */
export type DiffMode = 'diff' | 'file' | 'blame'

export const diffMode = reactive({ mode: 'diff' as DiffMode })

const MODES: DiffMode[] = ['diff', 'file', 'blame']

try {
  const saved = localStorage.getItem(KEY)
  if (MODES.includes(saved as DiffMode)) diffMode.mode = saved as DiffMode
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
