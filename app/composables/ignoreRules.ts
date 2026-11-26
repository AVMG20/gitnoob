/**
 * The `.gitignore` lines the Files tree offers for a path.
 *
 * Every line is anchored with a leading `/`, so ignoring `config.php` at the
 * root does not also ignore `app/config.php`. Characters that mean something
 * to git's matcher are escaped, so a file named `a[1].txt` ignores itself and
 * not `a1.txt`.
 */

export interface IgnoreChoice {
  label: string
  pattern: string
}

/** A path written so git's matcher reads it as the name it is. */
export function escapeIgnore(path: string): string {
  let out = path.replace(/[\\*?[\]]/g, (char) => `\\${char}`)
  // Trailing spaces are dropped unless escaped.
  out = out.replace(/ +$/, (spaces) => spaces.replace(/ /g, '\\ '))
  // A line starting with `#` is a comment, with `!` a negation.
  if (/^[#!]/.test(out)) out = `\\${out}`
  return out
}

function parentOf(path: string) {
  const cut = path.lastIndexOf('/')
  return cut < 0 ? '' : path.slice(0, cut)
}

/** The extension of a file name, without the dot: none for `.env` or `Makefile`. */
export function extensionOf(path: string): string {
  const name = path.slice(path.lastIndexOf('/') + 1)
  const dot = name.lastIndexOf('.')
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1) : ''
}

/** What to offer for one path: itself, files like it, and the folder it is in. */
export function ignoreChoices(path: string, dir: boolean): IgnoreChoice[] {
  if (dir) return [{ label: `This folder (${path}/)`, pattern: `/${escapeIgnore(path)}/` }]
  const out: IgnoreChoice[] = [{ label: `This file (${path})`, pattern: `/${escapeIgnore(path)}` }]
  const ext = extensionOf(path)
  if (ext) out.push({ label: `All .${ext} files`, pattern: `*.${escapeIgnore(ext)}` })
  const parent = parentOf(path)
  if (parent) out.push({ label: `The folder ${parent}/`, pattern: `/${escapeIgnore(parent)}/` })
  return out
}
