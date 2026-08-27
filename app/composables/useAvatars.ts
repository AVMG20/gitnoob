import { invoke } from './useInvoke'
import { reactive } from 'vue'

/**
 * Author pictures, asked for once per address.
 *
 * The graph asks for a picture while it is drawing, and it draws the same
 * hundred rows again on every scroll. So a lookup is a synchronous read of what
 * is already known, and the fetch that fills it in happens once in the
 * background: `''` until the answer arrives, then either a picture or `null`
 * for "there is none, draw the initials".
 */
const found = reactive(new Map<string, string | null>())
const asking = new Set<string>()

function key(email: string) {
  return email.trim().toLowerCase()
}

/**
 * The picture for an address, or null when there is none and undefined while
 * the question is still out.
 */
export function avatarFor(email: string): string | null | undefined {
  const id = key(email)
  if (!id) return null
  if (found.has(id)) return found.get(id)
  if (!asking.has(id)) {
    asking.add(id)
    invoke<string | null>('avatar', { email: id })
      .then((url) => found.set(id, url ?? null))
      // A lookup that fails is a lookup that found nothing: the initials are
      // drawn, and nothing is said about it. Nobody opened a git client to hear
      // that a picture is missing.
      .catch(() => found.set(id, null))
      .finally(() => asking.delete(id))
  }
  return undefined
}

/** Forgets every answer, so the next draw asks again. */
export function forgetAvatars() {
  found.clear()
  asking.clear()
}

/**
 * The two letters drawn when there is no picture.
 *
 * A name gives its first and last initial, an address the start of its local
 * part — enough to tell two people in the same history apart.
 */
export function initials(name: string, email: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean)
  if (words.length >= 2) {
    return (words[0]![0]! + words[words.length - 1]![0]!).toUpperCase()
  }
  const one = words[0] ?? email.split('@')[0] ?? ''
  return one.slice(0, 2).toUpperCase() || '?'
}

/**
 * A colour for an address, the same one every time.
 *
 * Spread around the wheel rather than picked from a list: two people whose
 * names begin with the same letter should not also share a colour.
 */
const tints = new Map<string, string>()

export function tint(email: string): string {
  const id = key(email)
  let colour = tints.get(id)
  if (!colour) {
    let hash = 0
    for (let i = 0; i < id.length; i++) hash = (hash * 31 + id.charCodeAt(i)) >>> 0
    colour = `hsl(${hash % 360} 42% 46%)`
    tints.set(id, colour)
  }
  return colour
}
