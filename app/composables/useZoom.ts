import { ref } from 'vue'

/**
 * How large everything is drawn.
 *
 * A view preference like the theme, so it lives in the browser's own storage
 * rather than in the config file every profile shares, and is applied as the
 * composable loads rather than on mount — a window that painted at one size and
 * then jumped to another is worse than a window that took a moment to appear.
 *
 * The webview's own zoom rather than a font size. Nearly every size in the
 * stylesheet is stated in pixels, and so are the row height the commit list
 * virtualizes against and the geometry of the graph it draws in SVG; scaling
 * the text alone would leave all three behind and the rows would grow out of
 * the lanes drawn in them. Zoom moves the whole coordinate space, so the
 * relationship between what CSS says and what the list measures holds.
 */

const KEY = 'gitnoob.zoom'

/** What the buttons offer. 1 is the size the app was designed at. */
export const ZOOM_STEPS = [0.8, 0.9, 1, 1.1, 1.25, 1.4, 1.6] as const

const MIN = ZOOM_STEPS[0]
const MAX = ZOOM_STEPS[ZOOM_STEPS.length - 1]!

const zoom = ref(1)

function apply(factor: number) {
  // Only the Tauri host can zoom; `npm run dev` in a browser has its own.
  if (!('__TAURI_INTERNALS__' in window)) return
  import('@tauri-apps/api/webview')
    .then(({ getCurrentWebview }) => getCurrentWebview().setZoom(factor))
    .catch(() => {
      // An older host without the permission still shows the app at 1.
    })
}

function setZoom(factor: number) {
  zoom.value = Math.min(MAX, Math.max(MIN, factor))
  apply(zoom.value)
  try {
    localStorage.setItem(KEY, String(zoom.value))
  } catch {
    // A window that cannot remember the size still draws at it.
  }
}

/** Moves to the next step up or down, so a keystroke lands on a round number. */
function step(by: 1 | -1) {
  const at = ZOOM_STEPS.findIndex((one) => Math.abs(one - zoom.value) < 0.001)
  const from = at === -1 ? ZOOM_STEPS.findIndex((one) => one >= zoom.value) : at
  const next = ZOOM_STEPS[Math.min(ZOOM_STEPS.length - 1, Math.max(0, from + by))]
  if (next) setZoom(next)
}

try {
  const saved = Number(localStorage.getItem(KEY))
  if (Number.isFinite(saved) && saved >= MIN && saved <= MAX) zoom.value = saved
} catch {
  // No stored preference is not a problem.
}
apply(zoom.value)

export function useZoom() {
  return {
    zoom,
    steps: ZOOM_STEPS,
    setZoom,
    zoomIn: () => step(1),
    zoomOut: () => step(-1),
    reset: () => setZoom(1)
  }
}
