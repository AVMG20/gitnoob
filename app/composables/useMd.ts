import MarkdownIt from 'markdown-it'
import type { StateCore, Token } from 'markdown-it'

/** The renderer itself, which the package exports as a value rather than a type. */
type Md = InstanceType<typeof MarkdownIt>

import { highlightBlock } from './useHighlight'

/**
 * The markdown a forge comment was written in, drawn as elements.
 *
 * Bodies come from GitHub and GitLab, where people write full markdown —
 * tables, nested lists, task lists, images, the template's own HTML comments —
 * so the renderer has to be one that knows all of it rather than a handful of
 * regular expressions. markdown-it is that renderer.
 *
 * People also write plain HTML in those bodies, because both forges render it:
 * a `<details>` fold around a long log, a `<p align="center">` badge row, a
 * `<img width="600">` screenshot, a `<kbd>`. That used to arrive here as
 * literal `&lt;details&gt;`, which is worse than not supporting it. So raw
 * markup is read now — but never carried across. Every tag in the source is
 * taken apart, checked against a fixed vocabulary of names and attributes, and
 * written out again from that vocabulary; nothing is copied through. So the
 * promise the old `html: false` made still holds word for word: every tag in
 * the output is one we emitted.
 *
 * What is added on top is the part a forge does and CommonMark does not:
 * `@name` and `#123` become chips, task list items become boxes, links leave
 * for the browser, and a fence with a language we have is coloured.
 */

/** http, https and mailto are the only schemes a comment can make clickable. */
const SAFE_SCHEME = /^(?:https?|mailto):/i

/** An image is fetched rather than followed, so only the two that fetch. */
const IMAGE_SCHEME = /^https?:/i

/**
 * Templates the forge fills a new request's body with are mostly instructions
 * to the author, wrapped in HTML comments so they do not show.
 *
 * markdown-it would drop most of these itself now that markup is read, but not
 * all of them — a comment that opens and is never closed, or one sitting where
 * the parser wants inline content — and the ones it keeps would print. So they
 * still go before the parser sees them.
 *
 * Fenced code is cut out first: a comment body explaining HTML comments should
 * still be able to print one.
 */
function withoutComments(source: string): string {
  const fence = /^[ \t]*(?:```|~~~).*$/gm
  const bounds: [number, number][] = []
  let open: number | null = null
  for (const mark of source.matchAll(fence)) {
    if (open === null) open = mark.index
    else {
      bounds.push([open, mark.index + mark[0].length])
      open = null
    }
  }
  // A fence left open runs to the end of the text, the way the parser reads it.
  if (open !== null) bounds.push([open, source.length])

  const strip = (text: string) =>
    text
      .replace(/<!--[\s\S]*?-->/g, '')
      // A comment nobody closed swallows the rest of the body in a browser;
      // here it would print. Neither is useful, so it ends at the line.
      .replace(/<!--[^\n]*$/gm, '')

  let out = ''
  let at = 0
  for (const [from, to] of bounds) {
    out += strip(source.slice(at, from)) + source.slice(from, to)
    at = to
  }
  return out + strip(source.slice(at))
}

/* ---------------------------------------------------------------------------
   The vocabulary raw markup is rebuilt from.

   Three lists, and everything not in any of them is unwrapped: the tag goes
   and its text stays. That is the right default — an unknown wrapper is
   almost always a layout `<div>` somebody nested, and losing the box while
   keeping the words loses nothing worth reading.
   --------------------------------------------------------------------------- */

/** Reads one attribute's value, and answers with what to write, or nothing. */
type Attribute = (value: string) => string | null

/**
 * A lookup that answers only for what was actually written down.
 *
 * A plain object inherits from `Object.prototype`, so `KEEP['constructor']`
 * answers with a function rather than with nothing — which would let a tag
 * nobody allowed through the one gate that decides what is allowed, and would
 * hand `build` something to call that is not an attribute reader. Every table
 * in this file is read through here.
 */
function known<T>(table: Record<string, T>, key: string): T | undefined {
  return Object.prototype.hasOwnProperty.call(table, key) ? table[key] : undefined
}

/** Anything, since it is written back out escaped. */
const TEXT: Attribute = (value) => value

/** Present or absent; the value a boolean attribute carries means nothing. */
const FLAG: Attribute = () => ''

/** `colspan`, `rowspan`, `start`: a small count, not an expression. */
const COUNT: Attribute = (value) => (/^\d{1,3}$/.test(value) ? value : null)

/** `width` and `height`, in pixels or per cent, as a forge writes them. */
const SIZE: Attribute = (value) => (/^\d{1,4}%?$/.test(value) ? value : null)

/** The four values `align` has ever meant anything as. */
const ALIGN: Attribute = (value) =>
  /^(?:left|right|center|justify)$/i.test(value) ? value.toLowerCase() : null

// Both of these are handed a value the caller has already decoded once, so
// neither decodes again.
const HREF: Attribute = (value) => {
  const url = cleanUrl(value)
  return SAFE_SCHEME.test(url) ? url : null
}

const SRC: Attribute = (value) => {
  const url = cleanUrl(value)
  return IMAGE_SCHEME.test(url) ? url : null
}

/**
 * Tags that are kept, and what each may carry.
 *
 * Nothing is allowed globally. `class`, `id`, `style`, `data-*` and every
 * `on*` are absent from every entry on purpose: `style` alone would let a
 * comment restyle the window it is being read in, and the release policy
 * permits inline style even though it forbids inline script.
 */
const KEEP: Record<string, Record<string, Attribute>> = {
  // Structure.
  p: { align: ALIGN },
  div: { align: ALIGN },
  span: {},
  br: {},
  hr: {},
  h1: { align: ALIGN },
  h2: { align: ALIGN },
  h3: { align: ALIGN },
  h4: { align: ALIGN },
  h5: { align: ALIGN },
  h6: { align: ALIGN },
  blockquote: {},
  pre: {},
  code: {},
  ul: {},
  ol: { start: COUNT, reversed: FLAG },
  li: {},
  dl: {},
  dt: {},
  dd: {},
  details: { open: FLAG },
  summary: {},

  // Inline.
  b: {},
  strong: {},
  i: {},
  em: {},
  s: {},
  del: {},
  strike: {},
  u: {},
  ins: {},
  mark: {},
  sub: {},
  sup: {},
  kbd: {},
  samp: {},
  var: {},
  small: {},
  q: {},
  abbr: { title: TEXT },
  cite: {},

  // Tables, which is most of what a bot writes.
  table: { align: ALIGN },
  thead: {},
  tbody: {},
  tfoot: {},
  caption: { align: ALIGN },
  tr: {},
  th: { align: ALIGN, colspan: COUNT, rowspan: COUNT },
  td: { align: ALIGN, colspan: COUNT, rowspan: COUNT },

  // The two that reach the network, and so carry the same policy the markdown
  // path applies. Both are finished off in `build`, which adds what they are
  // never allowed to be without.
  a: { href: HREF, title: TEXT },
  img: { src: SRC, alt: TEXT, title: TEXT, width: SIZE, height: SIZE }
}

/** Kept tags that never have a closing half, so nothing waits on one. */
const VOID = new Set(['br', 'hr', 'img'])

/**
 * Tags whose content goes with them.
 *
 * Either it is not prose (`script`, `style`, the head of a document), or it is
 * a way to draw something this window has no business drawing (`iframe`,
 * `object`), or it is a parser corner that browsers famously disagree about
 * (`svg`, `math`, `template`, `noscript`) and disagreement is where markup
 * that reads as text to one reader and as a tag to another comes from.
 */
const SILENCED = new Set([
  'script',
  'style',
  'iframe',
  'frame',
  'frameset',
  'object',
  'embed',
  'applet',
  'noscript',
  'noembed',
  'noframes',
  'template',
  'svg',
  'math',
  'textarea',
  'title',
  'xmp',
  'plaintext',
  'form',
  'select',
  'option',
  'optgroup',
  'button',
  'canvas',
  'map',
  'marquee',
  'dialog',
  'slot',
  'head'
])

/** The same idea, for tags that have no content to take with them. */
const SILENCED_ALONE = new Set([
  'meta',
  'link',
  'base',
  'source',
  'input',
  'param',
  'track',
  'area',
  'col'
])

/** How deep markup may nest before the rest of it is read as text. */
const DEPTH_LIMIT = 100

/* ---------------------------------------------------------------------------
   Reading a tag.
   --------------------------------------------------------------------------- */

interface Tag {
  closing: boolean
  name: string
  attributes: [string, string][]
  /** Written `<x />`, so nothing is waiting to be closed. */
  shut: boolean
  /** Where in the source the tag ends. */
  end: number
}

/**
 * Reads one tag out of `source` at `start`, or answers nothing.
 *
 * Deliberately more forgiving than a browser about where a tag ends and
 * stricter about what a name is: `<b <img src=x onerror=alert(1)>` is read
 * here as a `b` carrying three attributes nobody allowed, which is then
 * written back out as a bare `<b>`. Reading it the other way round and copying
 * the original bytes is the whole bug class this avoids.
 */
function scanTag(source: string, start: number): Tag | null {
  let at = start + 1
  let closing = false
  if (source[at] === '/') {
    closing = true
    at += 1
  }
  const from = at
  while (at < source.length && /[A-Za-z0-9-]/.test(source[at]!)) at += 1
  const name = source.slice(from, at).toLowerCase()
  if (!/^[a-z][a-z\d-]*$/.test(name)) return null

  const attributes: [string, string][] = []
  for (;;) {
    while (at < source.length && /\s/.test(source[at]!)) at += 1
    if (at >= source.length) return null
    if (source[at] === '>') return { closing, name, attributes, shut: false, end: at + 1 }
    if (source[at] === '/' && source[at + 1] === '>') {
      return { closing, name, attributes, shut: true, end: at + 2 }
    }
    // A closing tag carries nothing, so anything here means this is not one.
    if (closing) return null
    // `<img/src=x>` is a tag to every browser and to both forges, so a stray
    // slash between attributes is skipped rather than ending the read. Only
    // reached for markup already inside a block markdown-it opened: on its own
    // markdown-it does not see it as a tag either, and it stays escaped text.
    if (source[at] === '/') {
      at += 1
      continue
    }

    const keyFrom = at
    while (at < source.length && !/[\s/>=]/.test(source[at]!)) at += 1
    if (at === keyFrom) return null
    const key = source.slice(keyFrom, at).toLowerCase()

    let value = ''
    let seek = at
    while (seek < source.length && /\s/.test(source[seek]!)) seek += 1
    if (source[seek] === '=') {
      seek += 1
      while (seek < source.length && /\s/.test(source[seek]!)) seek += 1
      const quote = source[seek]
      if (quote === '"' || quote === "'") {
        const shut = source.indexOf(quote, seek + 1)
        if (shut === -1) return null
        value = source.slice(seek + 1, shut)
        at = shut + 1
      } else {
        const valueFrom = seek
        while (seek < source.length && !/[\s>]/.test(source[seek]!)) seek += 1
        if (seek === valueFrom) return null
        value = source.slice(valueFrom, seek)
        at = seek
      }
    }
    attributes.push([key, value])
  }
}

/**
 * The named characters worth knowing, which is the ones a scheme is built of.
 *
 * Anything else stays written as it was, which leaves the value failing the
 * scheme test — the safe way for a gap in this table to fail.
 */
const NAMED: Record<string, string> = {
  amp: '&',
  lt: '<',
  gt: '>',
  quot: '"',
  apos: "'",
  nbsp: ' ',
  Tab: '\t',
  NewLine: '\n',
  colon: ':',
  sol: '/',
  period: '.',
  num: '#',
  semi: ';',
  equals: '='
}

const ENTITY = /&(?:#(\d{1,7})|#[xX]([\da-fA-F]{1,6})|([a-zA-Z][a-zA-Z\d]{1,31}));/g

function character(code: number): string | null {
  if (!Number.isFinite(code) || code <= 0 || code > 0x10ffff) return null
  // Half of a pair, on its own, is not a character.
  if (code >= 0xd800 && code <= 0xdfff) return null
  return String.fromCodePoint(code)
}

/**
 * An attribute's value as the browser will read it, not as it was written.
 *
 * Done exactly once, on the way in, and never again: a value decoded twice
 * reads `&amp;#106;` as `j`, which is a way of writing `javascript:` that
 * survives a test the once-decoded form fails.
 */
function decodeEntities(value: string): string {
  return value.replace(ENTITY, (all, decimal, hex, name) => {
    if (typeof decimal === 'string') return character(Number.parseInt(decimal, 10)) ?? all
    if (typeof hex === 'string') return character(Number.parseInt(hex, 16)) ?? all
    return (typeof name === 'string' ? known(NAMED, name) : undefined) ?? all
  })
}

/**
 * A url with what a browser ignores taken out of it, ready to be tested.
 *
 * A browser skips whitespace and control characters before it reads a scheme,
 * so `java\tscript:x` is `javascript:x` by the time anything is followed, and
 * a test run on the written form would pass it and fail only the one spelled
 * plainly.
 */
function cleanUrl(value: string): string {
  return value.replace(/[\s\p{Cc}]/gu, '')
}

/** Both halves, for the markdown path, where nothing has decoded yet. */
function tidyUrl(value: string): string {
  return cleanUrl(decodeEntities(value))
}

function escapeText(value: string): string {
  return value.replace(/[&<>"]/g, (character) => {
    if (character === '&') return '&amp;'
    if (character === '<') return '&lt;'
    if (character === '>') return '&gt;'
    return '&quot;'
  })
}

/**
 * Writes one allowed tag out, from its name and the attributes that survived.
 *
 * Answers nothing when the tag is not worth drawing without what it lost: a
 * link whose address failed the scheme test would be a dead one, and an image
 * with no source would be a broken icon, so both are unwrapped instead.
 */
function build(tag: Tag): string | null {
  const allowed = known(KEEP, tag.name)
  if (!allowed) return null
  const written: string[] = []
  const seen = new Set<string>()

  for (const [key, raw] of tag.attributes) {
    if (seen.has(key)) continue
    const read = known(allowed, key)
    // Nothing but a reader written down in the table above is called, and
    // nothing but a string it answered with is written out.
    if (typeof read !== 'function') continue
    const value = read(decodeEntities(raw))
    if (typeof value !== 'string') continue
    seen.add(key)
    written.push(value === '' ? key : `${key}="${escapeText(value)}"`)
  }

  if (tag.name === 'a') {
    if (!seen.has('href')) return null
    // The same policy the markdown path applies at `link_open`: a comment's
    // link leaves for the browser rather than replacing the window.
    written.push('target="_blank"', 'rel="noopener noreferrer"')
  }
  if (tag.name === 'img') {
    if (!seen.has('src')) return null
    if (!seen.has('alt')) written.push('alt=""')
    written.push('loading="lazy"', 'referrerpolicy="no-referrer"')
  }

  return written.length ? `<${tag.name} ${written.join(' ')}>` : `<${tag.name}>`
}

/** What an image that cannot be drawn leaves behind, so it is not just gone. */
function altOf(tag: Tag): string {
  const alt = tag.attributes.find(([key]) => key === 'alt')?.[1] ?? ''
  return alt.trim() ? decodeEntities(alt) : ''
}

/**
 * What the walk over one body has open, and what it is currently ignoring.
 *
 * Each open tag remembers whether it is allowed to stay open past the end of
 * the block it was written in. Only a tag written as its own block may: a
 * `<details>` fold and a hand-written `<table>` really do wrap several
 * paragraphs, and both arrive that way. A `<td>` written in the middle of a
 * list item does not, and letting one span would put its closing half after
 * the list had already closed.
 */
interface Open {
  name: string
  spans: boolean
  /** How deep in the markdown it was opened, so it closes no later. */
  depth: number
}

interface Walk {
  stack: Open[]
  silenced: { name: string; depth: number } | null
  /** How many markdown blocks are open around whatever is being read. */
  depth: number
}

/**
 * Rebuilds one run of raw markup, keeping the walk's open tags up to date.
 *
 * The walk is shared across the whole body rather than per chunk, because a
 * fold is written as three separate ones: `<details><summary>…</summary>`, the
 * markdown inside it, and `</details>`.
 */
function rebuild(source: string, walk: Walk, fromBlock: boolean): string {
  let out = ''
  let text = ''
  let at = 0

  const flush = () => {
    if (!text) return
    if (!walk.silenced) out += escapeText(text)
    text = ''
  }

  while (at < source.length) {
    const mark = source.indexOf('<', at)
    if (mark === -1) {
      text += source.slice(at)
      break
    }
    text += source.slice(at, mark)

    // A comment, a declaration or a processing instruction: none of them are
    // content, and all of them end without being read. Each ends where its own
    // kind ends, which for the first two is not the next `>`.
    if (source.startsWith('<!--', mark)) {
      const shut = source.indexOf('-->', mark + 4)
      at = shut === -1 ? source.length : shut + 3
      continue
    }
    if (source.startsWith('<![CDATA[', mark)) {
      const shut = source.indexOf(']]>', mark + 9)
      at = shut === -1 ? source.length : shut + 3
      continue
    }
    if (source[mark + 1] === '!' || source[mark + 1] === '?') {
      const shut = source.indexOf('>', mark + 1)
      at = shut === -1 ? source.length : shut + 1
      continue
    }

    const tag = scanTag(source, mark)
    if (!tag) {
      // Not a tag at all, so it is the character it looks like.
      text += '<'
      at = mark + 1
      continue
    }
    at = tag.end

    if (walk.silenced) {
      // Inside something being ignored, only its own name means anything.
      if (tag.name === walk.silenced.name) {
        if (tag.closing) {
          walk.silenced.depth -= 1
          if (walk.silenced.depth === 0) walk.silenced = null
        } else if (!tag.shut) {
          walk.silenced.depth += 1
        }
      }
      text = ''
      continue
    }

    if (tag.closing) {
      if (!known(KEEP, tag.name) || VOID.has(tag.name)) continue
      const opened = walk.stack.findLastIndex((one) => one.name === tag.name)
      // A closing tag nothing opened closes nothing.
      if (opened === -1) continue
      flush()
      for (let back = walk.stack.length - 1; back >= opened; back -= 1) {
        out += `</${walk.stack[back]!.name}>`
      }
      walk.stack.length = opened
      continue
    }

    if (SILENCED_ALONE.has(tag.name)) continue
    if (LINKED.has(tag.name)) {
      // A recording cannot be played here — the release policy allows media
      // from nowhere but the app itself — so what is drawn is the way to it.
      // Better than the silence this used to be, which lost the address too.
      // Nothing is silenced with it: an unclosed `<video>` is followed by
      // ordinary prose, not by the fallback text a closed one wraps.
      const link = build({ ...tag, name: 'a', attributes: linkFor(tag) })
      if (!link) continue
      flush()
      out += `${link}${escapeText(fileNameIn(tag))}</a>`
      continue
    }
    if (SILENCED.has(tag.name)) {
      // What was written before it stays: only what it encloses goes with it.
      flush()
      if (!tag.shut) walk.silenced = { name: tag.name, depth: 1 }
      continue
    }
    if (!known(KEEP, tag.name)) continue
    if (walk.stack.length >= DEPTH_LIMIT) continue

    const written = build(tag)
    if (!written) {
      // An image whose address is not one that can be fetched still had
      // something to say about what it showed.
      const alt = tag.name === 'img' ? altOf(tag) : ''
      if (alt) text += alt
      continue
    }
    flush()
    const spans = fromBlock && SPANS_BLOCKS.has(tag.name)
    // A tag that outlives this block cannot be opened inside one that does
    // not: `<a href><td>` would leave the link open under the cell, out of
    // reach of the closing pass, and every paragraph after it inside the
    // link. So whatever cannot span is closed before the one that can opens.
    if (spans) out += closeInline(walk)
    out += written
    if (VOID.has(tag.name)) continue
    // `<b/>` is written by people who mean `<b></b>`, and leaving it open
    // would fold the rest of the line into it.
    if (tag.shut) out += `</${tag.name}>`
    else walk.stack.push({ name: tag.name, spans, depth: walk.depth })
  }

  flush()
  return out
}

/**
 * The tags allowed to stay open from one block to the next.
 *
 * A fold really does wrap several paragraphs, and so does a table cell, so the
 * open list has to survive between tokens for either to work. Nothing inline
 * does: `<b>` at the top of a body with no closing half used to make the whole
 * comment bold, and an unclosed `<a href>` used to make the whole comment one
 * link to wherever it pointed — which is a comment-shaped phishing button.
 */
const SPANS_BLOCKS = new Set([
  'details',
  'div',
  'blockquote',
  'table',
  'thead',
  'tbody',
  'tfoot',
  'caption',
  'tr',
  'th',
  'td',
  'ul',
  'ol',
  'li',
  'dl',
  'dt',
  'dd',
  'pre'
])

/**
 * Closes everything this block opened that is not allowed to outlive it.
 *
 * Only has to look at the top of the stack, because nothing that cannot span
 * is ever left underneath something that can: opening one closes the other
 * first.
 */
function closeInline(walk: Walk): string {
  let out = ''
  while (walk.stack.length && !walk.stack[walk.stack.length - 1]!.spans) {
    out += `</${walk.stack.pop()!.name}>`
  }
  return out
}

/**
 * Closes what a markdown block that is ending had opened inside it.
 *
 * A `<details>` fold written at the top of a body is one thing; a `<td>`
 * written in the middle of a list item is another, and both arrive as raw
 * blocks. What separates them is how deep in the markdown each was written:
 * the fold outlives the paragraphs it wraps, and the cell cannot outlive the
 * item it was typed into, because its closing half would then land after the
 * list itself had closed.
 */
function closeDeeper(walk: Walk, depth: number): string {
  let out = ''
  while (walk.stack.length && walk.stack[walk.stack.length - 1]!.depth > depth) {
    out += `</${walk.stack.pop()!.name}>`
  }
  return out
}

/** Closes whatever the body left open, so it cannot fold what comes after it. */
function closeAll(walk: Walk): string {
  let out = ''
  while (walk.stack.length) out += `</${walk.stack.pop()!.name}>`
  return out
}

/**
 * The tags drawn as a link to what they would have played.
 *
 * GitHub writes `<video src="…">` into a body for a screen recording, which is
 * a real part of a great many reviews.
 */
const LINKED = new Set(['video', 'audio'])

function linkFor(tag: Tag): [string, string][] {
  const src = tag.attributes.find(([key]) => key === 'src')?.[1] ?? ''
  return [['href', src]]
}

function fileNameIn(tag: Tag): string {
  const src = tag.attributes.find(([key]) => key === 'src')?.[1] ?? ''
  const path = src.split(/[?#]/)[0] ?? ''
  let readable = path
  try {
    readable = decodeURIComponent(path)
  } catch {
    // A half-written escape is not worth failing over; the raw path reads.
  }
  return readable.split('/').filter(Boolean).pop() || 'the recording'
}

/**
 * Rebuilds every raw tag in the body, in the order the body is read.
 *
 * Runs before the chips and the task boxes, so that by the time either of them
 * pushes a token of its own there is no `html_block` or `html_inline` left for
 * one to be mistaken for.
 */
const safeHtml = (md: Md) => {
  md.core.ruler.push('safe_html', (state: StateCore) => {
    const walk: Walk = { stack: [], silenced: null, depth: 0 }
    const kept: Token[] = []

    /** Whether each open block token was dropped, so its close goes too. */
    const hidden: boolean[] = []
    const inside = () => hidden[hidden.length - 1] ?? false

    const written = (content: string) => {
      const token = new state.Token('safe_html', '', 0)
      token.content = content
      return token
    }

    for (const token of state.tokens) {
      // Raw markup, rebuilt. This is also where a silenced tag opens and
      // closes, so it runs whether or not something is currently silenced.
      if (token.type === 'html_block') {
        const content = rebuild(token.content, walk, true) + closeInline(walk)
        // A silence runs to the end of the block that opened it and no
        // further. A `<style>` element really is one block, so it is silenced
        // whole; the word `<style>` in the middle of a sentence is not, and
        // used to delete every paragraph after the one it appeared in.
        walk.silenced = null
        if (content) kept.push(written(content))
        continue
      }

      // An inline run still has to be read while something is silenced: the
      // tag that ends the silence may be sitting in it, with ordinary text on
      // either side that belongs to opposite sides of the answer.
      if (token.type === 'inline' && token.children) {
        const children: Token[] = []
        for (const child of token.children) {
          if (child.type === 'html_inline') {
            const content = rebuild(child.content, walk, false)
            if (content) children.push(written(content))
            continue
          }
          // Markdown written inside a silenced tag is silenced with it.
          if (walk.silenced) continue
          children.push(child)
        }
        const shut = closeInline(walk)
        if (shut) children.push(written(shut))
        walk.silenced = null
        token.children = children
        if (!inside()) kept.push(token)
        continue
      }

      // markdown-it writes a column's alignment as `style="text-align:…"`,
      // which is the one attribute this file says nothing may ever carry. The
      // value is its own and not a commenter's, but an exception nobody can
      // see is worse than none: it says the same thing as `align`, which is
      // allowed, is styled already, and is what a forge writes by hand.
      if (token.type === 'th_open' || token.type === 'td_open') {
        const style = String(token.attrGet('style') ?? '')
        const side = /text-align:\s*(left|right|center)/.exec(style)?.[1]
        if (side) {
          token.attrs = (token.attrs ?? []).filter(([name]) => name !== 'style')
          token.attrSet('align', side)
        }
      }

      // Everything a silenced tag encloses goes with it. Whether a block was
      // dropped is remembered rather than recomputed, so that a silence which
      // ends halfway through one does not leave its closing half behind.
      if (token.nesting === 1) {
        hidden.push(walk.silenced !== null)
        walk.depth += 1
        if (!inside()) kept.push(token)
        continue
      }
      if (token.nesting === -1) {
        const hide = hidden.pop()
        walk.depth -= 1
        // Before the block's own closing half, not after it.
        const shut = closeDeeper(walk, walk.depth)
        if (!hide) {
          if (shut) kept.push(written(shut))
          kept.push(token)
        }
        continue
      }
      if (!walk.silenced) kept.push(token)
    }

    const rest = closeAll(walk)
    if (rest) kept.push(written(rest))

    state.tokens = kept
    return true
  })
}

/**
 * `@name` and `#123`, which mean something on a forge and nothing here.
 *
 * A name may hold a dot or a hyphen but cannot end on one: the sentence
 * "unblocks #71." ends in a full stop, and a pattern that swallowed it left a
 * number that no longer read as one — so the chip was silently dropped.
 */
const CHIP = /(^|[^\w`/])([@#])([A-Za-z0-9](?:[\w.-]*[A-Za-z0-9_])?)/g

/**
 * Turns mentions and request numbers into chips.
 *
 * Done over the parsed tokens rather than over the finished HTML, so a `@` in
 * a code span, in a URL or in a link's own text is left alone — the token
 * stream already knows which is which, and a regular expression over HTML does
 * not.
 */
const chips = (md: Md) => {
  md.core.ruler.push('forge_chips', (state: StateCore) => {
    for (const token of state.tokens) {
      if (token.type !== 'inline' || !token.children) continue
      const out: Token[] = []
      let inLink = 0
      for (const child of token.children) {
        if (child.type === 'link_open') inLink += 1
        if (child.type === 'link_close') inLink -= 1
        if (child.type !== 'text' || inLink > 0 || !CHIP.test(child.content)) {
          out.push(child)
          continue
        }
        CHIP.lastIndex = 0
        let at = 0
        for (const found of child.content.matchAll(CHIP)) {
          const all = found[0]
          const lead = found[1] ?? ''
          const mark = found[2] ?? ''
          const name = found[3] ?? ''
          // `#` in front of anything but a number is a heading mark or a URL
          // fragment, never a request on the forge.
          if (mark === '#' && !/^\d+$/.test(name)) continue
          const before = child.content.slice(at, found.index) + lead
          if (before) {
            const text = new state.Token('text', '', 0)
            text.content = before
            out.push(text)
          }
          const chip = new state.Token('safe_html', '', 0)
          const kind = mark === '@' ? 'mention' : 'ref'
          chip.content = `<span class="${kind}">${md.utils.escapeHtml(mark + name)}</span>`
          out.push(chip)
          at = found.index + all.length
        }
        const rest = child.content.slice(at)
        if (rest) {
          const text = new state.Token('text', '', 0)
          text.content = rest
          out.push(text)
        }
      }
      token.children = out
    }
    return true
  })
}

/** `- [ ]` and `- [x]`, which every forge draws as a box. */
const taskLists = (md: Md) => {
  md.core.ruler.push('task_lists', (state: StateCore) => {
    const tokens = state.tokens
    for (let at = 0; at < tokens.length; at += 1) {
      if (tokens[at]!.type !== 'list_item_open') continue
      const inline = tokens[at + 2]
      if (!inline || inline.type !== 'inline' || !inline.children?.length) continue
      const first = inline.children[0]!
      if (first.type !== 'text') continue
      const box = first.content.match(/^\[([ xX])\]\s+/)
      if (!box) continue

      first.content = first.content.slice(box[0].length)
      const ticked = box[1] !== ' '
      const mark = new state.Token('safe_html', '', 0)
      mark.content = `<span class="task-box${ticked ? ' done' : ''}" aria-hidden="true"></span>`
      inline.children.unshift(mark)
      tokens[at]!.attrJoin('class', 'task')
    }
    return true
  })
}

const md = new MarkdownIt({
  // Markup in a body is read — and then rebuilt from a fixed vocabulary by the
  // `safe_html` rule below, which is the only thing that makes this safe.
  html: true,
  // A bare URL is a link, because people paste them bare.
  linkify: true,
  // Forges break on a single newline and so does everyone writing for them.
  // CommonMark does not, which is why an address list used to run together.
  breaks: true,
  typographer: false,
  highlight: (code, info) => {
    const language = info.trim().split(/\s+/)[0] ?? ''
    // The fence's content carries the newline that closed it, which would
    // otherwise be drawn as a blank last line of the block.
    const painted = highlightBlock(code.replace(/\n$/, ''), language)
    const attribute = /^[\w+.#-]+$/.test(language) ? ` class="language-${language}"` : ''
    return `<pre><code${attribute}>${painted}</code></pre>`
  }
})

// Anything but http, https and mailto is left as text, so a `javascript:` url
// cannot be clicked and a `file:` one cannot be reached.
md.validateLink = (url) => SAFE_SCHEME.test(tidyUrl(url))

md.use(safeHtml).use(chips).use(taskLists)

// The one channel markup is allowed to arrive on, and it carries only strings
// built here.
md.renderer.rules.safe_html = (tokens, at) => tokens[at]!.content

// Nothing should reach either of these: `safe_html` consumes every raw token
// before the renderer runs. If one ever does — a rule reordered, a plugin
// added, an exception swallowed — it arrives as the text it was written as,
// which is what this file did before it read markup at all.
md.renderer.rules.html_inline = (tokens, at) => md.utils.escapeHtml(tokens[at]!.content)
md.renderer.rules.html_block = md.renderer.rules.html_inline

// A link in a comment is a link to somewhere else: it opens in the browser
// rather than replacing the window the repository is in.
md.renderer.rules.link_open = (tokens, at, options, _env, self) => {
  tokens[at]!.attrSet('target', '_blank')
  tokens[at]!.attrSet('rel', 'noopener noreferrer')
  return self.renderToken(tokens, at, options)
}

// Screenshots are half of what a request's description is, so images are drawn
// — but they are fetched from somewhere else, so they are fetched late.
md.renderer.rules.image = (tokens, at, options, env, self) => {
  const token = tokens[at]!
  // The alt text is the token's own children, not an attribute, so the
  // default rule builds it — and a rule that only sets attributes has to too.
  token.attrSet('alt', self.renderInlineAsText(token.children ?? [], options, env))
  token.attrSet('loading', 'lazy')
  token.attrSet('referrerpolicy', 'no-referrer')
  return self.renderToken(tokens, at, options)
}

/** Renders a comment body to HTML that is safe to hand to `v-html`. */
export function renderMarkdown(source: string): string {
  if (!source?.trim()) return ''
  const prepared = withoutComments(source)
  if (!prepared.trim()) return ''
  return md.render(prepared).trim()
}
