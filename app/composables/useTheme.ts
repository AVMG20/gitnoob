import { ref } from 'vue'

export type ThemeId = 'slate' | 'dusk' | 'fjord' | 'ink' | 'void' | 'pine' | 'paper' | 'mist' | 'sand'

export interface Theme {
  id: ThemeId
  name: string
  /** The group it belongs to, shown over the swatches. */
  kind: 'Light' | 'Semi-dark' | 'Dark'
  /** Background, accent, text — the three colours a card is painted with. */
  swatch: [string, string, string]
}

/** Slate first: it is the default, and the list is the order they appear in. */
export const THEMES: Theme[] = [
  { id: 'slate', name: 'Slate', kind: 'Semi-dark', swatch: ['#14181d', '#4f9cf9', '#d7dee6'] },
  { id: 'dusk', name: 'Dusk', kind: 'Semi-dark', swatch: ['#1d1a16', '#e3943a', '#e2dcd2'] },
  { id: 'fjord', name: 'Fjord', kind: 'Semi-dark', swatch: ['#161a24', '#7aa2f7', '#d6dceb'] },
  { id: 'ink', name: 'Ink', kind: 'Dark', swatch: ['#0d0f12', '#4f9cf9', '#ccd4dc'] },
  { id: 'void', name: 'Void', kind: 'Dark', swatch: ['#000000', '#35bec9', '#d7dee6'] },
  { id: 'pine', name: 'Pine', kind: 'Dark', swatch: ['#0e1310', '#4fd18b', '#cfdcd3'] },
  { id: 'paper', name: 'Paper', kind: 'Light', swatch: ['#ffffff', '#2f7fe0', '#23292f'] },
  { id: 'mist', name: 'Mist', kind: 'Light', swatch: ['#eff1f5', '#3979dd', '#2a313a'] },
  { id: 'sand', name: 'Sand', kind: 'Light', swatch: ['#faf7f2', '#c25f3a', '#33302a'] }
]

const KEY = 'gitnoob.theme'

const theme = ref<ThemeId>('slate')

function apply(id: ThemeId) {
  document.documentElement.dataset.theme = id
}

function setTheme(id: ThemeId) {
  theme.value = id
  apply(id)
  try {
    localStorage.setItem(KEY, id)
  } catch {
    // A window that cannot remember the choice still shows the theme.
  }
}

// Read and apply once, when the composable is first imported — before the app
// paints, so a light theme never flashes in as dark first.
try {
  const saved = localStorage.getItem(KEY) as ThemeId | null
  if (saved && THEMES.some((one) => one.id === saved)) {
    theme.value = saved
  }
  apply(theme.value)
} catch {
  apply(theme.value)
}

export function useTheme() {
  return { theme, themes: THEMES, setTheme }
}
