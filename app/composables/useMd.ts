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
 * regular expressions. markdown-it is that renderer, run with `html: false`,
 * which is what keeps a commenter from arriving as markup: raw tags in the
 * source are escaped to text, and every tag in the output is one we emitted.
 *
 * What is added on top is the part a forge does and CommonMark does not:
 * `@name` and `#123` become chips, task list items become boxes, links leave
 * for the browser, and a fence with a language we have is coloured.
 */

/** http, https and mailto are the only schemes a comment can make clickable. */
const SAFE_SCHEME = /^(?:https?|mailto):/i

/**
 * Templates the forge fills a new request's body with are mostly instructions
 * to the author, wrapped in HTML comments so they do not show. With `html`
 * off they would show — escaped, as literal `<!--` — which is worse than the
 * markup we turned off. So they go before the parser sees them.
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

/**
 * The three tags that turn up in bodies often enough to be worth reading
 * through: a line break, and the fold people put long logs in. Their content
 * is what matters, so the tags go and the text stays.
 */
function unwrapKnownTags(source: string): string {
  return source
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/?(?:details|summary)(?:\s[^>]*)?>/gi, '\n')
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
          const chip = new state.Token('html_inline', '', 0)
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
      const mark = new state.Token('html_inline', '', 0)
      mark.content = `<span class="task-box${ticked ? ' done' : ''}" aria-hidden="true"></span>`
      inline.children.unshift(mark)
      tokens[at]!.attrJoin('class', 'task')
    }
    return true
  })
}

const md = new MarkdownIt({
  // The one setting that matters: raw HTML in a body is text, never markup.
  html: false,
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
md.validateLink = (url) => SAFE_SCHEME.test(url.trim())

md.use(chips).use(taskLists)

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
  const prepared = unwrapKnownTags(withoutComments(source))
  if (!prepared.trim()) return ''
  return md.render(prepared).trim()
}
