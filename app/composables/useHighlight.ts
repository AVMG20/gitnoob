import hljs from 'highlight.js/lib/core'

import bash from 'highlight.js/lib/languages/bash'
import c from 'highlight.js/lib/languages/c'
import cpp from 'highlight.js/lib/languages/cpp'
import csharp from 'highlight.js/lib/languages/csharp'
import css from 'highlight.js/lib/languages/css'
import dart from 'highlight.js/lib/languages/dart'
import diff from 'highlight.js/lib/languages/diff'
import dockerfile from 'highlight.js/lib/languages/dockerfile'
import go from 'highlight.js/lib/languages/go'
import graphql from 'highlight.js/lib/languages/graphql'
import ini from 'highlight.js/lib/languages/ini'
import java from 'highlight.js/lib/languages/java'
import javascript from 'highlight.js/lib/languages/javascript'
import json from 'highlight.js/lib/languages/json'
import kotlin from 'highlight.js/lib/languages/kotlin'
import lua from 'highlight.js/lib/languages/lua'
import markdown from 'highlight.js/lib/languages/markdown'
import php from 'highlight.js/lib/languages/php'
import powershell from 'highlight.js/lib/languages/powershell'
import python from 'highlight.js/lib/languages/python'
import ruby from 'highlight.js/lib/languages/ruby'
import rust from 'highlight.js/lib/languages/rust'
import scss from 'highlight.js/lib/languages/scss'
import sql from 'highlight.js/lib/languages/sql'
import swift from 'highlight.js/lib/languages/swift'
import typescript from 'highlight.js/lib/languages/typescript'
import xml from 'highlight.js/lib/languages/xml'
import yaml from 'highlight.js/lib/languages/yaml'

// Registered explicitly rather than importing the full bundle: this keeps the
// app self-contained without shipping 190 grammars nobody opens.
const languages: Record<string, unknown> = {
  bash,
  c,
  cpp,
  csharp,
  css,
  dart,
  diff,
  dockerfile,
  go,
  graphql,
  ini,
  java,
  javascript,
  json,
  kotlin,
  lua,
  markdown,
  php,
  powershell,
  python,
  ruby,
  rust,
  scss,
  sql,
  swift,
  typescript,
  xml,
  yaml
}

for (const [name, language] of Object.entries(languages)) {
  hljs.registerLanguage(name, language as never)
}

/**
 * File extension to grammar.
 *
 * Vue, Svelte and HTML all read well as xml, whose grammar hands the inside of
 * a `<script>` block to javascript and a `<style>` block to css — which is what
 * makes a single-file component come out right, so long as it is coloured whole
 * rather than a line at a time.
 */
const BY_EXTENSION: Record<string, string> = {
  bash: 'bash',
  c: 'c',
  cc: 'cpp',
  cjs: 'javascript',
  conf: 'ini',
  cpp: 'cpp',
  cs: 'csharp',
  css: 'css',
  cxx: 'cpp',
  dart: 'dart',
  diff: 'diff',
  env: 'ini',
  go: 'go',
  gql: 'graphql',
  graphql: 'graphql',
  h: 'c',
  hpp: 'cpp',
  htm: 'xml',
  html: 'xml',
  ini: 'ini',
  java: 'java',
  js: 'javascript',
  json: 'json',
  jsonc: 'json',
  jsx: 'javascript',
  kt: 'kotlin',
  less: 'scss',
  lua: 'lua',
  md: 'markdown',
  mdx: 'markdown',
  mjs: 'javascript',
  patch: 'diff',
  php: 'php',
  phtml: 'php',
  ps1: 'powershell',
  py: 'python',
  rb: 'ruby',
  rs: 'rust',
  sass: 'scss',
  scss: 'scss',
  sh: 'bash',
  sql: 'sql',
  svelte: 'xml',
  swift: 'swift',
  toml: 'ini',
  ts: 'typescript',
  tsx: 'typescript',
  vue: 'xml',
  xml: 'xml',
  yaml: 'yaml',
  yml: 'yaml',
  zsh: 'bash'
}

/** Files with no extension that are still worth colouring. */
const BY_NAME: Record<string, string> = {
  dockerfile: 'dockerfile',
  gemfile: 'ruby',
  makefile: 'bash',
  rakefile: 'ruby'
}

/**
 * What to call the file, for the chip above a diff.
 *
 * Not the grammar it is coloured with: a `.vue` file is painted with the xml
 * grammar because that reads best, and labelling it "xml" told the reader
 * something about our highlighter rather than about their file.
 */
const NAMED: Record<string, string> = {
  cc: 'c++',
  cjs: 'javascript',
  cpp: 'c++',
  cs: 'c#',
  cxx: 'c++',
  h: 'c header',
  hpp: 'c++ header',
  htm: 'html',
  jsx: 'javascript',
  kt: 'kotlin',
  md: 'markdown',
  mjs: 'javascript',
  ps1: 'powershell',
  py: 'python',
  rb: 'ruby',
  rs: 'rust',
  ts: 'typescript',
  tsx: 'typescript',
  yml: 'yaml'
}

export function labelFor(path: string) {
  const file = path.split('/').pop() ?? path
  const name = file.toLowerCase()
  if (BY_NAME[name]) return name
  if (!file.includes('.')) return null
  const extension = name.split('.').pop()!
  return NAMED[extension] ?? extension
}

export function languageFor(path: string) {
  const file = path.split('/').pop() ?? path
  const byName = BY_NAME[file.toLowerCase()]
  if (byName) return byName
  const extension = file.includes('.') ? file.split('.').pop()!.toLowerCase() : ''
  return BY_EXTENSION[extension] ?? null
}

const escapeHtml = (text: string) =>
  text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')

/**
 * Colours one line of code.
 *
 * Highlighting line by line loses context that spans lines — the body of a long
 * block comment is not recognised as one — but a diff only ever has fragments
 * to show, and this keeps a large file instant to render.
 */
export function highlightLine(code: string, language: string | null) {
  if (!code) return ''
  if (!language) return escapeHtml(code)
  try {
    return hljs.highlight(code, { language, ignoreIllegals: true }).value
  } catch {
    return escapeHtml(code)
  }
}

/**
 * Colours a whole file, and hands back one line of HTML per line of source.
 *
 * Worth the extra work over [`highlightLine`] wherever the whole text is at
 * hand, because the things a line cannot know about itself are exactly the ones
 * that matter: the body of a block comment, a string that runs on, and — the
 * reason this exists — the script inside a `.vue` file, which is painted with
 * the xml grammar and only becomes JavaScript once the `<script>` tag above it
 * has been read.
 *
 * A highlight can leave spans open across a newline, which would then wrap the
 * rest of the file when the lines are laid out as separate rows. So each line
 * closes what is still open and the next one opens it again.
 */
export function highlightWhole(text: string, language: string | null): string[] {
  const source = text.split('\n')
  if (!language) return source.map(escapeHtml)

  let html: string
  try {
    html = hljs.highlight(text, { language, ignoreIllegals: true }).value
  } catch {
    return source.map(escapeHtml)
  }

  const lines: string[] = []
  const open: string[] = []
  let line = ''
  // Everything hljs emits is either a span tag, a newline, or escaped text —
  // any `<` in the text itself has already become `&lt;`.
  for (const token of html.match(/<span[^>]*>|<\/span>|\n|[^<\n]+/g) ?? []) {
    if (token === '\n') {
      lines.push(line + '</span>'.repeat(open.length))
      line = open.join('')
    } else if (token === '</span>') {
      open.pop()
      line += token
    } else if (token.startsWith('<span')) {
      open.push(token)
      line += token
    } else {
      line += token
    }
  }
  lines.push(line + '</span>'.repeat(open.length))

  // A highlighter that lost a line somewhere is not worth trusting over the
  // plain text: the marks in the gutter are keyed by line number.
  return lines.length === source.length ? lines : source.map(escapeHtml)
}
