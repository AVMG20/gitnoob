import { ref } from 'vue'
import { highlightTree, tagHighlighter, type Highlighter } from '@lezer/highlight'
import type { Language } from '@codemirror/language'
import { snippetLanguage, syntaxSpecs } from './useCodeMirror'
import { version } from './useHighlight'

/**
 * Syntax colours for lines cut out of a file: the search window's results.
 *
 * Each file's hit lines are parsed together, once, with the same Lezer
 * grammar the editor uses, and coloured with the same theme specs, as inline
 * styles so nothing has to be mounted. Answers are cached by theme, file and
 * the lines themselves. A language not loaded yet is loaded in the
 * background and the lines come back plain until it lands; `snippetTick`
 * goes up then, so whoever drew them can draw them again.
 */

/** [from, to, css] in one line, in UTF-16 units. */
export type Token = [number, number, string]

export const snippetTick = ref(0)

const languages = new Map<string, Language | null>()
const loading = new Set<string>()
const cache = new Map<string, Token[][]>()

function extensionOf(path: string) {
  const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase()
  return name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : ''
}

/** The loaded language for a path, or undefined while it loads. */
function languageOf(path: string): Language | null | undefined {
  const key = extensionOf(path)
  if (languages.has(key)) return languages.get(key)!
  if (!loading.has(key)) {
    loading.add(key)
    void snippetLanguage(path).then((language) => {
      languages.set(key, language)
      loading.delete(key)
      if (language) snippetTick.value++
    })
  }
  return undefined
}

let painter: { version: number; highlighter: Highlighter; css: string[] } | null = null

/** The theme's specs as a highlighter whose classes are indices into `css`. */
function themePainter() {
  if (painter && painter.version === version.value) return painter
  const specs = syntaxSpecs()
  const css = specs.map((spec) =>
    [
      spec.color ? `color:${spec.color}` : '',
      spec.fontStyle ? `font-style:${spec.fontStyle}` : '',
      spec.fontWeight ? `font-weight:${spec.fontWeight}` : ''
    ]
      .filter(Boolean)
      .join(';')
  )
  const highlighter = tagHighlighter(specs.map((spec, at) => ({ tag: spec.tag, class: `s${at}` })))
  painter = { version: version.value, highlighter, css }
  cache.clear()
  return painter
}

/**
 * Whether a line cut out of a `.phtml` template is PHP on its own (a line
 * inside a `<?php` block, such as `$items = $block->getItems();`) rather than
 * HTML with PHP tags in it. Read alone, in the template's own mixed mode, such
 * a line would be taken for HTML text and come out plain.
 */
export function phpLine(line: string): boolean {
  const text = line.trim()
  if (!text || text.startsWith('<') || /<\?|\?>/.test(text)) return false
  return /^(\$|\/\/|\/\*|\*|#|use\b|if\b|else|endif|foreach\b|endforeach|for\b|endfor|while\b|endwhile|switch\b|case\b|return\b|echo\b)|;$/.test(text)
}

/**
 * The tokens of each of `lines`, from `path`'s grammar; null while its
 * language is still on its way (or there is none, or no theme colours).
 * A `.phtml` template's lines are read one by one, each as plain PHP or as
 * the template's HTML with PHP in it, whichever it is.
 */
export function colourLines(path: string, lines: string[]): Token[][] | null {
  const language = languageOf(path)
  if (!language) return null
  const template = extensionOf(path) === 'phtml'
  const plain = template ? languageOf('plain.php') : null
  if (template && !plain) return null
  const { highlighter, css } = themePainter()
  if (!css.length) return null
  const text = lines.join('\n')
  const key = `${path}\u0000${text}`
  const known = cache.get(key)
  if (known) return known
  const out: Token[][] = lines.map(() => [])
  if (template) {
    lines.forEach((line, row) => {
      const tree = (phpLine(line) ? plain! : language).parser.parse(line)
      paint(tree, highlighter, css, [line], out.slice(row, row + 1))
    })
  } else {
    paint(language.parser.parse(text), highlighter, css, lines, out)
  }
  if (cache.size > 800) cache.clear()
  cache.set(key, out)
  return out
}

/** Puts the tokens of `tree`, a parse of `lines` joined, into `out`, line by line. */
function paint(tree: ReturnType<Language['parser']['parse']>, highlighter: Highlighter, css: string[], lines: string[], out: Token[][]) {
  const starts: number[] = []
  let at = 0
  for (const line of lines) {
    starts.push(at)
    at += line.length + 1
  }
  let row = 0
  highlightTree(tree, highlighter, (from, to, classes) => {
    const style = classes
      .split(' ')
      .map((name) => css[Number(name.slice(1))])
      .filter(Boolean)
      .join(';')
    if (!style) return
    // A token can cross a line break (a block comment); it is cut at each one.
    while (row < starts.length - 1 && starts[row + 1]! <= from) row++
    let line = row
    let start = from
    while (start < to && line < lines.length) {
      const end = Math.min(to, starts[line]! + lines[line]!.length)
      if (end > start) out[line]!.push([start - starts[line]!, end - starts[line]!, style])
      line++
      start = starts[line] ?? to
    }
  })
}

export interface Part {
  text: string
  style: string | null
}

/** A stretch of a line inside or outside a match, in runs of one colour. */
export interface Run {
  hit: boolean
  parts: Part[]
}

/** A line cut at the search's matches, each piece coloured by its tokens. */
export function runsOf(text: string, tokens: Token[] | null, ranges: [number, number][] = []): Run[] {
  const out: Run[] = []
  let at = 0
  const stretch = (from: number, to: number, hit: boolean) => {
    if (to > from) out.push({ hit, parts: colour(text, tokens ?? [], from, to) })
  }
  for (const [from, to] of ranges) {
    stretch(at, Math.max(at, from), false)
    stretch(Math.max(at, from), to, true)
    at = Math.max(at, to)
  }
  stretch(at, text.length, false)
  return out
}

function colour(text: string, tokens: Token[], from: number, to: number): Part[] {
  const parts: Part[] = []
  let at = from
  for (const [start, end, style] of tokens) {
    if (end <= at || start >= to) continue
    if (start > at) parts.push({ text: text.slice(at, start), style: null })
    const stop = Math.min(end, to)
    parts.push({ text: text.slice(Math.max(start, at), stop), style })
    at = stop
  }
  if (at < to) parts.push({ text: text.slice(at, to), style: null })
  return parts
}

/** For tests. */
export function resetSnippetColours() {
  languages.clear()
  loading.clear()
  cache.clear()
  painter = null
}
