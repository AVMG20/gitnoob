import { onMounted, onUnmounted } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { goBack, goForward } from './useTabs'

/**
 * The mouse's back and forward buttons.
 *
 * Two ways in, because no one way works everywhere. The page sees them as
 * buttons 3 and 4 on Windows and Linux; WebView2 would also run its own
 * history navigation on them, which in a single-page app is a blank window,
 * so the default is always prevented. macOS's web view does not reliably pass
 * them to the page at all, so the backend watches for them natively and sends
 * a `nav-button` event. Where both arrive for one press, the second is dropped.
 */

export type NavDirection = 'back' | 'forward'
type Source = 'page' | 'native'

/** How long apart one press can arrive by the two routes: native on the way
    down, the page on the way up, so as long as the button is held. Each copy
    is used up by its twin, so a long window never eats a second press. */
const SAME_PRESS_MS = 10_000

let pending: { direction: NavDirection; source: Source; at: number } | null = null

/**
 * Acts on one button press, unless it is the other route's copy of a press
 * already acted on. Returns whether it navigated.
 */
export function navFrom(direction: NavDirection, source: Source, at = Date.now()) {
  // A dialog, the search window or the conflict resolver is on top: the
  // press is not about what is behind it.
  if (typeof document !== 'undefined' && document.querySelector('.scrim, .overlay')) {
    pending = { direction, source, at }
    return false
  }
  if (
    pending &&
    pending.source !== source &&
    pending.direction === direction &&
    at - pending.at < SAME_PRESS_MS
  ) {
    pending = null
    return false
  }
  pending = { direction, source, at }
  if (direction === 'back') goBack()
  else goForward()
  return true
}

/** For tests: forgets the last press. */
export function resetNav() {
  pending = null
}

function directionOf(button: number): NavDirection | null {
  if (button === 3) return 'back'
  if (button === 4) return 'forward'
  return null
}

/** Down and click only stop the web view's own navigation; up is what acts. */
function swallow(event: MouseEvent) {
  if (directionOf(event.button)) event.preventDefault()
}

function onUp(event: MouseEvent) {
  const direction = directionOf(event.button)
  if (!direction) return
  event.preventDefault()
  navFrom(direction, 'page')
}

/** Listens for the buttons for as long as the calling component is mounted. */
export function useNavButtons() {
  let unlisten: UnlistenFn | undefined
  let gone = false

  onMounted(async () => {
    window.addEventListener('mouseup', onUp)
    window.addEventListener('mousedown', swallow)
    window.addEventListener('auxclick', swallow)
    const off = await listen<NavDirection>('nav-button', ({ payload }) => {
      if (payload === 'back' || payload === 'forward') navFrom(payload, 'native')
    }).catch(() => undefined)
    if (gone) off?.()
    else unlisten = off
  })

  onUnmounted(() => {
    gone = true
    window.removeEventListener('mouseup', onUp)
    window.removeEventListener('mousedown', swallow)
    window.removeEventListener('auxclick', swallow)
    unlisten?.()
  })
}
