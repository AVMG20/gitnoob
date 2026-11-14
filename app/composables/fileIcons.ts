/**
 * Which icon a file gets: a glyph and a tone, from its name.
 *
 * Whole names first (package.json, Dockerfile, .gitignore, README…), then
 * `.blade.php`, then the extension. Anything else is a plain file. Pure and
 * cached by the lowercased name or extension, so a long virtualised list
 * asks the same question for free and gets the same object back.
 *
 * The glyphs are drawn in FileIcon.vue; the tones are CSS there too, mixed
 * with the theme's text colour so every one reads on light and dark themes.
 */

export type Glyph =
  | 'file'
  | 'folder'
  | 'folder-open'
  | 'letters'
  | 'braces'
  | 'markup'
  | 'hash'
  | 'terminal'
  | 'sliders'
  | 'text'
  | 'table'
  | 'image'
  | 'archive'
  | 'lock'
  | 'git'
  | 'box'
  | 'book'
  | 'key'
  | 'scale'
  | 'vue'

export interface FileIconSpec {
  glyph: Glyph
  /** The colour class: `t-<tone>`. */
  tone: string
  /** For `letters`: what the badge says, one or two characters. */
  text?: string
  /** What it is, for a tooltip or a screen reader. */
  label: string
}

const icon = (glyph: Glyph, tone: string, label: string, text?: string): FileIconSpec =>
  Object.freeze({ glyph, tone, label, text })

const letters = (text: string, tone: string, label: string) => icon('letters', tone, label, text)

const GENERIC = icon('file', 'plain', 'File')

const PHP = letters('P', 'php', 'PHP')
const BLADE = letters('B', 'blade', 'Blade template')
const JS = letters('JS', 'js', 'JavaScript')
const TS = letters('TS', 'ts', 'TypeScript')
const JSX = letters('JX', 'react', 'JSX')
const TSX = letters('TX', 'react', 'TSX')
const SHELL = icon('terminal', 'shell', 'Shell script')
const CONFIG = icon('sliders', 'config', 'Settings')
const IMAGE = icon('image', 'image', 'Image')
const ARCHIVE = icon('archive', 'archive', 'Archive')
const LOCK = icon('lock', 'lock', 'Lock file')
const ENV = icon('key', 'env', 'Environment')
const DOCKER = icon('box', 'docker', 'Docker')
const README = icon('book', 'readme', 'Read me')
const LICENSE = icon('scale', 'license', 'Licence')
const GIT = icon('git', 'git', 'Git')

/** By extension, lowercase, without the dot. */
const BY_EXT: Record<string, FileIconSpec> = {
  php: PHP,
  phtml: PHP,
  js: JS,
  mjs: JS,
  cjs: JS,
  ts: TS,
  mts: TS,
  cts: TS,
  jsx: JSX,
  tsx: TSX,
  vue: icon('vue', 'vue', 'Vue component'),
  go: letters('GO', 'go', 'Go'),
  rs: letters('RS', 'rust', 'Rust'),
  py: letters('PY', 'python', 'Python'),
  rb: letters('RB', 'ruby', 'Ruby'),
  java: letters('J', 'java', 'Java'),
  kt: letters('KT', 'kotlin', 'Kotlin'),
  kts: letters('KT', 'kotlin', 'Kotlin'),
  swift: letters('SW', 'swift', 'Swift'),
  c: letters('C', 'c', 'C'),
  h: letters('H', 'c', 'C header'),
  cpp: letters('C+', 'cpp', 'C++'),
  cc: letters('C+', 'cpp', 'C++'),
  cxx: letters('C+', 'cpp', 'C++'),
  hpp: letters('H+', 'cpp', 'C++ header'),
  cs: letters('C#', 'csharp', 'C#'),
  sh: SHELL,
  bash: SHELL,
  zsh: SHELL,
  fish: SHELL,
  nu: SHELL,
  ps1: SHELL,
  json: icon('braces', 'json', 'JSON'),
  jsonc: icon('braces', 'json', 'JSON'),
  yaml: icon('sliders', 'yaml', 'YAML'),
  yml: icon('sliders', 'yaml', 'YAML'),
  toml: icon('sliders', 'toml', 'TOML'),
  ini: CONFIG,
  conf: CONFIG,
  cfg: CONFIG,
  properties: CONFIG,
  editorconfig: CONFIG,
  xml: icon('markup', 'xml', 'XML'),
  html: icon('markup', 'html', 'HTML'),
  htm: icon('markup', 'html', 'HTML'),
  css: icon('hash', 'css', 'CSS'),
  scss: icon('hash', 'scss', 'Sass'),
  sass: icon('hash', 'scss', 'Sass'),
  less: icon('hash', 'less', 'Less'),
  md: letters('M', 'markdown', 'Markdown'),
  markdown: letters('M', 'markdown', 'Markdown'),
  mdx: letters('M', 'markdown', 'MDX'),
  sql: letters('SQ', 'sql', 'SQL'),
  svg: icon('image', 'svg', 'SVG'),
  png: IMAGE,
  jpg: IMAGE,
  jpeg: IMAGE,
  gif: IMAGE,
  webp: IMAGE,
  ico: IMAGE,
  bmp: IMAGE,
  avif: IMAGE,
  lock: LOCK,
  env: ENV,
  txt: icon('text', 'text', 'Text'),
  log: icon('text', 'text', 'Log'),
  csv: icon('table', 'csv', 'CSV'),
  tsv: icon('table', 'csv', 'TSV'),
  pdf: icon('text', 'pdf', 'PDF'),
  zip: ARCHIVE,
  tar: ARCHIVE,
  gz: ARCHIVE,
  tgz: ARCHIVE,
  bz2: ARCHIVE,
  xz: ARCHIVE,
  '7z': ARCHIVE,
  rar: ARCHIVE
}

/** By whole name, lowercase. */
const BY_NAME: Record<string, FileIconSpec> = {
  'package.json': letters('N', 'npm', 'npm package'),
  'package-lock.json': LOCK,
  'composer.json': letters('C', 'composer', 'Composer package'),
  'composer.lock': LOCK,
  'cargo.toml': icon('box', 'rust', 'Cargo manifest'),
  'cargo.lock': LOCK,
  'yarn.lock': LOCK,
  'pnpm-lock.yaml': LOCK,
  'go.mod': letters('GO', 'go', 'Go module'),
  'go.sum': LOCK,
  dockerfile: DOCKER,
  '.dockerignore': DOCKER,
  '.gitignore': GIT,
  '.gitattributes': GIT,
  '.gitmodules': GIT,
  '.gitkeep': GIT,
  artisan: letters('A', 'laravel', 'Laravel Artisan'),
  makefile: icon('terminal', 'make', 'Makefile'),
  gnumakefile: icon('terminal', 'make', 'Makefile'),
  '.editorconfig': CONFIG,
  '.bashrc': SHELL,
  '.bash_profile': SHELL,
  '.zshrc': SHELL,
  '.zshenv': SHELL,
  '.zprofile': SHELL,
  '.profile': SHELL,
  '.npmrc': letters('N', 'npm', 'npm settings')
}

/** Names that start the same way: `.env.local`, `README.md`, `docker-compose.prod.yml`. */
const BY_PREFIX: [string, FileIconSpec][] = [
  ['.env', ENV],
  ['readme', README],
  ['license', LICENSE],
  ['licence', LICENSE],
  ['copying', LICENSE],
  ['docker-compose', DOCKER],
  ['compose.y', DOCKER],
  ['dockerfile.', DOCKER]
]

const FOLDER = icon('folder', 'folder', 'Folder')
const FOLDER_OPEN = icon('folder-open', 'folder', 'Folder')

/** Folders that get a tint of their own, kept faint. */
const FOLDER_TONES: Record<string, string> = {
  '.git': 'git',
  '.github': 'github',
  node_modules: 'npm',
  vendor: 'composer',
  src: 'src',
  app: 'src',
  tests: 'tests',
  test: 'tests',
  public: 'public'
}

const folders = new Map<string, FileIconSpec>()
const cache = new Map<string, FileIconSpec>()

function baseName(path: string) {
  const cut = path.lastIndexOf('/')
  return cut >= 0 ? path.slice(cut + 1) : path
}

/** The icon for a folder, open or not. */
export function folderIcon(path: string, open = false): FileIconSpec {
  const name = baseName(path).toLowerCase()
  const key = `${open ? 1 : 0}${name}`
  let found = folders.get(key)
  if (!found) {
    const tone = FOLDER_TONES[name]
    found = tone
      ? icon(open ? 'folder-open' : 'folder', `folder-${tone}`, 'Folder')
      : open
        ? FOLDER_OPEN
        : FOLDER
    folders.set(key, found)
  }
  return found
}

/** The icon for a file, by its name. */
export function fileIcon(path: string): FileIconSpec {
  const name = baseName(path).toLowerCase()
  const known = cache.get(name)
  if (known) return known
  const found = lookup(name)
  // Names repeat (index.ts, README.md); a cache by name keeps lookups to one.
  if (cache.size > 5000) cache.clear()
  cache.set(name, found)
  return found
}

/**
 * Whether a name starts with a prefix as a word of its own: `readme`,
 * `readme.md` and `README-dev`, but not `readmeparser.ts`, which is code.
 * A prefix that ends in a dot has said where it stops.
 */
function startsWord(name: string, prefix: string) {
  if (!name.startsWith(prefix)) return false
  if (/[.]$/.test(prefix) || prefix === 'compose.y') return true
  const next = name[prefix.length]
  return next === undefined || next === '.' || next === '-' || next === '_'
}

function lookup(name: string): FileIconSpec {
  const whole = BY_NAME[name]
  if (whole) return whole
  // A Blade view is one whatever it is called: `readme.blade.php` too.
  if (name.endsWith('.blade.php')) return BLADE
  for (const [prefix, found] of BY_PREFIX) if (startsWord(name, prefix)) return found
  const dot = name.lastIndexOf('.')
  // No extension, or only a leading dot (`.prettierrc`): a plain file.
  if (dot <= 0) return name.startsWith('.') ? (BY_EXT[name.slice(1)] ?? GENERIC) : GENERIC
  return BY_EXT[name.slice(dot + 1)] ?? GENERIC
}

/** For a list that mixes both. */
export function iconFor(path: string, folder = false, open = false): FileIconSpec {
  return folder ? folderIcon(path, open) : fileIcon(path)
}
