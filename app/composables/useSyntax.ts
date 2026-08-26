import { ref } from 'vue'

export type SyntaxId =
  | 'theme'
  | 'vscode'
  | 'gitkraken'
  | 'github'
  | 'onedark'
  | 'monokai'
  | 'solarized'
  | 'plain'

export interface Scheme {
  id: SyntaxId
  name: string
  /** Where the colours come from, so the choice is made by recognition. */
  from: string
  /** Keyword, string, comment — the three that tell two schemes apart. */
  swatch: [string, string, string]
}

/**
 * The syntax schemes on offer.
 *
 * Each is a set of `--hl-*` variables in the stylesheet with a light variant
 * beside it, so a scheme holds under all eighteen app themes rather than only
 * the dark ones. `theme` is the one that ships: it follows whatever the app
 * theme is, which is what every one of these was before there was a choice.
 */
export const SCHEMES: Scheme[] = [
  { id: 'theme', name: 'Match the theme', from: 'JetBrains Darcula, and its light scheme', swatch: ['#cc7832', '#6a8759', '#7f8c8d'] },
  { id: 'vscode', name: 'VS Code', from: 'Dark+ and Light+', swatch: ['#569cd6', '#ce9178', '#6a9955'] },
  { id: 'gitkraken', name: 'GitKraken', from: 'its diff viewer', swatch: ['#8ab4f8', '#a5d6a7', '#7c8895'] },
  { id: 'github', name: 'GitHub', from: 'github.com', swatch: ['#ff7b72', '#a5d6ff', '#8b949e'] },
  { id: 'onedark', name: 'One Dark', from: 'Atom', swatch: ['#c678dd', '#98c379', '#5c6370'] },
  { id: 'monokai', name: 'Monokai', from: 'Sublime Text', swatch: ['#f92672', '#e6db74', '#75715e'] },
  { id: 'solarized', name: 'Solarized', from: 'Ethan Schoonover', swatch: ['#859900', '#2aa198', '#657b83'] },
  { id: 'plain', name: 'No colours', from: 'one colour, as a terminal would', swatch: ['#8b98a5', '#8b98a5', '#8b98a5'] }
]

const KEY = 'gitnoob.syntax'

const syntax = ref<SyntaxId>('theme')

function apply(id: SyntaxId) {
  document.documentElement.dataset.syntax = id
}

function setSyntax(id: SyntaxId) {
  syntax.value = id
  apply(id)
  try {
    localStorage.setItem(KEY, id)
  } catch {
    // A window that cannot remember the choice still shows the scheme.
  }
}

// Read and applied as the composable loads, like the theme, so code is never
// seen in one scheme's colours and then repainted in another's.
try {
  const saved = localStorage.getItem(KEY) as SyntaxId | null
  if (saved && SCHEMES.some((one) => one.id === saved)) syntax.value = saved
} catch {
  // No stored preference is not a problem.
}
apply(syntax.value)

export function useSyntax() {
  return { syntax, schemes: SCHEMES, setSyntax }
}
