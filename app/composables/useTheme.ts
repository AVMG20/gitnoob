import { ref } from 'vue'

export type ThemeId =
  | 'slate'
  | 'dusk'
  | 'fjord'
  | 'plum'
  | 'moss'
  | 'ink'
  | 'void'
  | 'jade'
  | 'crimson'
  | 'ember'
  | 'pine'
  | 'obsidian'
  | 'mono'
  | 'paper'
  | 'mist'
  | 'sand'
  | 'frost'
  | 'lilac'

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
  { id: 'plum', name: 'Plum', kind: 'Semi-dark', swatch: ['#1a151d', '#e879b8', '#e0d6e4'] },
  { id: 'moss', name: 'Moss', kind: 'Semi-dark', swatch: ['#171a15', '#a3cf5c', '#d9dfd1'] },
  { id: 'ink', name: 'Ink', kind: 'Dark', swatch: ['#0d0f12', '#4f9cf9', '#ccd4dc'] },
  { id: 'void', name: 'Void', kind: 'Dark', swatch: ['#000000', '#35bec9', '#d7dee6'] },
  // Void's own colour, then the same black under three others.
  { id: 'jade', name: 'Jade', kind: 'Dark', swatch: ['#000000', '#2fe08a', '#d4e0d7'] },
  { id: 'crimson', name: 'Crimson', kind: 'Dark', swatch: ['#000000', '#ff6b5b', '#e2d8d8'] },
  { id: 'ember', name: 'Ember', kind: 'Dark', swatch: ['#000000', '#ff8c42', '#e3dbcf'] },
  { id: 'pine', name: 'Pine', kind: 'Dark', swatch: ['#0e1310', '#4fd18b', '#cfdcd3'] },
  { id: 'obsidian', name: 'Obsidian', kind: 'Dark', swatch: ['#08090b', '#8b7df7', '#d5d7dc'] },
  { id: 'mono', name: 'Mono', kind: 'Dark', swatch: ['#0a0a0a', '#e8e8e8', '#e6e6e6'] },
  { id: 'paper', name: 'Paper', kind: 'Light', swatch: ['#ffffff', '#2f7fe0', '#23292f'] },
  { id: 'mist', name: 'Mist', kind: 'Light', swatch: ['#eff1f5', '#3979dd', '#2a313a'] },
  { id: 'sand', name: 'Sand', kind: 'Light', swatch: ['#faf7f2', '#c25f3a', '#33302a'] },
  { id: 'frost', name: 'Frost', kind: 'Light', swatch: ['#f7fafb', '#0f8f9e', '#23323a'] },
  { id: 'lilac', name: 'Lilac', kind: 'Light', swatch: ['#fbf9fe', '#7c4dd6', '#2b2735'] }
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
