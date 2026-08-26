/**
 * A little markdown, for comment bodies.
 *
 * Forge comments are written in markdown but shown here in plain elements, and
 * the honest middle ground is a handful of transforms over escaped text: not
 * a renderer anyone would choose, but enough that a code snippet reads as one,
 * a quote sits apart and a link clicks. Everything is HTML-escaped before a
 * single tag is introduced, so nothing a commenter wrote can arrive as markup.
 */

/** Escapes the characters that would otherwise be read as markup. */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function inline(text: string): string {
  let out = escapeHtml(text)
  // Code spans are lifted out first and put back last: everything between
  // their marks is exactly what was written, so a list marker or an asterisk
  // inside one must survive untouched.
  const codeSpans: string[] = []
  out = out.replace(/`([^`]+)`/g, (_all, code: string) => {
    codeSpans.push(`<code>${code}</code>`)
    return `\u0000${codeSpans.length - 1}\u0000`
  })
  // Links: [text](url), http(s) only so nothing else is clickable.
  out = out.replace(
    /\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g,
    (_all, label: string, url: string) =>
      `<a href="${url}" target="_blank" rel="noopener noreferrer">${label}</a>`
  )
  // A bare URL becomes a link too, because people paste them bare.
  out = out.replace(
    /(^|[\s>])(https?:\/\/[^\s<]+)/g,
    (_all, lead: string, url: string) =>
      `${lead}<a href="${url}" target="_blank" rel="noopener noreferrer">${url}</a>`
  )
  out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
  out = out.replace(/(^|\W)\*([^*\n]+)\*(?=\W|$)/g, '$1<em>$2</em>')
  out = out.replace(/(^|\W)_([^_\n]+)_(?=\W|$)/g, '$1<em>$2</em>')
  out = out.replace(/~~([^~]+)~~/g, '<del>$1</del>')
  // `#123` refers to the issue or request with that number on the same forge.
  out = out.replace(/(^|\s)#(\d+)\b/g, '$1<span class="ref">#$2</span>')
  out = out.replace(
    /(^|\s)@([a-zA-Z0-9_-]+)/g,
    '$1<span class="mention">@$2</span>'
  )
  return out.replace(/\u0000(\d+)\u0000/g, (_all, at: string) => codeSpans[Number(at)]!)
}

/**
 * Renders a comment body to safe HTML.
 *
 * Block level: fenced code, headings, quotes, lists and rules are recognised;
 * everything else runs together as paragraphs with single newlines kept.
 */
export function renderMarkdown(source: string): string {
  if (!source.trim()) return ''
  const lines = source.replace(/\r\n/g, '\n').split('\n')
  const blocks: string[] = []
  let paragraph: string[] = []
  let list: 'ul' | 'ol' | null = null
  let quote: string[] = []
  let fence: { language: string; body: string[] } | null = null

  const flushParagraph = () => {
    if (paragraph.length) {
      blocks.push(`<p>${inline(paragraph.join('\n'))}</p>`)
      paragraph = []
    }
  }
  const flushList = () => {
    if (list) {
      blocks.push(`</${list}>`)
      list = null
    }
  }
  const flushQuote = () => {
    if (quote.length) {
      blocks.push(`<blockquote>${inline(quote.join('\n'))}</blockquote>`)
      quote = []
    }
  }

  for (const raw of lines) {
    if (fence) {
      // The closing fence carries the same three marks as the opening one.
      if (/^\s*```/.test(raw)) {
        const body = inline(fence.body.join('\n'))
        const language = fence.language ? ` class="language-${fence.language}"` : ''
        blocks.push(`<pre><code${language}>${body}</code></pre>`)
        fence = null
      } else {
        fence.body.push(raw)
      }
      continue
    }
    const opened = raw.match(/^\s*```\s*(\S*)\s*$/)
    if (opened) {
      flushParagraph()
      flushList()
      flushQuote()
      fence = { language: opened[1] ?? '', body: [] }
      continue
    }

    const trimmed = raw.trim()
    if (!trimmed) {
      flushParagraph()
      flushList()
      flushQuote()
      continue
    }

    const heading = trimmed.match(/^(#{1,4})\s+(.*)$/)
    if (heading) {
      flushParagraph()
      flushList()
      flushQuote()
      // Headings inside comments are noise at full size; every level lands on
      // the same two weights and only the size steps down.
      const weight = 15 - heading[1]!.length * 2
      blocks.push(
        `<div class="md-head" style="font-size:${weight}px">${inline(heading[2]!)}</div>`
      )
      continue
    }

    const item = trimmed.match(/^[-*]\s+(.*)$/) ?? trimmed.match(/^\d+[.)]\s+(.*)$/)
    if (item) {
      flushParagraph()
      flushQuote()
      const ordered = /^\d/.test(trimmed)
      if (list !== (ordered ? 'ol' : 'ul')) {
        flushList()
        list = ordered ? 'ol' : 'ul'
        blocks.push(`<${list}>`)
      }
      blocks.push(`<li>${inline(item[1]!)}</li>`)
      continue
    }

    if (trimmed.startsWith('&gt;') || trimmed.startsWith('>')) {
      flushParagraph()
      flushList()
      quote.push(trimmed.replace(/^&gt;?\s?/, '').replace(/^>\s?/, ''))
      continue
    }

    if (/^(-{3,}|\*{3,})$/.test(trimmed)) {
      flushParagraph()
      flushList()
      flushQuote()
      blocks.push('<hr/>')
      continue
    }

    flushList()
    flushQuote()
    paragraph.push(raw)
  }
  if (fence) {
    // Closed by the end of the text rather than by its own mark.
    blocks.push(`<pre><code>${inline(fence.body.join('\n'))}</code></pre>`)
  }
  flushParagraph()
  flushList()
  flushQuote()
  return blocks.join('')
}
