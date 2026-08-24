import hljs from 'highlight.js/lib/core'

import bash from 'highlight.js/lib/languages/bash'
import css from 'highlight.js/lib/languages/css'
import diff from 'highlight.js/lib/languages/diff'
import go from 'highlight.js/lib/languages/go'
import ini from 'highlight.js/lib/languages/ini'
import java from 'highlight.js/lib/languages/java'
import javascript from 'highlight.js/lib/languages/javascript'
import json from 'highlight.js/lib/languages/json'
import kotlin from 'highlight.js/lib/languages/kotlin'
import markdown from 'highlight.js/lib/languages/markdown'
import php from 'highlight.js/lib/languages/php'
import python from 'highlight.js/lib/languages/python'
import ruby from 'highlight.js/lib/languages/ruby'
import rust from 'highlight.js/lib/languages/rust'
import scss from 'highlight.js/lib/languages/scss'
import sql from 'highlight.js/lib/languages/sql'
import typescript from 'highlight.js/lib/languages/typescript'
import xml from 'highlight.js/lib/languages/xml'
import yaml from 'highlight.js/lib/languages/yaml'

// Registered explicitly rather than importing the full bundle: this keeps the
// app self-contained without shipping 190 grammars nobody opens.
const languages: Record<string, unknown> = {
  bash,
  css,
  diff,
  go,
  ini,
  java,
  javascript,
  json,
  kotlin,
  markdown,
  php,
  python,
  ruby,
  rust,
  scss,
  sql,
  typescript,
  xml,
  yaml
}

for (const [name, language] of Object.entries(languages)) {
  hljs.registerLanguage(name, language as never)
}

/** File extension to grammar. Vue, Svelte and HTML all read well as xml. */
const BY_EXTENSION: Record<string, string> = {
  bash: 'bash',
  c: 'java',
  cjs: 'javascript',
  conf: 'ini',
  cpp: 'java',
  cs: 'java',
  css: 'css',
  diff: 'diff',
  env: 'ini',
  go: 'go',
  h: 'java',
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
  md: 'markdown',
  mdx: 'markdown',
  mjs: 'javascript',
  patch: 'diff',
  php: 'php',
  phtml: 'php',
  py: 'python',
  rb: 'ruby',
  rs: 'rust',
  sass: 'scss',
  scss: 'scss',
  sh: 'bash',
  sql: 'sql',
  svelte: 'xml',
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
  dockerfile: 'bash',
  gemfile: 'ruby',
  makefile: 'bash',
  rakefile: 'ruby'
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
