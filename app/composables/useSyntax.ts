import { computed, ref } from 'vue'
import { bundledThemesInfo } from 'shiki'
import { setHighlightTheme } from './useHighlight'

/**
 * Which theme code is coloured with.
 *
 * These are Shiki's own themes rather than a set of ours: sixty-five of them,
 * the same files VS Code loads, so a scheme somebody already knows by name is
 * there under that name. The list used to be eight hand-written sets of CSS
 * variables, which meant every new scheme was a stylesheet edit and a light
 * variant to hand-tune beside it.
 */

/** No colour at all, for reading a diff the way a terminal shows it. */
export const PLAIN = 'plain'

export interface SyntaxTheme {
  id: string
  name: string
  /** Which background it was drawn for, so a light theme is picked knowingly. */
  kind: 'Light' | 'Dark' | 'Plain'
}

export const SYNTAX_THEMES: SyntaxTheme[] = [
  { id: PLAIN, name: 'No colours', kind: 'Plain' },
  ...bundledThemesInfo.map((one) => ({
    id: one.id,
    name: one.displayName,
    kind: one.type === 'light' ? ('Light' as const) : ('Dark' as const)
  }))
]

const KNOWN = new Set(SYNTAX_THEMES.map((one) => one.id))

/**
 * Where the eight schemes that used to be here now point.
 *
 * A stored preference is a choice somebody made, so it survives the change
 * rather than being reset to the default. `theme` and `gitkraken` had no Shiki
 * equivalent — they were built here — so they land on the closest thing.
 */
const REPLACED: Record<string, string> = {
  theme: 'dark-plus',
  vscode: 'dark-plus',
  gitkraken: 'github-dark-dimmed',
  github: 'github-dark',
  onedark: 'one-dark-pro',
  monokai: 'monokai',
  solarized: 'solarized-dark'
}

const KEY = 'gitnoob.syntax'

/**
 * The default follows the window.
 *
 * A dark scheme under a light app theme is unreadable, and the app theme is
 * already on the root element by the time this runs. Read off the DOM rather
 * than imported so this composable does not have to pull the theme store, and
 * the git store behind it, in with it.
 */
function fallback(): string {
  try {
    return document.documentElement.dataset.light === undefined ? 'github-dark' : 'github-light'
  } catch {
    return 'github-dark'
  }
}

function stored(): string | null {
  try {
    const saved = localStorage.getItem(KEY)
    if (!saved) return null
    const moved = REPLACED[saved] ?? saved
    return KNOWN.has(moved) ? moved : null
  } catch {
    // No stored preference is not a problem.
    return null
  }
}

const syntax = ref<string>(PLAIN)

function setSyntax(id: string) {
  if (!KNOWN.has(id)) return
  syntax.value = id
  setHighlightTheme(id)
  try {
    localStorage.setItem(KEY, id)
  } catch {
    // A window that cannot remember the choice still shows the theme.
  }
}

// Read and applied as the composable loads, like the app theme, so code is
// never seen in one theme's colours and then repainted in another's.
if (typeof document !== 'undefined') {
  const first = stored() ?? fallback()
  syntax.value = first
  setHighlightTheme(first)
}

/** The picker's rows, in the shape `SearchSelect` takes. */
const choices = computed(() =>
  SYNTAX_THEMES.map((one) => ({
    value: one.id,
    label: one.name,
    note: one.kind === 'Plain' ? 'as a terminal would' : one.kind,
    // The id is searched as well as the name, so "mocha" and "catppuccin-mocha"
    // both find the same row.
    hint: one.id
  }))
)

export function useSyntax() {
  return { syntax, themes: SYNTAX_THEMES, choices, setSyntax }
}
