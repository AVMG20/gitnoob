import { reactive } from 'vue'
import { invoke } from './useInvoke'
import { useConfig } from './useConfig'
import { useToasts } from './useToasts'

/**
 * Framework support (`src-tauri/src/frameworks/`): which framework the open
 * repository uses, ⌘-click through its config files and strings, and the
 * "wiring" view an outside tool gives of a class (Magento's magequery).
 *
 * Detection runs in the backend with a few `stat` calls and is cached; this
 * only asks once per repository opened and keeps the answer for the chip in
 * the title bar.
 */

export interface FrameworkInfo {
  id: string
  name: string
  version: string | null
  how: string
  /** Whether the wiring view can run here. */
  wiring: boolean
}

/** A place a framework link leads to. */
export interface FrameworkTarget {
  path: string
  line: number
  column: number
  kind: string
  text: string
  dependency: boolean
  /** `adminhtml · module`, `frontend · Vendor/theme · override`. */
  label: string | null
}

export interface FrameworkLinks {
  from: number
  to: number
  title: string
  note: string | null
  targets: FrameworkTarget[]
  /** The class the token names, for the wiring link. */
  class: string | null
}

export interface WiringRow {
  label: string
  detail: string | null
  class: string | null
  path: string | null
  line: number | null
}

export interface Wiring {
  class: string
  kind: string | null
  module: string | null
  path: string | null
  preference: string | null
  sections: { title: string; rows: WiringRow[] }[]
}

export const framework = reactive({
  info: null as FrameworkInfo | null,
  /** The repository `info` is about. */
  repo: null as string | null
})

/** The wiring popover. */
export const wiring = reactive({
  open: false,
  loading: false,
  class: '',
  error: '',
  result: null as Wiring | null
})

/** Whether framework support is on in Settings. */
export function frameworksOn() {
  return useConfig().settings.value?.frameworks !== false
}

/** What the chip in the title bar says in its tooltip: "Magento 2.4.7 detected". */
export function frameworkLabel(info: FrameworkInfo) {
  return `${info.name}${info.version ? ` ${info.version}` : ''} detected`
}

/** Asks which framework a repository uses, for the title bar. */
export async function loadFramework(repo: string | null) {
  framework.repo = repo
  if (!repo || !frameworksOn()) {
    framework.info = null
    return
  }
  const info = await invoke<FrameworkInfo | null>('framework_info').catch(() => null)
  if (framework.repo === repo) framework.info = info ?? null
}

/** The files a framework link can start from: config, templates, scripts, PHP. */
const LINKED = /\.(xml|phtml|php|js|json|html)$/i

/** Whether ⌘-click in this file should ask the framework first. */
export function frameworkFile(path: string) {
  return !!framework.info && frameworksOn() && LINKED.test(path)
}

/**
 * Where a ⌘-click at `offset` leads in the framework's terms, or null to
 * leave it to the language.
 */
export async function frameworkLinks(path: string, offset: number, text: string | null) {
  if (!frameworkFile(path)) return null
  return invoke<FrameworkLinks | null>('framework_links', { path, offset, text }).catch(() => null)
}

/** A path to a view file, with a folder: `rma/request_created.html`, `css/styles.css`. */
const VIEW_PATH = /\/[^/]*\.(html|phtml|js|css|less|json|svg|png|jpe?g|gif|webp|ico|woff2?|ttf|eot)$/i

/**
 * The token a framework link would cover around `pos` in a line of text:
 * a namespaced class (`Foo\Bar\Baz`), a `Vendor_Module::path`, a
 * `Vendor_Module/js/x`, a `.phtml` or a view file under a folder
 * (`rma/request_created.html`, `css/styles.css`). Null for a plain word,
 * which the editor's own word rule underlines.
 */
export function frameworkToken(line: string, pos: number): { from: number; to: number } | null {
  const char = /[A-Za-z0-9_\\:/.\-]/
  let from = pos
  let to = pos
  while (from > 0 && char.test(line[from - 1]!)) from--
  while (to < line.length && char.test(line[to]!)) to++
  // Not the punctuation at either end.
  while (from < to && /[:/.\-\\]/.test(line[from]!)) from++
  while (to > from && /[:/.\-]/.test(line[to - 1]!)) to--
  const token = line.slice(from, to)
  if (to - from < 3 || pos < from || pos > to) return null
  const shaped =
    token.includes('\\') ||
    token.includes('::') ||
    /^[A-Z][A-Za-z0-9]*_[A-Za-z0-9]+\//.test(token) ||
    /\.phtml$/.test(token) ||
    VIEW_PATH.test(token)
  return shaped ? { from, to } : null
}

/** Runs the framework's tool on a class and shows what it says. */
export async function openWiring(fqn: string) {
  wiring.open = true
  wiring.loading = true
  wiring.class = fqn
  wiring.error = ''
  wiring.result = null
  try {
    const found = await invoke<Wiring | null>('framework_wiring', { class: fqn })
    if (wiring.class !== fqn) return
    if (!found) wiring.error = 'No framework is detected in this repository.'
    wiring.result = found
  } catch (error) {
    if (wiring.class === fqn) wiring.error = String(error)
  } finally {
    if (wiring.class === fqn) wiring.loading = false
  }
}

export function closeWiring() {
  wiring.open = false
}

/** The class a PHP file declares, fully qualified, from its text. */
export function classOfText(text: string): string | null {
  const name = /^\s*(?:(?:abstract|final|readonly)\s+)*(?:class|interface|trait|enum)\s+([A-Za-z_][A-Za-z0-9_]*)/m.exec(text)
  if (!name) return null
  const space = /^\s*namespace\s+([A-Za-z0-9_\\]+)\s*;/m.exec(text)
  return space ? `${space[1]}\\${name[1]}` : name[1]!
}

/** The ⋯ menu's "Magento wiring": the class the file declares. */
export async function openWiringForFile(path: string) {
  const read = await invoke<{ text: string }>('read_file', { path }).catch(() => null)
  const fqn = read ? classOfText(read.text) : null
  if (!fqn) {
    useToasts().info('No class in this file')
    return
  }
  await openWiring(fqn)
}
