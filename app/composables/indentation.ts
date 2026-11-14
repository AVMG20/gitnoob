import { reactive } from 'vue'

/**
 * How the editor indents a file: tabs or spaces, how many, how wide a tab is.
 *
 * Decided in this order, the first that says anything winning:
 *   1. an override picked in the header for this file, this session;
 *   2. `.editorconfig` (read in Rust, `editorconfig.rs`);
 *   3. the file itself, from its leading whitespace;
 *   4. Settings → Editor, when it says Spaces or Tabs rather than Detect;
 *   5. the language's custom (4 for PHP, 2 for TypeScript, tabs for Go…);
 *   6. Settings → Editor's indent size, with spaces.
 */

export interface Indent {
  tabs: boolean
  /** Columns per level. */
  size: number
  /** Columns a tab is drawn as. */
  tabWidth: number
  /** Which rule decided it, for the header's tooltip. */
  from: 'override' | 'editorconfig' | 'file' | 'settings' | 'language' | 'default'
}

/** What `.editorconfig` says, as `indent_config` answers. */
export interface EditorconfigIndent {
  style?: string | null
  size?: number | null
  tab_width?: number | null
}

export interface IndentSettings {
  /** `detect`, `spaces` or `tabs`. */
  style?: string
  size?: number
  tabWidth?: number
}

/**
 * Tabs or spaces, and the width of a level, read off the file's own leading
 * whitespace. The width is the commonest step between one indented line and
 * the next, not the first line's indent, so a file whose first indented line
 * is a continuation still reads right. Steps of one (the ` *` of a doc
 * comment) say nothing. Null when the file has no indented lines.
 */
export function detectIndent(text: string): { tabs: boolean; size: number } | null {
  let tabbed = 0
  let spaced = 0
  const steps = new Map<number, number>()
  let previous = 0
  for (const line of text.split('\n')) {
    if (!line.trim()) continue
    const lead = /^[ \t]*/.exec(line)![0]
    if (lead.startsWith('\t')) tabbed++
    else if (lead.length) spaced++
    if (lead.includes('\t')) {
      previous = 0
      continue
    }
    const width = lead.length
    const step = Math.abs(width - previous)
    if (step >= 2 && step <= 8) steps.set(step, (steps.get(step) ?? 0) + 1)
    previous = width
  }
  if (!tabbed && !spaced) return null
  if (tabbed > spaced) return { tabs: true, size: 0 }
  let best = 0
  let count = 0
  for (const [step, seen] of [...steps].sort((a, b) => a[0] - b[0])) {
    // On a tie the smaller step wins: two levels at once is the usual way
    // to see a step twice the real one.
    if (seen > count) {
      best = step
      count = seen
    }
  }
  if (!best) return null
  return { tabs: false, size: best }
}

/** The indentation people expect in a language, for an empty or flat file. */
export function languageIndent(path: string): { tabs: boolean; size: number } | null {
  const name = path.slice(path.lastIndexOf('/') + 1).toLowerCase()
  if (name === 'makefile' || name === 'gnumakefile' || name.endsWith('.mk')) return { tabs: true, size: 0 }
  const extension = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : ''
  if (extension === 'go') return { tabs: true, size: 0 }
  if (['php', 'py', 'rs', 'java', 'cs', 'kt', 'swift', 'c', 'h', 'cpp', 'hpp'].includes(extension)) {
    return { tabs: false, size: 4 }
  }
  if (
    ['js', 'mjs', 'cjs', 'jsx', 'ts', 'mts', 'cts', 'tsx', 'vue', 'json', 'yaml', 'yml', 'css', 'scss', 'less', 'html', 'htm'].includes(
      extension
    )
  ) {
    return { tabs: false, size: 2 }
  }
  return null
}

const clampWidth = (value: number | null | undefined, fallback: number) =>
  value && value >= 1 && value <= 16 ? Math.round(value) : fallback

/** Puts the rules together, in the order at the top of this file. */
export function resolveIndent(input: {
  path: string
  text: string
  editorconfig?: EditorconfigIndent | null
  settings?: IndentSettings | null
  override?: { tabs: boolean; size: number } | null
}): Indent {
  const settingsSize = clampWidth(input.settings?.size, 4)
  const tabWidth = clampWidth(input.editorconfig?.tab_width ?? input.settings?.tabWidth, 4)
  const tabsAt = (from: Indent['from']): Indent => ({ tabs: true, size: tabWidth, tabWidth, from })
  const spacesAt = (size: number, from: Indent['from']): Indent => ({ tabs: false, size, tabWidth, from })

  if (input.override) {
    return input.override.tabs ? tabsAt('override') : spacesAt(clampWidth(input.override.size, 4), 'override')
  }
  const config = input.editorconfig
  if (config && (config.style || config.size)) {
    const tabs = config.style ? config.style === 'tab' : false
    if (tabs) return { tabs: true, size: clampWidth(config.size, tabWidth), tabWidth, from: 'editorconfig' }
    // A size with no style: spaces of that size, unless the file is tabbed.
    if (!config.style && detectIndent(input.text)?.tabs) return tabsAt('editorconfig')
    return spacesAt(clampWidth(config.size, detectIndent(input.text)?.size || settingsSize), 'editorconfig')
  }
  const found = detectIndent(input.text)
  if (found) return found.tabs ? tabsAt('file') : spacesAt(found.size, 'file')
  if (input.settings?.style === 'tabs') return tabsAt('settings')
  if (input.settings?.style === 'spaces') return spacesAt(settingsSize, 'settings')
  const custom = languageIndent(input.path)
  if (custom) return custom.tabs ? tabsAt('language') : spacesAt(custom.size, 'language')
  return spacesAt(settingsSize, 'default')
}

/** "Spaces: 4" or "Tabs", for the header. */
export function indentLabel(indent: Indent) {
  return indent.tabs ? 'Tabs' : `Spaces: ${indent.size}`
}

/** Overrides picked in the header, by editor key, for this session. */
export const indentOverrides = reactive<Record<string, { tabs: boolean; size: number }>>({})
