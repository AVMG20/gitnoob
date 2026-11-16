import { reactive } from 'vue'
import { Copy, GitCommitHorizontal, Users } from 'lucide-vue-next'
import { copyText, fullTime, useGit, type BlameRun, type CommitDetail } from './useGit'
import { invoke } from './useInvoke'
import { openTab } from './useTabs'
import type { MenuItem } from './useContextMenu'

/**
 * What the blame column says about a line, shared by the editor's gutter and
 * the read-only file view so the two read the same.
 */

/** The day a run came in, as `2025-03-14` in local time: one width, every row. */
export function blameDate(seconds: number) {
  const at = new Date(seconds * 1000)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`
}

/** The hover text: who, their address, when to the minute, the hash and the subject. */
export function blameTooltip(run: BlameRun | null | undefined) {
  if (!run || run.uncommitted) return 'Not committed yet: this line is your own working copy.'
  return [
    `${run.author} <${run.email}>`,
    fullTime(run.time),
    `${run.short} ${run.summary}`,
    'Click for the commit.'
  ].join('\n')
}

/** How long ago, in the largest unit that says it: `3 days ago`, `2 years ago`. */
export function blameAgo(seconds: number, now = Date.now() / 1000) {
  const diff = Math.max(0, now - seconds)
  const units: [number, string][] = [
    [365 * 86400, 'year'],
    [30 * 86400, 'month'],
    [86400, 'day'],
    [3600, 'hour'],
    [60, 'minute']
  ]
  for (const [size, name] of units) {
    const count = Math.floor(diff / size)
    if (count >= 1) return `${count} ${name}${count === 1 ? '' : 's'} ago`
  }
  return 'just now'
}

/** Where a run's commit is shown: selected in the graph, its details beside it. */
export function openBlamed(run: BlameRun | null | undefined) {
  if (!run || run.uncommitted) return
  void useGit().revealCommit(run.oid)
}

// --- the popover a click on an entry opens
//
// A click on the column is a question about a line you are reading, so it is
// answered beside the line, the way an IDE's annotation popup does it, and the
// file stays where it was. The graph is one of the things it offers, not where
// the click goes.

export const blamePopup = reactive({
  open: false,
  run: null as BlameRun | null,
  /** The file the column belongs to, for "Show this file's change". */
  path: '',
  /** The entry it hangs from, in window coordinates. */
  anchor: { left: 0, top: 0, bottom: 0 },
  /** The subject until the whole message is read, then the whole message. */
  message: ''
})

let asked = 0
let returnTo: HTMLElement | null = null

/**
 * Opens the popover for a run, hung from the entry that was clicked. `back` is
 * what takes focus again when it closes (the editor). Uncommitted lines have
 * no commit to describe, so a click on one does nothing.
 */
export function openBlamePopup(
  run: BlameRun | null | undefined,
  path: string,
  anchor: Element | null,
  back?: HTMLElement | null
) {
  if (!run || run.uncommitted) return false
  const box = anchor?.getBoundingClientRect()
  blamePopup.anchor = { left: box?.left ?? 0, top: box?.top ?? 0, bottom: box?.bottom ?? 0 }
  blamePopup.run = run
  blamePopup.path = path
  blamePopup.message = run.summary
  blamePopup.open = true
  returnTo = back ?? null
  const ticket = ++asked
  // Read through libgit2, so a click on the column logs no git command.
  invoke<CommitDetail>('commit_detail', { oid: run.oid })
    .then((detail) => {
      if (ticket !== asked || !detail) return
      blamePopup.message = detail.body ? `${detail.summary}\n\n${detail.body}` : detail.summary
    })
    .catch(() => undefined)
  return true
}

/** Closes it; `refocus` hands focus back to the editor it was opened from. */
export function closeBlamePopup(refocus = false) {
  if (!blamePopup.open) return
  blamePopup.open = false
  asked += 1
  const back = returnTo
  returnTo = null
  if (refocus && back?.isConnected) back.focus({ preventScroll: true })
}

/** What the commit changed in this file, in a tab of its own; the file stays open. */
export function showBlamedChange() {
  const run = blamePopup.run
  const path = blamePopup.path
  closeBlamePopup()
  if (run && path) openTab({ path, commit: run.oid, mode: 'diff' })
}

export function showBlamedInGraph() {
  const run = blamePopup.run
  closeBlamePopup()
  openBlamed(run)
}

export function copyBlamedHash(refocus = true) {
  const run = blamePopup.run
  if (!run) return
  void copyText(run.oid, 'Commit hash')
  closeBlamePopup(refocus)
}

/**
 * The right-click menu of the line numbers and the column beside them.
 *
 * `run` is the entry under the pointer when it was the column that was
 * right-clicked, which adds what can be done with that commit.
 */
export function blameMenu(on: boolean, toggle: () => void, run?: BlameRun | null): MenuItem[] {
  const items: MenuItem[] = []
  if (on && run && !run.uncommitted) {
    items.push(
      { label: 'Copy commit hash', icon: Copy, hint: run.short, action: () => copyText(run.oid, 'Commit hash') },
      { label: 'Show in graph', icon: GitCommitHorizontal, action: () => openBlamed(run) },
      { label: '', separator: true }
    )
  }
  items.push({
    label: on ? 'Close Annotations' : 'Annotate with Git Blame',
    icon: Users,
    hint: on ? undefined : 'Who last touched each line',
    action: toggle
  })
  return items
}
