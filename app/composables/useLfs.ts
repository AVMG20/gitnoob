/**
 * Git LFS, as far as the window needs to know about it.
 *
 * What LFS commits is a three-line pointer, and what a checkout puts on disk is
 * either the real file or — when the objects were never fetched, or `git lfs`
 * is not installed — that same pointer. A viewer that cannot tell the
 * difference shows three lines of metadata and calls it the file.
 */

/** What an LFS pointer says about the file it stands in for. */
export interface LfsPointer {
  /** The object id, `sha256:…` as written. */
  oid: string
  /** How big the real file is, in bytes. */
  size: number
}

/** Whether this repository uses LFS, and whether the tool for it is here. */
export interface LfsStatus {
  in_use: boolean
  installed: boolean
}

/**
 * Reads a pointer, or null for anything that is an ordinary file.
 *
 * The format is specified: a `version` line naming the spec, then `oid` and
 * `size`. Anything not matching exactly is a file that merely looks like one.
 */
export function readPointer(text: string | null): LfsPointer | null {
  if (!text) return null
  const lines = text.split('\n')
  if (!lines[0]?.startsWith('version https://git-lfs.github.com/spec/')) return null

  let oid: string | null = null
  let size: number | null = null
  for (const line of lines.slice(1)) {
    if (!line.trim()) continue
    const at = line.indexOf(' ')
    if (at < 0) return null
    const key = line.slice(0, at)
    const value = line.slice(at + 1).trim()
    if (key === 'oid') oid = value
    else if (key === 'size') {
      const found = Number(value)
      size = Number.isInteger(found) && found >= 0 ? found : null
    }
  }
  return oid !== null && size !== null ? { oid, size } : null
}

/** A byte count as a person would say it. */
export function humanSize(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let size = bytes
  let at = 0
  while (size >= 1024 && at < units.length - 1) {
    size /= 1024
    at += 1
  }
  const rounded = size >= 100 || at === 0 ? Math.round(size) : Math.round(size * 10) / 10
  return `${rounded} ${units[at]}`
}
