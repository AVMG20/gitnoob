import { describe, expect, it } from 'vitest'
import { renderMarkdown } from '../app/composables/useMd'

/** Tags and entities out, so a test can say what a body reads as. */
const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, '')
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .trim()

describe('renderMarkdown', () => {
  it('carries text over as paragraphs, single newlines kept as breaks', () => {
    const html = renderMarkdown('one\ntwo\n\nthree')
    expect(html).toContain('<br>')
    expect(html).toMatch(/<p>one<br>\s*two<\/p>/)
    expect(html).toContain('<p>three</p>')
  })

  it('escapes markup before anything else is introduced', () => {
    const html = renderMarkdown('<script>alert(1)</script> & <b>bold</b>')
    expect(html).not.toContain('<script>')
    expect(html).not.toContain('<b>')
    expect(html).toContain('&lt;script&gt;')
    expect(html).toContain('&amp;')
  })

  it('keeps code exactly as written, in spans and fences', () => {
    expect(renderMarkdown('use `x < 3` here')).toContain('<code>x &lt; 3</code>')
    const html = renderMarkdown('```\nconst a = 1 < 2\n```')
    expect(html).toContain('<pre><code>const a = 1 &lt; 2</code></pre>')
  })

  it('names a fence language as a class and colours what it knows', () => {
    const known = renderMarkdown('```ts\nconst a = 1\n```')
    expect(known).toContain('<pre><code class="language-ts">')
    expect(known).toContain('hljs-keyword')
    // A language nothing is registered for is still labelled, never guessed at.
    const unknown = renderMarkdown('```wobble\nx\n```')
    expect(text(unknown)).toBe('x')
    expect(unknown).not.toContain('hljs-')
  })

  it('does not let a fence language break out of its attribute', () => {
    const spaced = renderMarkdown('```"><img src=x onerror=alert(document.domain)>\nx\n```')
    const packed = renderMarkdown('```"><img/src=x/onerror=alert(document.domain)>\nx\n```')
    const quoted = renderMarkdown('```ts"onmouseover="alert(1)\nx\n```')
    for (const html of [spaced, packed, quoted]) {
      expect(html).not.toContain('<img')
      expect(html).not.toContain('<svg')
      expect(html).not.toMatch(/class="language-/)
    }
  })

  it('links named and bare urls, and only schemes worth clicking', () => {
    const named = renderMarkdown('[site](https://example.com)')
    expect(named).toContain('href="https://example.com"')
    expect(named).toContain('target="_blank"')
    expect(named).toContain('rel="noopener noreferrer"')
    expect(renderMarkdown('see https://example.com/x')).toContain('href="https://example.com/x"')
    expect(renderMarkdown('write to <bob@example.com>')).toContain('href="mailto:bob@example.com"')
    for (const bad of ['[x](javascript:alert(1))', '[x](file:///etc/passwd)', '[x](data:text/html,x)']) {
      expect(renderMarkdown(bad)).not.toContain('<a')
    }
  })

  it('does not nest one link inside another', () => {
    const html = renderMarkdown('[https://example.com](https://example.com)')
    expect(html.match(/<a /g)).toHaveLength(1)
  })

  it('marks emphasis, deletions, mentions and references', () => {
    const html = renderMarkdown('**bold** and *em* and ~~gone~~\n@arno see #36')
    expect(html).toContain('<strong>bold</strong>')
    expect(html).toContain('<em>em</em>')
    expect(html).toContain('<s>gone</s>')
    expect(html).toContain('<span class="mention">@arno</span>')
    expect(html).toContain('<span class="ref">#36</span>')
  })

  it('chips every mention and reference in a line, punctuation and all', () => {
    const html = renderMarkdown('Thanks @nadia and @sam. Closes #64, and unblocks #71.')
    expect(html).toContain('<span class="mention">@nadia</span>')
    expect(html).toContain('<span class="mention">@sam</span>.')
    expect(html).toContain('<span class="ref">#64</span>,')
    // The full stop ending the sentence is not part of the number.
    expect(html).toContain('<span class="ref">#71</span>.')
  })

  it('leaves a mention alone inside code, inside a link and inside a url', () => {
    expect(renderMarkdown('`@arno`')).not.toContain('class="mention"')
    expect(renderMarkdown('[@arno](https://example.com/arno)')).not.toContain('class="mention"')
    expect(renderMarkdown('https://example.com/x#36')).not.toContain('class="ref"')
    // `#` in front of anything but a number is a heading mark or a fragment,
    // never a request number.
    expect(renderMarkdown('the #main branch')).not.toContain('class="ref"')
  })

  it('draws headings, quotes, rules and lists at every level', () => {
    expect(renderMarkdown('## Title')).toContain('<h2>Title</h2>')
    expect(renderMarkdown('###### Small')).toContain('<h6>Small</h6>')
    expect(renderMarkdown('Title\n=====')).toContain('<h1>Title</h1>')
    expect(renderMarkdown('> quoted')).toContain('<blockquote>')
    expect(renderMarkdown('---')).toContain('<hr>')
    expect(renderMarkdown('- one\n- two')).toMatch(/<ul>[\s\S]*<li>one<\/li>/)
    expect(renderMarkdown('1. a\n2. b')).toMatch(/<ol>[\s\S]*<li>a<\/li>/)
  })

  it('nests a list inside a list rather than flattening it', () => {
    const html = renderMarkdown('- one\n  - inner\n- two')
    expect(html.match(/<ul>/g)).toHaveLength(2)
    expect(html).toMatch(/<li>one[\s\S]*<ul>[\s\S]*inner/)
  })

  it('draws a task list as boxes, ticked and not', () => {
    const html = renderMarkdown('- [x] done\n- [ ] not done')
    expect(html).toContain('<span class="task-box done" aria-hidden="true"></span>')
    expect(html).toContain('<span class="task-box" aria-hidden="true"></span>')
    expect(html).toContain('<li class="task">')
    expect(text(html)).toContain('done')
    expect(html).not.toContain('[x]')
  })

  it('draws a table', () => {
    const html = renderMarkdown('| a | b |\n| - | - |\n| 1 | 2 |')
    expect(html).toContain('<table>')
    expect(html).toContain('<th>a</th>')
    expect(html).toContain('<td>1</td>')
  })

  it('draws an image, fetched late and without a referrer', () => {
    const html = renderMarkdown('![a shot](https://example.com/x.png)')
    expect(html).toContain('<img src="https://example.com/x.png"')
    expect(html).toContain('alt="a shot"')
    expect(html).toContain('loading="lazy"')
    expect(html).toContain('referrerpolicy="no-referrer"')
  })

  it('hides the html comments a request template is mostly made of', () => {
    const body = [
      '<!-- Say what this changes and why. -->',
      '## What',
      'A fix.',
      '<!--',
      'Several lines of instructions',
      'nobody was meant to read.',
      '-->',
      'Done.'
    ].join('\n')
    const html = renderMarkdown(body)
    expect(html).not.toContain('&lt;!--')
    expect(html).not.toContain('instructions')
    expect(text(html)).toContain('A fix.')
    expect(text(html)).toContain('Done.')
  })

  it('leaves a comment inside a fence alone, since it is being shown on purpose', () => {
    const html = renderMarkdown('```html\n<!-- kept -->\n```')
    expect(text(html)).toContain('<!-- kept -->')
  })

  it('reads through the tags a body is folded and broken with', () => {
    const html = renderMarkdown('<details><summary>The log</summary>\n\nline one\n</details>')
    expect(html).not.toContain('&lt;details&gt;')
    expect(text(html)).toContain('The log')
    expect(text(html)).toContain('line one')
    expect(renderMarkdown('one<br>two')).toContain('<br>')
  })

  it('says nothing about whitespace alone, or about a body of comments alone', () => {
    expect(renderMarkdown('')).toBe('')
    expect(renderMarkdown('  \n ')).toBe('')
    expect(renderMarkdown('<!-- nothing but this -->')).toBe('')
  })
})
