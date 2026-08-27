import hljs from 'highlight.js/lib/core'

import bash from 'highlight.js/lib/languages/bash'
import c from 'highlight.js/lib/languages/c'
import clojure from 'highlight.js/lib/languages/clojure'
import cmake from 'highlight.js/lib/languages/cmake'
import cpp from 'highlight.js/lib/languages/cpp'
import csharp from 'highlight.js/lib/languages/csharp'
import css from 'highlight.js/lib/languages/css'
import dart from 'highlight.js/lib/languages/dart'
import diff from 'highlight.js/lib/languages/diff'
import dockerfile from 'highlight.js/lib/languages/dockerfile'
import elixir from 'highlight.js/lib/languages/elixir'
import erlang from 'highlight.js/lib/languages/erlang'
import go from 'highlight.js/lib/languages/go'
import gradle from 'highlight.js/lib/languages/gradle'
import graphql from 'highlight.js/lib/languages/graphql'
import groovy from 'highlight.js/lib/languages/groovy'
import haskell from 'highlight.js/lib/languages/haskell'
import ini from 'highlight.js/lib/languages/ini'
import java from 'highlight.js/lib/languages/java'
import javascript from 'highlight.js/lib/languages/javascript'
import json from 'highlight.js/lib/languages/json'
import julia from 'highlight.js/lib/languages/julia'
import kotlin from 'highlight.js/lib/languages/kotlin'
import latex from 'highlight.js/lib/languages/latex'
import lua from 'highlight.js/lib/languages/lua'
import makefile from 'highlight.js/lib/languages/makefile'
import markdown from 'highlight.js/lib/languages/markdown'
import nginx from 'highlight.js/lib/languages/nginx'
import nix from 'highlight.js/lib/languages/nix'
import objectivec from 'highlight.js/lib/languages/objectivec'
import ocaml from 'highlight.js/lib/languages/ocaml'
import perl from 'highlight.js/lib/languages/perl'
import plaintext from 'highlight.js/lib/languages/plaintext'
import php from 'highlight.js/lib/languages/php'
import powershell from 'highlight.js/lib/languages/powershell'
import properties from 'highlight.js/lib/languages/properties'
import protobuf from 'highlight.js/lib/languages/protobuf'
import python from 'highlight.js/lib/languages/python'
import r from 'highlight.js/lib/languages/r'
import ruby from 'highlight.js/lib/languages/ruby'
import rust from 'highlight.js/lib/languages/rust'
import scala from 'highlight.js/lib/languages/scala'
import scss from 'highlight.js/lib/languages/scss'
import shell from 'highlight.js/lib/languages/shell'
import sql from 'highlight.js/lib/languages/sql'
import swift from 'highlight.js/lib/languages/swift'
import typescript from 'highlight.js/lib/languages/typescript'
import vim from 'highlight.js/lib/languages/vim'
import xml from 'highlight.js/lib/languages/xml'
import yaml from 'highlight.js/lib/languages/yaml'

// Registered explicitly rather than importing the full bundle: this keeps the
// app self-contained without shipping 190 grammars nobody opens.
const languages: Record<string, unknown> = {
  bash,
  c,
  clojure,
  cmake,
  cpp,
  csharp,
  css,
  dart,
  diff,
  dockerfile,
  elixir,
  erlang,
  go,
  gradle,
  graphql,
  groovy,
  haskell,
  ini,
  java,
  javascript,
  json,
  julia,
  kotlin,
  latex,
  lua,
  makefile,
  markdown,
  nginx,
  nix,
  objectivec,
  ocaml,
  perl,
  php,
  plaintext,
  powershell,
  properties,
  protobuf,
  python,
  r,
  ruby,
  rust,
  scala,
  scss,
  shell,
  sql,
  swift,
  typescript,
  vim,
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
  as: 'javascript',
  asd: 'clojure',
  bash: 'bash',
  bat: 'powershell',
  c: 'c',
  cc: 'cpp',
  cfg: 'ini',
  cjs: 'javascript',
  clj: 'clojure',
  cljs: 'clojure',
  cmake: 'cmake',
  cmd: 'powershell',
  conf: 'ini',
  cpp: 'cpp',
  cs: 'csharp',
  csproj: 'xml',
  css: 'css',
  csv: 'plaintext',
  cxx: 'cpp',
  dart: 'dart',
  diff: 'diff',
  ejs: 'xml',
  env: 'ini',
  erb: 'ruby',
  erl: 'erlang',
  ex: 'elixir',
  exs: 'elixir',
  fish: 'shell',
  gawk: 'bash',
  go: 'go',
  gql: 'graphql',
  gradle: 'gradle',
  graphql: 'graphql',
  groovy: 'groovy',
  h: 'c',
  handlebars: 'xml',
  hbs: 'xml',
  hpp: 'cpp',
  hrl: 'erlang',
  hs: 'haskell',
  htm: 'xml',
  html: 'xml',
  ini: 'ini',
  java: 'java',
  jl: 'julia',
  js: 'javascript',
  json: 'json',
  json5: 'json',
  jsonc: 'json',
  jsonl: 'json',
  jsx: 'javascript',
  kt: 'kotlin',
  kts: 'kotlin',
  less: 'scss',
  lisp: 'clojure',
  lua: 'lua',
  m: 'objectivec',
  make: 'makefile',
  md: 'markdown',
  mdx: 'markdown',
  mjs: 'javascript',
  mk: 'makefile',
  ml: 'ocaml',
  mli: 'ocaml',
  mm: 'objectivec',
  ndjson: 'json',
  nix: 'nix',
  patch: 'diff',
  php: 'php',
  phtml: 'php',
  pl: 'perl',
  plist: 'xml',
  pm: 'perl',
  properties: 'properties',
  proto: 'protobuf',
  ps1: 'powershell',
  psm1: 'powershell',
  py: 'python',
  pyi: 'python',
  r: 'r',
  rb: 'ruby',
  resx: 'xml',
  rs: 'rust',
  sass: 'scss',
  sbt: 'scala',
  scala: 'scala',
  scss: 'scss',
  sh: 'bash',
  sql: 'sql',
  storyboard: 'xml',
  svelte: 'xml',
  svg: 'xml',
  swift: 'swift',
  tex: 'latex',
  tf: 'ini',
  tfvars: 'ini',
  toml: 'ini',
  ts: 'typescript',
  tsx: 'typescript',
  txt: 'plaintext',
  vim: 'vim',
  vue: 'xml',
  xaml: 'xml',
  xml: 'xml',
  xsd: 'xml',
  xsl: 'xml',
  yaml: 'yaml',
  yml: 'yaml',
  zsh: 'bash'
}

/**
 * Files that carry their language in their name rather than an extension.
 *
 * Dotfiles are here too, and matched whole: `.gitignore` split on its dot gives
 * "gitignore", which is not an extension anybody registered, so every dotfile
 * in a repository — and there are a dozen in most — came out with no colour at
 * all. Their leading dot is kept in the key, which is what tells `.env` the
 * dotfile apart from a `production.env`.
 */
const BY_NAME: Record<string, string> = {
  '.babelrc': 'json',
  '.bash_profile': 'bash',
  '.bashrc': 'bash',
  '.dockerignore': 'bash',
  '.editorconfig': 'ini',
  '.env': 'ini',
  '.eslintrc': 'json',
  '.gitattributes': 'bash',
  '.gitconfig': 'ini',
  '.gitignore': 'bash',
  '.gitmodules': 'ini',
  '.npmrc': 'ini',
  '.prettierrc': 'json',
  '.profile': 'bash',
  '.vimrc': 'vim',
  '.zshrc': 'bash',
  'cmakelists.txt': 'cmake',
  dockerfile: 'dockerfile',
  gemfile: 'ruby',
  justfile: 'makefile',
  // The makefile grammar, not bash: a rule's target and its recipe are
  // different things, and bash colours neither of them.
  makefile: 'makefile',
  procfile: 'yaml',
  rakefile: 'ruby',
  vagrantfile: 'ruby'
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

/**
 * The extension, as the two maps key them: the part after the last dot, and
 * nothing at all for a name that is only a dot and a word.
 *
 * `.gitignore` has no extension — the dot makes it hidden, it does not separate
 * a name from a type — but `String.split` cannot tell the difference and handed
 * back "gitignore" as though it were one. Every dotfile in a repository went
 * through the extension map, missed, and came out uncoloured.
 */
function extensionOf(file: string): string {
  const name = file.startsWith('.') ? file.slice(1) : file
  const at = name.lastIndexOf('.')
  return at === -1 ? '' : name.slice(at + 1).toLowerCase()
}

/**
 * A dotfile's name without whatever was appended to it: `.env.local` is a
 * `.env`, and `.eslintrc.local` an `.eslintrc`. Only consulted once the
 * extension itself has come up empty, so `.eslintrc.json` is still json.
 */
function dotfileBase(file: string): string {
  if (!file.startsWith('.')) return ''
  const rest = file.slice(1)
  const at = rest.indexOf('.')
  return at === -1 ? file : `.${rest.slice(0, at)}`
}

export function labelFor(path: string) {
  const file = path.split('/').pop() ?? path
  const name = file.toLowerCase()
  if (BY_NAME[name]) return name
  const extension = extensionOf(name)
  const base = dotfileBase(name)
  // What follows a dotfile's own name is a label only where it names a type we
  // know: `.eslintrc.json` is json, while `.env.local` is still a `.env`.
  if (base && !BY_EXTENSION[extension]) return BY_NAME[base] ? base : null
  return extension ? (NAMED[extension] ?? extension) : null
}

export function languageFor(path: string) {
  const file = (path.split('/').pop() ?? path).toLowerCase()
  const byName = BY_NAME[file]
  if (byName) return byName
  const byExtension = BY_EXTENSION[extensionOf(file)]
  if (byExtension) return byExtension
  const base = dotfileBase(file)
  return (base ? BY_NAME[base] : null) ?? null
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

/**
 * Colours a fenced code block out of a comment body.
 *
 * The fence names its own language rather than a path doing it, so the name is
 * asked of highlight.js directly — that way its aliases (`ts`, `sh`, `yml`)
 * work, and a fence that names nothing we have is escaped rather than guessed
 * at.
 */
export function highlightBlock(code: string, info: string): string {
  const language = info && hljs.getLanguage(info) ? info : null
  return highlightWhole(code, language).join('\n')
}
