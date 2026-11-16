import { it } from 'vitest'
import { EditorState } from '@codemirror/state'
import { ensureSyntaxTree } from '@codemirror/language'
import { highlightTree, classHighlighter } from '@lezer/highlight'
import { editorLanguage } from '~/composables/useCodeMirror'
import { highlightWhole, ready, languageFor } from '~/composables/useHighlight'
import { colourLines } from '~/composables/snippetColours'
it('probe', async () => {
  const doc = '<div class="a">\n<?php foreach ($block->getItems() as $item): ?>\n<?= $block->escapeHtml(__("Hi")) ?>\n<?php endforeach ?></div>\n'
  for (const p of ['x.phtml', 'x.php']) {
    const ext = await editorLanguage(p)
    const state = EditorState.create({ doc, extensions: ext ? [ext] : [] })
    const tree = ensureSyntaxTree(state, doc.length, 5000)!
    const out: string[] = []
    highlightTree(tree, classHighlighter, (f, t, c) => out.push(doc.slice(f, t) + '=' + c))
    console.log(p, out.join(' | '))
  }
  await ready()
  console.log(languageFor('a.phtml'), highlightWhole(doc, languageFor('a.phtml')).join('\n'))
  await new Promise(r => setTimeout(r, 2000))
  console.log(highlightWhole(doc, languageFor('a.phtml')).join('\n'))
  console.log(JSON.stringify(colourLines('x.phtml', doc.split('\n'))))
  await new Promise(r => setTimeout(r, 500))
  console.log(JSON.stringify(colourLines('x.phtml', doc.split('\n'))))
})
