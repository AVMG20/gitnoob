import { ref } from 'vue'
import { bundledLanguages, bundledLanguagesInfo, createHighlighter, type Highlighter } from 'shiki'

/**
 * Colour for code, from Shiki.
 *
 * Shiki runs the same TextMate grammars VS Code does, which is what makes a
 * `.vue` file come out right: its grammar knows a `<script>` block holds
 * TypeScript and a `<style>` block holds CSS, rather than the whole file being
 * approximated as XML the way the old highlighter had to.
 *
 * The cost is that a grammar is loaded rather than compiled in, so the first
 * paint of a language we have not seen yet has no colour. `version` below is
 * how a component finds out that colour has arrived.
 */

/**
 * Grammars compiled in from the start.
 *
 * A repository is nearly always one of these, and a grammar that is already
 * loaded paints on the first frame rather than the second. The rest are fetched
 * when a file that needs them is opened.
 */
const PRELOAD = [
  'bash',
  'css',
  'html',
  'javascript',
  'json',
  'markdown',
  'python',
  'rust',
  'typescript',
  'vue',
  'yaml'
]

/** Shiki paints these without a grammar, and never fails on them. */
const PLAIN = 'text'

/**
 * Bumped whenever a grammar or a theme finishes loading.
 *
 * Every function below reads it before it paints, so a component that colours
 * code inside a `computed` re-runs on its own once the grammar it wanted is
 * there. Without it the first file opened in a language stays grey until
 * something unrelated happens to redraw it.
 */
export const version = ref(0)

/** The Shiki theme in use, or `plain` for no colour at all. */
export const theme = ref<string>('github-dark')

let highlighter: Highlighter | null = null
let booting: Promise<void> | null = null
const loading = new Set<string>()

/** Every id and alias Shiki ships, so a fence naming `ts` or `yml` resolves. */
const ALIASES = new Map<string, string>()
for (const language of bundledLanguagesInfo) {
  ALIASES.set(language.id, language.id)
  for (const alias of language.aliases ?? []) ALIASES.set(alias, language.id)
}

/**
 * Starts the highlighter, once.
 *
 * Nothing waits on this. Callers paint plain text until it resolves and the
 * version bump brings them back.
 */
function boot(): Promise<void> {
  if (booting) return booting
  booting = createHighlighter({
    themes: [theme.value === 'plain' ? 'github-dark' : theme.value],
    langs: PRELOAD
  })
    .then((made) => {
      highlighter = made
      version.value++
    })
    .catch(() => {
      // A highlighter that would not start is not worth retrying on every
      // keystroke; code stays plain, which is legible.
    })
  return booting
}

if (typeof window !== 'undefined') void boot()

/**
 * Resolves once the highlighter is up and its first theme is loaded.
 *
 * Nothing in the app waits on this — a component paints plain text and is
 * brought back by the version bump. It is here for a test, which has no frames
 * to be brought back on and would otherwise be asserting against the fallback.
 */
export function ready(): Promise<void> {
  return boot()
}

/** Loads a grammar in the background, then asks whoever wanted it to repaint. */
function ensure(language: string) {
  if (!highlighter || loading.has(language)) return
  if (highlighter.getLoadedLanguages().includes(language)) return
  if (!(language in bundledLanguages)) return
  loading.add(language)
  void highlighter
    .loadLanguage(language as keyof typeof bundledLanguages)
    .then(() => {
      version.value++
    })
    .catch(() => {
      // A grammar that will not load leaves the file plain rather than empty.
    })
    .finally(() => loading.delete(language))
}

/** Loads a theme in the background and repaints once it is there. */
export function setHighlightTheme(id: string): void {
  theme.value = id
  if (id === 'plain') {
    version.value++
    return
  }
  void boot().then(() => {
    if (!highlighter) return
    if (highlighter.getLoadedThemes().includes(id)) {
      version.value++
      return
    }
    highlighter
      .loadTheme(id as never)
      .then(() => {
        version.value++
      })
      .catch(() => {
        // An unknown theme leaves the last one on rather than clearing colour.
      })
  })
}

const escapeHtml = (text: string) =>
  text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

/** Shiki's font-style bits, which it does not export as a value. */
const ITALIC = 1
const BOLD = 2
const UNDERLINE = 4

/** One line of tokens as the HTML a row is filled with. */
function paint(tokens: { content: string; color?: string; fontStyle?: number }[]): string {
  let html = ''
  for (const token of tokens) {
    const text = escapeHtml(token.content)
    const style: string[] = []
    if (token.color) style.push(`color:${token.color}`)
    const font = token.fontStyle ?? 0
    if (font > 0) {
      if (font & ITALIC) style.push('font-style:italic')
      if (font & BOLD) style.push('font-weight:bold')
      if (font & UNDERLINE) style.push('text-decoration:underline')
    }
    html += style.length ? `<span style="${style.join(';')}">${text}</span>` : text
  }
  return html
}

/** Files that are several other languages inside a wrapper. */
const SFC = new Set(['vue', 'svelte'])

/** The tags that wrapper is made of, and which say what a line is. */
const SFC_TAG = /<\/?(?:template|script|style)[\s>]/

/**
 * Which grammar a piece of a single-file component should be read with.
 *
 * The `vue` grammar decides what a line is from the block tag above it, so a
 * fragment that does not carry one is template text as far as it is concerned —
 * and a review shows exactly that. A pull request touching the script body of a
 * component has no `<script>` tag anywhere in its hunks, and the whole file came
 * out flat.
 *
 * Nothing in a review says which block a hunk came from; the forge sends the
 * patch and not the file. So the fragment is read for what it looks like, and
 * only where it looks like one thing clearly. A tie, or nothing recognised at
 * all, leaves the wrapper's own grammar in place — which is what this did
 * before, and is never worse than it.
 */
function blockOf(text: string, language: string): string {
  if (!SFC.has(language) || SFC_TAG.test(text)) return language
  let markup = 0
  let script = 0
  let style = 0
  for (const line of text.split('\n')) {
    const trimmed = line.trim()
    if (!trimmed) continue
    // Order matters: `<div :class="x">` is markup, whatever else is in it.
    if (/^<\/?[a-zA-Z]/.test(trimmed) || trimmed.includes('{{')) markup++
    else if (/\b(?:const|let|var|function|import|export|return|await|async|new|typeof)\b|=>/.test(trimmed)) script++
    else if (/^[-\w]+\s*:\s*[^;]+;$/.test(trimmed) || /^[.#&][\w-]/.test(trimmed)) style++
  }
  if (script > markup && script >= style) return 'typescript'
  if (style > markup && style > script) return 'css'
  return language
}

/**
 * Tokens for some text, or null when they cannot be had yet.
 *
 * Reads `version` first so that a caller inside a `computed` takes a dependency
 * on it and comes back when the grammar lands.
 */
function tokenise(text: string, language: string | null) {
  void version.value
  if (theme.value === 'plain') return null
  if (!highlighter) {
    void boot()
    return null
  }
  const id = blockOf(text, language ?? PLAIN)
  if (id !== PLAIN && !highlighter.getLoadedLanguages().includes(id)) {
    ensure(id)
    return null
  }
  if (!highlighter.getLoadedThemes().includes(theme.value)) return null
  try {
    return highlighter.codeToTokens(text, { lang: id as never, theme: theme.value as never }).tokens
  } catch {
    return null
  }
}

/**
 * Colours one line of code.
 *
 * A line on its own cannot be read in the context of the file it came from — a
 * line of TypeScript out of a `.vue` file is template text as far as the
 * grammar can tell, because the `<script>` tag that made it TypeScript is some
 * lines above and not in what was passed. So this is the fallback, and
 * [`highlightWhole`] is what a caller should reach for wherever it can.
 */
export function highlightLine(code: string, language: string | null): string {
  if (!code) return ''
  const tokens = tokenise(code, language)
  if (!tokens || tokens.length !== 1) return escapeHtml(code)
  return paint(tokens[0]!)
}

/**
 * Colours a whole file, and hands back one line of HTML per line of source.
 *
 * Worth the extra work over [`highlightLine`] wherever the whole text is at
 * hand, because the things a line cannot know about itself are exactly the ones
 * that matter: the body of a block comment, a string that runs on, and the
 * script inside a `.vue` file.
 *
 * Shiki hands back its tokens already grouped by line, so unlike the old
 * highlighter there are no spans left open across a newline to close and
 * reopen.
 */
export function highlightWhole(text: string, language: string | null): string[] {
  const source = text.split('\n')
  const tokens = tokenise(text, language)
  // A highlighter that lost a line somewhere is not worth trusting over the
  // plain text: the marks in the gutter are keyed by line number.
  if (!tokens || tokens.length !== source.length) return source.map(escapeHtml)
  return tokens.map(paint)
}

/**
 * Colours a fenced code block out of a comment body.
 *
 * The fence names its own language rather than a path doing it, so the name is
 * put through Shiki's own aliases — that way `ts`, `sh` and `yml` work — and a
 * fence naming something we have no grammar for is escaped rather than guessed
 * at.
 */
export function highlightBlock(code: string, info: string): string {
  return highlightWhole(code, ALIASES.get(info.toLowerCase()) ?? null).join('\n')
}

/**
 * File extension to grammar.
 *
 * Unlike the old highlighter these are the real grammars: `.vue` is `vue` and
 * `.tsx` is `tsx`, rather than both being approximated by
 * something close enough. A single-file component now comes out right a line at a time as well
 * as whole, so long as the whole file is what was passed.
 */
const BY_EXTENSION: Record<string, string> = {
  as: 'javascript',
  asd: 'clojure',
  bash: 'bash',
  bat: 'bat',
  c: 'c',
  cc: 'cpp',
  cfg: 'ini',
  cjs: 'javascript',
  clj: 'clojure',
  cljs: 'clojure',
  cmake: 'cmake',
  cmd: 'bat',
  conf: 'ini',
  cpp: 'cpp',
  cs: 'csharp',
  csproj: 'xml',
  css: 'css',
  csv: 'csv',
  cxx: 'cpp',
  dart: 'dart',
  diff: 'diff',
  ejs: 'html',
  env: 'ini',
  erb: 'erb',
  erl: 'erlang',
  ex: 'elixir',
  exs: 'elixir',
  fish: 'fish',
  gawk: 'awk',
  go: 'go',
  gql: 'graphql',
  gradle: 'groovy',
  graphql: 'graphql',
  groovy: 'groovy',
  h: 'c',
  handlebars: 'handlebars',
  hbs: 'handlebars',
  hcl: 'hcl',
  hpp: 'cpp',
  hrl: 'erlang',
  hs: 'haskell',
  htm: 'html',
  html: 'html',
  ini: 'ini',
  java: 'java',
  jl: 'julia',
  js: 'javascript',
  json: 'json',
  json5: 'json5',
  jsonc: 'jsonc',
  jsonl: 'json',
  jsx: 'jsx',
  kt: 'kotlin',
  kts: 'kotlin',
  less: 'less',
  lisp: 'lisp',
  lua: 'lua',
  m: 'objective-c',
  make: 'make',
  md: 'markdown',
  mdx: 'mdx',
  mjs: 'javascript',
  mk: 'make',
  ml: 'ocaml',
  mli: 'ocaml',
  mm: 'objective-cpp',
  ndjson: 'json',
  nix: 'nix',
  patch: 'diff',
  php: 'php',
  phtml: 'php',
  pl: 'perl',
  plist: 'xml',
  pm: 'perl',
  properties: 'properties',
  proto: 'proto',
  ps1: 'powershell',
  psm1: 'powershell',
  py: 'python',
  pyi: 'python',
  r: 'r',
  rb: 'ruby',
  resx: 'xml',
  rs: 'rust',
  sass: 'sass',
  sbt: 'scala',
  scala: 'scala',
  scss: 'scss',
  sh: 'shellscript',
  sql: 'sql',
  storyboard: 'xml',
  svelte: 'svelte',
  svg: 'xml',
  swift: 'swift',
  tex: 'latex',
  tf: 'terraform',
  tfvars: 'terraform',
  toml: 'toml',
  ts: 'typescript',
  tsx: 'tsx',
  txt: PLAIN,
  vim: 'viml',
  vue: 'vue',
  xaml: 'xml',
  xml: 'xml',
  xsd: 'xml',
  xsl: 'xml',
  yaml: 'yaml',
  yml: 'yaml',
  zsh: 'shellscript'
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
  '.dockerignore': 'ini',
  '.editorconfig': 'ini',
  '.env': 'ini',
  '.eslintrc': 'json',
  '.gitattributes': 'ini',
  '.gitconfig': 'ini',
  '.gitignore': 'ini',
  '.gitmodules': 'ini',
  '.npmrc': 'ini',
  '.prettierrc': 'json',
  '.profile': 'bash',
  '.vimrc': 'viml',
  '.zshrc': 'bash',
  'cargo.lock': 'toml',
  'cmakelists.txt': 'cmake',
  dockerfile: 'docker',
  gemfile: 'ruby',
  justfile: 'make',
  // The make grammar, not bash: a rule's target and its recipe are different
  // things, and bash colours neither of them.
  makefile: 'make',
  procfile: 'yaml',
  rakefile: 'ruby',
  vagrantfile: 'ruby'
}

/**
 * What to call the file, for the chip above a diff.
 *
 * Not the grammar it is coloured with: the grammar is an implementation detail
 * and naming it told the reader something about our highlighter rather than
 * about their file.
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
  // `Dockerfile.prod` is a Dockerfile: what follows the name is which
  // environment it builds, not what kind of file it is.
  if (file.startsWith('dockerfile.')) return BY_NAME.dockerfile ?? null
  const base = dotfileBase(file)
  return (base ? BY_NAME[base] : null) ?? null
}
