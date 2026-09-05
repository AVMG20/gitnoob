/**
 * Where a new branch gets its name.
 *
 * Typed straight into the graph, on the row the branch will start from, rather
 * than into a dialog: the row is already the answer to "from where", and a
 * window in front of it was a page of reading to type one word. The graph is
 * not always on screen, though — a file or a review can be open in its place —
 * so the toolbar asks whoever is listening and falls back to the dialog when
 * nobody is.
 */

/**
 * Starts naming a branch from `start`, or from HEAD when `null`. Returns true
 * once the editor is on screen, false when it cannot be.
 */
type Taker = (start: string | null) => boolean

let taker: Taker | null = null

export function useBranchNaming() {
  return {
    /**
     * Registers the graph while it is mounted. Returns what to call to
     * withdraw it, which the graph does when it goes.
     */
    offer(take: Taker): () => void {
      taker = take
      return () => {
        if (taker === take) taker = null
      }
    },
    /** Asks for a name inline. False means open the dialog instead. */
    begin(start?: string): boolean {
      return taker?.(start ?? null) ?? false
    }
  }
}
