import { describe, expect, it } from 'vitest'
import { renderMarkdown } from '../app/composables/useMd'

describe('renderMarkdown', () => {
  it('carries text over as paragraphs, newlines kept', () => {
    const html = renderMarkdown('one\ntwo\n\nthree')
    expect(html).toBe('<p>one\ntwo</p><p>three</p>')
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
    const html = renderMarkdown('```ts\nconst a = 1 < 2\n```')
    expect(html).toContain('<pre><code class="language-ts">const a = 1 &lt; 2</code></pre>')
  })

  it('links named and bare urls, http and https only', () => {
    expect(renderMarkdown('[site](https://example.com)')).toContain(
      '<a href="https://example.com" target="_blank" rel="noopener noreferrer">site</a>'
    )
    expect(renderMarkdown('see https://example.com/x')).toContain('href="https://example.com/x"')
    // A javascript: url is text, not a link.
    const bad = renderMarkdown('[x](javascript:alert(1))')
    expect(bad).not.toContain('href="javascript')
  })

  it('marks emphasis, deletions, mentions and references', () => {
    const html = renderMarkdown('**bold** and *em* and ~~gone~~\n@arno see #36')
    expect(html).toContain('<strong>bold</strong>')
    expect(html).toContain('<em>em</em>')
    expect(html).toContain('<del>gone</del>')
    expect(html).toContain('<span class="mention">@arno</span>')
    expect(html).toContain('<span class="ref">#36</span>')
  })

  it('steps headings down in size and draws lists, quotes and rules', () => {
    expect(renderMarkdown('## Title')).toContain('font-size:11px')
    expect(renderMarkdown('- one\n- two')).toContain('<ul><li>one</li><li>two</li></ul>')
    expect(renderMarkdown('1. a\n2. b')).toContain('<ol><li>a</li><li>b</li></ol>')
    expect(renderMarkdown('> quoted')).toContain('<blockquote>quoted</blockquote>')
    expect(renderMarkdown('---')).toContain('<hr/>')
  })

  it('closes a fence left open at the end of the text', () => {
    expect(renderMarkdown('```\nstill code')).toContain('<pre><code>still code</code></pre>')
  })

  it('says nothing about whitespace alone', () => {
    expect(renderMarkdown('')).toBe('')
    expect(renderMarkdown('  \n ')).toBe('')
  })

  it('keeps a plain fence language as a class and drops anything else', () => {
    expect(renderMarkdown('```ts\nx\n```')).toContain('<pre><code class="language-ts">')
    expect(renderMarkdown('```c#\nx\n```')).toContain('<pre><code class="language-c#">')
    expect(renderMarkdown('```objective-c++\nx\n```')).toContain(
      '<pre><code class="language-objective-c++">'
    )
    expect(renderMarkdown('```\nx\n```')).toContain('<pre><code>')
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
    expect(packed).toContain('<pre><code>x</code></pre>')
    expect(spaced).toContain('&lt;img')
  })

  it('does not let list markers or emphasis inside code be transformed', () => {
    const html = renderMarkdown('`*not em*`')
    expect(html).not.toContain('<em>')
  })
})
