import { ref } from 'vue'
import { explain } from './gitErrors'

/**
 * The notices in the corner of the window.
 *
 * Failures used to go to the activity log and nowhere else: one red line at the
 * bottom of the window, which the next thing the app did — a refresh runs half
 * a dozen git commands — pushed out of sight before it had been read. A notice
 * that has to be caught in the second it exists is not a notice.
 *
 * So a failure now also stands in the corner until it is dismissed, saying what
 * to do about it rather than what git said, with git's own words a click away.
 */

export type ToastLevel = 'error' | 'info'

export interface Toast {
  id: number
  level: ToastLevel
  title: string
  /** Everything that was said, for when the one line is not enough. */
  detail: string | null
  /** How many times this same thing has happened in a row. */
  count: number
  at: number
}

/** How long a piece of good news stays up. Failures stay until dismissed. */
const INFO_MS = 5000

/**
 * How long the same message keeps counting up rather than stacking.
 *
 * A push that fails, is retried and fails again is one problem, not two, and a
 * loop that fails a hundred times is still one problem — it should say "×100",
 * not bury the window.
 */
const SAME_MS = 30_000

/** At most this many at once; the oldest goes to make room. */
const LIMIT = 4

const items = ref<Toast[]>([])
let seq = 0
const timers = new Map<number, ReturnType<typeof setTimeout>>()

function forget(id: number) {
  const timer = timers.get(id)
  if (timer !== undefined) {
    clearTimeout(timer)
    timers.delete(id)
  }
}

function dismiss(id: number) {
  forget(id)
  items.value = items.value.filter((one) => one.id !== id)
}

function clear() {
  for (const one of items.value) forget(one.id)
  items.value = []
}

function fade(toast: Toast) {
  if (toast.level !== 'info') return
  forget(toast.id)
  timers.set(
    toast.id,
    setTimeout(() => dismiss(toast.id), INFO_MS)
  )
}

/** Puts one up, or counts up the one already saying it. */
function raise(level: ToastLevel, title: string, detail: string | null = null): Toast {
  const now = Date.now()
  const same = items.value.find(
    (one) =>
      one.level === level &&
      one.title === title &&
      one.detail === detail &&
      now - one.at < SAME_MS
  )
  if (same) {
    same.count += 1
    same.at = now
    fade(same)
    return same
  }

  const toast: Toast = { id: ++seq, level, title, detail, count: 1, at: now }
  items.value = [...items.value, toast]
  while (items.value.length > LIMIT) {
    const oldest = items.value[0]
    if (!oldest) break
    dismiss(oldest.id)
  }
  fade(toast)
  return toast
}

export function useToasts() {
  return {
    items,
    dismiss,
    clear,
    /** Good news, which takes itself away again. */
    info: (title: string, detail: string | null = null) => raise('info', title, detail),
    /**
     * A failure, said in the app's own words with git's kept underneath.
     *
     * Takes the line that was headed for the log, so every place that already
     * reports a failure gets a notice out of it without being rewritten.
     */
    fail: (text: string) => {
      const { title, detail } = explain(text)
      return raise('error', title, detail)
    }
  }
}
