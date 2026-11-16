import { beforeAll, describe, expect, it, vi } from 'vitest'
import { EditorState } from '@codemirror/state'
import { ensureSyntaxTree, language } from '@codemirror/language'
import { classHighlighter, highlightTree, tags } from '@lezer/highlight'

vi.mock('~/composables/useCodeMirror', async (original) => ({
  ...(await original<typeof import('~/composables/useCodeMirror')>()),
  syntaxSpecs: () => [
    { tag: [tags.variableName, tags.special(tags.variableName)], color: '#a00' },
    { tag: [tags.keyword, tags.controlKeyword], color: '#00b' },
    { tag: tags.tagName, color: '#0c0' },
    { tag: tags.propertyName, color: '#c0c' }
  ]
}))

const { editorLanguage, snippetLanguage } = await import('~/composables/useCodeMirror')
const { colourLines, phpLine, resetSnippetColours, snippetTick } = await import('~/composables/snippetColours')
const { highlightWhole, labelFor, languageFor, ready, version } = await import('~/composables/useHighlight')
const { fileIcon } = await import('~/composables/fileIcons')
const { languageIndent } = await import('~/composables/indentation')
const { memberAccess } = await import('~/composables/useAutocomplete')

/**
 * A Magento `.phtml` template is HTML with PHP in it: every place that picks
 * a language for a file reads it as PHP in lang-php's mixed mode, and says
 * so in the header.
 */
const TEMPLATE = 'app/code/Acme/Shop/view/frontend/templates/list.phtml'
const SOURCE = [
  '<?php',
  '/** @var \\Acme\\Shop\\Block\\Items $block */',
  '$items = $block->getItems();',
  '?>',
  '<ul class="items">',
  '<?php foreach ($items as $item): ?>',
  '    <li><?= $block->escapeHtml($item->getName()) ?></li>',
  '<?php endforeach ?>',
  '</ul>',
  ''
].join('\n')

beforeAll(() => ready())

describe('a .phtml template', () => {
  it('opens in the editor as PHP inside HTML, with tokens for both', async () => {
    const support = await editorLanguage(TEMPLATE)
    expect(support).not.toBeNull()
    const state = EditorState.create({ doc: SOURCE, extensions: [support!] })
    expect(state.facet(language)?.name).toBe('php')
    const tree = ensureSyntaxTree(state, SOURCE.length, 5000)!
    const tokens: [string, string][] = []
    highlightTree(tree, classHighlighter, (from, to, classes) => tokens.push([SOURCE.slice(from, to), classes]))
    const classOf = (text: string) => tokens.find(([token]) => token === text)?.[1]
    // PHP: the tag, a keyword and a variable.
    expect(classOf('<?php')).toContain('tok-meta')
    expect(classOf('foreach')).toContain('tok-keyword')
    expect(classOf('$items')).toContain('tok-variableName')
    // HTML around it: the tag names.
    expect(classOf('ul')).toContain('tok-typeName')
    expect(classOf('li')).toContain('tok-typeName')
  })

  it('is PHP for search results, with a line inside a PHP block read as PHP', async () => {
    expect((await snippetLanguage(TEMPLATE))?.name).toBe('php')
    resetSnippetColours()
    const lines = ['$items = $block->getItems();', '    <li><?= $block->escapeHtml($item->getName()) ?></li>']
    for (let i = 0; i < 100 && !colourLines(TEMPLATE, lines); i++) {
      const before = snippetTick.value
      while (snippetTick.value === before) await new Promise((done) => setTimeout(done, 5))
    }
    const coloured = colourLines(TEMPLATE, lines)!
    const styleOf = (row: number, text: string) => {
      const at = lines[row]!.indexOf(text)
      return coloured[row]!.find(([from, to]) => from <= at && to >= at + text.length)?.[2]
    }
    expect(styleOf(0, '$items')).toBe('color:#a00')
    expect(styleOf(1, 'li')).toBe('color:#0c0')
    expect(styleOf(1, 'escapeHtml')).toBe('color:#c0c')
  })

  it('tells a PHP line from an HTML one', () => {
    expect(phpLine('$items = $block->getItems();')).toBe(true)
    expect(phpLine('    // a comment')).toBe(true)
    expect(phpLine('foreach ($items as $item):')).toBe(true)
    expect(phpLine('<li><?= $x ?></li>')).toBe(false)
    expect(phpLine('Some text in a paragraph')).toBe(false)
    expect(phpLine('')).toBe(false)
  })

  it('is coloured as PHP in Diff mode, and labelled a PHP template', async () => {
    expect(languageFor(TEMPLATE)).toBe('php')
    // The grammar loads on first use, and the version goes up when it has.
    let html = highlightWhole(SOURCE, languageFor(TEMPLATE))
    for (let i = 0; i < 200 && !html[5]!.includes('<span'); i++) {
      const before = version.value
      while (version.value === before && i < 200) {
        await new Promise((done) => setTimeout(done, 10))
        i++
      }
      html = highlightWhole(SOURCE, languageFor(TEMPLATE))
    }
    expect(html[5]).toContain('<span style="color:')
    expect(html[5]).toContain('foreach')
    expect(labelFor(TEMPLATE)).toBe('php template')
    expect(labelFor('app/Tax.php')).toBe('php')
    // A Blade view is a template too, coloured as PHP.
    expect(labelFor('resources/views/home.blade.php')).toBe('blade template')
    expect(languageFor('resources/views/home.blade.php')).toBe('php')
  })

  it('gets the PHP icon, four-space indentation and PHP member completion', () => {
    expect(fileIcon(TEMPLATE)).toMatchObject({ text: 'P', tone: 'php', label: 'PHP template' })
    expect(languageIndent(TEMPLATE)).toEqual({ tabs: false, size: 4 })
    expect(memberAccess(TEMPLATE, '$block->get')).toBe('->')
  })
})
