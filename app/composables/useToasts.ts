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
 *
 * Good news gets one too, but only where the window would otherwise say nothing
 * — and in the app's own words. What it must never carry is whatever git wrote
 * on its way out: "Your branch is based on 'origin/x', but the upstream is
 * gone", "Your branch is behind 'origin/main' by 63 commits". That is advice
 * for someone at a terminal, about a state the branch bar is already showing,
 * and a notice that repeats the screen is one more thing to dismiss.
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

/** How long a piece of good news stays up. */
const INFO_MS = 5000

/**
 * How long a failure stays up.
 *
 * Longer than good news, because it says something you have to act on and the
 * words are worth reading twice — but not forever: a notice nobody dismissed is
 * a notice standing over the window an hour later. The clock stops while the
 * pointer is on the stack, so a failure being read is never taken away
 * mid-sentence, and one being opened up to see what git said keeps its place
 * for as long as it is open.
 */
const ERROR_MS = 15_000

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
/** True while the pointer is on the stack, which stops every clock in it. */
const held = ref(false)
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
  // The stack is gone from under the pointer, so the mouse will never leave it
  // and nothing would ever start a clock again.
  if (!items.value.length) held.value = false
}

function clear() {
  for (const one of items.value) forget(one.id)
  items.value = []
  held.value = false
}

/** Starts, or restarts, one notice's clock. */
function fade(toast: Toast) {
  forget(toast.id)
  if (held.value) return
  const after = toast.level === 'info' ? INFO_MS : ERROR_MS
  timers.set(
    toast.id,
    setTimeout(() => dismiss(toast.id), after)
  )
}

/**
 * Holds the stack, or lets it go again.
 *
 * Letting go starts every clock from the beginning rather than from wherever it
 * was: someone who moved the pointer away has just finished reading, and the
 * few seconds they had left are not what should decide how long the notice is
 * still there.
 */
function hold(on: boolean) {
  held.value = on
  if (on) {
    for (const one of items.value) forget(one.id)
    return
  }
  for (const one of items.value) fade(one)
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
    hold,
    /**
     * Good news, which takes itself away again.
     *
     * The words are the app's, not git's — a caller with git's output in hand
     * says what the action did and leaves the output in the log, where a person
     * who wants it can go and read it.
     */
    info: (title: string, detail: string | null = null) => raise('info', title, detail),
    /**
     * A failure, said in the app's own words with git's kept underneath.
     *
     * Takes the line that was headed for the log, so every place that already
     * reports a failure gets a notice out of it without being rewritten.
     */
    fail: (text: string) => {
      const { title, detail, quiet } = explain(text)
      // Some failures are answered by the window itself — a merge stopping on
      // conflicts opens the resolver — and a notice over that is noise.
      if (quiet) return null
      return raise('error', title, detail)
    }
  }
}
