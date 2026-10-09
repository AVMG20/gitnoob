import { describe, expect, it } from 'vitest'
import { findIn, markHtml, occurrences } from '~/composables/useFind'

describe('finding text in a file', () => {
  it('ignores case unless asked not to', () => {
    expect(occurrences('Store storeAgain STORE', 'store')).toEqual([
      [0, 5],
      [6, 11],
      [17, 22]
    ])
    expect(occurrences('Store storeAgain STORE', 'store', true)).toEqual([[6, 11]])
  })

  it('never counts the same characters twice', () => {
    expect(occurrences('aaaa', 'aa')).toEqual([
      [0, 2],
      [2, 4]
    ])
  })

  it('finds nothing for nothing', () => {
    expect(occurrences('anything', '')).toEqual([])
    expect(findIn(['a', 'b'], '')).toEqual([])
  })

  it('lists every match in reading order, by row and by place in the row', () => {
    expect(findIn(['one row', 'none', 'row row'], 'row')).toEqual([
      { row: 0, nth: 0 },
      { row: 2, nth: 0 },
      { row: 2, nth: 1 }
    ])
  })
})

describe('marking matches in coloured HTML', () => {
  it('leaves a line without a match exactly as it was', () => {
    const html = '<span style="color:red">return</span> x;'
    expect(markHtml(html, 'zzz')).toBe(html)
    expect(markHtml(html, '')).toBe(html)
  })

  it('marks plain text', () => {
    expect(markHtml('a store b', 'store')).toBe('a <mark class="find-hit">store</mark> b')
  })

  it('marks inside a coloured token without touching its tag', () => {
    expect(markHtml('<span style="color:red">storeAgain</span>', 'again')).toBe(
      '<span style="color:red">store<mark class="find-hit">Again</mark></span>'
    )
  })

  it('splits a match that runs across two tokens, so the tags stay nested', () => {
    expect(markHtml('<span>$this</span><span>-&gt;store</span>', 'this->st')).toBe(
      '<span>$<mark class="find-hit">this</mark></span>' +
        '<span><mark class="find-hit">-&gt;st</mark>ore</span>'
    )
  })

  it('counts an entity as the one character it stands for', () => {
    expect(markHtml('a &lt; b &amp;&amp; c', '&&')).toBe(
      'a &lt; b <mark class="find-hit">&amp;&amp;</mark> c'
    )
    expect(markHtml('&lt;div&gt;', 'div')).toBe('&lt;<mark class="find-hit">div</mark>&gt;')
  })

  it('picks out the current match from the others on the same line', () => {
    expect(markHtml('ab ab ab', 'ab', false, 1)).toBe(
      '<mark class="find-hit">ab</mark> <mark class="find-hit now">ab</mark> ' +
        '<mark class="find-hit">ab</mark>'
    )
  })

  it('gives matches that touch a mark each', () => {
    expect(markHtml('aaaa', 'aa', false, 1)).toBe(
      '<mark class="find-hit">aa</mark><mark class="find-hit now">aa</mark>'
    )
  })

  it('keeps characters outside the basic plane whole', () => {
    expect(markHtml('🙂 hi 🙂', 'hi')).toBe('🙂 <mark class="find-hit">hi</mark> 🙂')
  })
})
