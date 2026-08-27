import { ref } from 'vue'
import { refreshLanes } from './useGit'
import { DEFAULT_THEME, THEMES, type ThemeId } from './themeList'

/**
 * How much contrast the window is asked for, over and above the theme.
 *
 * A separate axis on purpose. What a theme is for is which colours the window
 * is in; how hard the quiet parts of it are to read is a different question,
 * and one with a different answer for different people and different screens.
 * Kept apart, one setting raises the floor across all eighteen themes; folded
 * together, it would be eighteen more themes to keep.
 */
export type Contrast = 'cosy' | 'normal' | 'high'

export const CONTRASTS: { id: Contrast; name: string; note: string }[] = [
  { id: 'cosy', name: 'Cosy', note: 'softer' },
  { id: 'normal', name: 'Normal', note: '4.5:1 or better' },
  { id: 'high', name: 'High', note: 'stronger' }
]

const KEY = 'gitnoob.theme'
const CONTRAST_KEY = 'gitnoob.contrast'

const theme = ref<ThemeId>(DEFAULT_THEME)
const contrast = ref<Contrast>('normal')

/**
 * Puts the choice on the root element, where the stylesheet can see it.
 *
 * Light themes are marked as such alongside the theme's own name, so anything
 * that needs a different value on a light background — the syntax schemes do —
 * can say so once rather than restating a list of light themes that grows.
 */
function apply(id: ThemeId, level: Contrast) {
  const root = document.documentElement
  root.dataset.theme = id
  if (level === 'normal') delete root.dataset.contrast
  else root.dataset.contrast = level
  if (THEMES.find((one) => one.id === id)?.kind === 'Light') root.dataset.light = ''
  else delete root.dataset.light
  // The graph's colours are read from the stylesheet, and the stylesheet has
  // just changed under it.
  refreshLanes()
}

function remember(key: string, value: string) {
  try {
    localStorage.setItem(key, value)
  } catch {
    // A window that cannot remember the choice still shows it.
  }
}

function setTheme(id: ThemeId) {
  theme.value = id
  apply(id, contrast.value)
  remember(KEY, id)
}

function setContrast(level: Contrast) {
  contrast.value = level
  apply(theme.value, level)
  remember(CONTRAST_KEY, level)
}

// Read and apply once, when the composable is first imported — before the app
// paints, so a light theme never flashes in as dark first.
try {
  const saved = localStorage.getItem(KEY) as ThemeId | null
  if (saved && THEMES.some((one) => one.id === saved)) theme.value = saved
  const level = localStorage.getItem(CONTRAST_KEY) as Contrast | null
  if (level && CONTRASTS.some((one) => one.id === level)) contrast.value = level
} catch {
  // Nothing saved, or no storage at all: the defaults stand.
}
apply(theme.value, contrast.value)

export function useTheme() {
  return { theme, themes: THEMES, setTheme, contrast, contrasts: CONTRASTS, setContrast }
}
