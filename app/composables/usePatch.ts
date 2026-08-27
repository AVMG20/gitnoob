import type { DiffHunk, DiffLine } from './useGit'

/**
 * Turns a unified patch into the hunk shape every diff view here draws.
 *
 * The patches arrive as text — GitHub and GitLab both hand a review's changes
 * over as unified diffs rather than as structured lines — so they are read
 * into the same `FileDiff` hunks a local diff takes, and the same renderer
 * serves both without either knowing the difference.
 *
 * Line numbers are worked out from the hunk headers themselves, which carry
 * where each half starts: additions count up the new file's numbering,
 * deletions the old file's, context advances both.
 */
export function parsePatch(patch: string): { hunks: DiffHunk[]; truncated: number } {
  if (!patch.trim()) return { hunks: [], truncated: 0 }

  const lines = patch.split('\n')
  const hunks: DiffHunk[] = []
  let hunk: DiffHunk | null = null
  // Running numbers, carried between hunks so nothing depends on them
  // arriving in order.
  let oldNo = 0
  let newNo = 0

  for (const raw of lines) {
    if (raw.startsWith('@@')) {
      const start = readHeader(raw)
      hunk = { header: raw.trimEnd(), lines: [] }
      hunks.push(hunk)
      oldNo = start.old ?? 0
      newNo = start.now ?? 0
      continue
    }
    if (!hunk) continue
    // The "no newline" remark describes the neighbours rather than being one:
    // it is kept, with an origin that says so, and counts towards neither
    // file's numbering.
    if (raw.startsWith('\\')) {
      hunk.lines.push({
        origin: '\\',
        old_lineno: null,
        new_lineno: null,
        content: 'No newline at end of file'
      })
      continue
    }
    // A well-formed body never carries headers mid-hunk, but a truncated one
    // may end anywhere, and stray context past the last header would sit here
    // with nothing to hang from.
    const sign = raw.charAt(0)
    if (sign !== '+' && sign !== '-' && sign !== ' ') continue

    const content = raw.slice(1)
    const line: DiffLine = {
      origin: sign,
      old_lineno: sign === '+' ? null : oldNo || null,
      new_lineno: sign === '-' ? null : newNo || null,
      content
    }
    if (sign !== '+') oldNo += 1
    if (sign !== '-') newNo += 1
    hunk.lines.push(line)
  }
  return { hunks, truncated: 0 }
}

/** Reads `-old,count +new,count` out of a hunk header. */
function readHeader(header: string): { old?: number; now?: number } {
  const match = header.match(/@@\s*-(\d+)(?:,\d+)?\s*\+(\d+)(?:,\d+)?/)
  if (!match) return {}
  return { old: Number(match[1]), now: Number(match[2]) }
}
