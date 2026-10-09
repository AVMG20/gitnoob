/**
 * Finding text in the file that is open.
 *
 * The views draw only the rows on screen, so the browser's own find could never
 * see most of the file even if the window had one. The search runs over the
 * plain lines instead, which is what the count and the scrolling are worked out
 * from, and each row that is drawn marks its own matches in the coloured HTML
 * it was going to draw anyway.
 *
 * Kept apart from the components because the HTML half is the part that is
 * easy to get subtly wrong — a match that runs across two coloured tokens, or
 * over an `&lt;` — and it is the part a test can reach without a browser.
 */

/**
 * What a view is handed to mark: the search, and where the current match is.
 *
 * `row` counts the rows the view draws — lines of the file in the file view,
 * lines and hunk headings together in the patch — which is the same list the
 * search was run over.
 */
export interface FindMarks {
  query: string
  matchCase: boolean
  row: number | null
  nth: number
}

/** One match: which row it is on, and which of that row's matches it is. */
export interface Hit {
  row: number
  nth: number
}

/**
 * Lower-cased one character at a time, so every offset into the result is an
 * offset into what it was made from. A handful of characters grow when lowered
 * (`İ` becomes two), and those are left as they are rather than shifting
 * everything after them.
 */
function fold(text: string): string {
  let out = ''
  for (const char of text) {
    const lower = char.toLowerCase()
    out += lower.length === char.length ? lower : char
  }
  return out
}

/** Where `query` occurs in `text`, left to right and never overlapping. */
export function occurrences(text: string, query: string, matchCase = false): [number, number][] {
  if (!query) return []
  const hay = matchCase ? text : fold(text)
  const needle = matchCase ? query : fold(query)
  const found: [number, number][] = []
  let from = 0
  for (;;) {
    const at = hay.indexOf(needle, from)
    if (at < 0) break
    found.push([at, at + needle.length])
    from = at + needle.length
  }
  return found
}

/** Every match in a list of lines, in reading order. */
export function findIn(lines: string[], query: string, matchCase = false): Hit[] {
  const hits: Hit[] = []
  if (!query) return hits
  lines.forEach((line, row) => {
    occurrences(line, query, matchCase).forEach((_, nth) => hits.push({ row, nth }))
  })
  return hits
}

const NAMED: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'" }

/** What one entity stands for, or the entity itself when it is not one this knows. */
function decode(entity: string): string {
  const body = entity.slice(1, -1)
  if (body.startsWith('#x') || body.startsWith('#X')) {
    return String.fromCodePoint(Number.parseInt(body.slice(2), 16))
  }
  if (body.startsWith('#')) return String.fromCodePoint(Number.parseInt(body.slice(1), 10))
  return NAMED[body] ?? entity
}

/**
 * One line of coloured HTML with every match of `query` wrapped in a `<mark>`.
 *
 * The text is read out of the HTML itself rather than taken from the plain line,
 * so the marks land on what is actually drawn. A match that runs across two
 * coloured tokens is closed before the tag between them and opened again after
 * it, which keeps the HTML nested the way it was. `current` is which of this
 * line's matches is the one the arrows are on, if any.
 */
export function markHtml(
  html: string,
  query: string,
  matchCase = false,
  current: number | null = null
): string {
  if (!query || !html) return html

  // The HTML in pieces: tags as they are, and the text one character at a
  // time, with an entity counted as the one character it stands for.
  const pieces: { raw: string; text: string | null }[] = []
  const pattern = /<[^>]*>|&#?[a-zA-Z0-9]+;|[\s\S]/gu
  for (const [raw] of html.matchAll(pattern)) {
    if (raw.startsWith('<') && raw.length > 1) pieces.push({ raw, text: null })
    else if (raw.startsWith('&') && raw.length > 1) pieces.push({ raw, text: decode(raw) })
    else pieces.push({ raw, text: raw })
  }

  const text = pieces.map((piece) => piece.text ?? '').join('')
  const found = occurrences(text, query, matchCase)
  if (!found.length) return html

  let out = ''
  let offset = 0
  let open = false
  let which = 0
  for (const piece of pieces) {
    if (piece.text === null) {
      if (open) {
        out += '</mark>'
        open = false
      }
      out += piece.raw
      continue
    }
    while (which < found.length && offset >= found[which]![1]) which++
    const range = found[which]
    const inside = !!range && offset >= range[0] && offset < range[1]
    if (inside && !open) {
      out += which === current ? '<mark class="find-hit now">' : '<mark class="find-hit">'
      open = true
    } else if (!inside && open) {
      out += '</mark>'
      open = false
    }
    out += piece.raw
    offset += piece.text.length
    // The end of one match can be the start of the next, which wants a mark
    // of its own rather than one long one.
    if (open && range && offset >= range[1]) {
      out += '</mark>'
      open = false
    }
  }
  if (open) out += '</mark>'
  return out
}
