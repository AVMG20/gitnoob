import { reactive, watch } from 'vue'

const KEY = 'gitnoob.layout'

const limits = { sidebar: [180, 460], panel: [280, 720] } as const

const layout = reactive({
  sidebar: 252,
  panel: 400,
  /** Which edge is being dragged, if any. */
  dragging: null as 'sidebar' | 'panel' | null
})

try {
  const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}')
  if (typeof saved.sidebar === 'number') layout.sidebar = saved.sidebar
  if (typeof saved.panel === 'number') layout.panel = saved.panel
} catch {
  // A missing or unreadable preference just means the defaults.
}

watch(
  () => [layout.sidebar, layout.panel],
  () => {
    try {
      localStorage.setItem(
        KEY,
        JSON.stringify({ sidebar: layout.sidebar, panel: layout.panel })
      )
    } catch {
      // Private windows can refuse to store; the layout still works this session.
    }
  }
)

const clamp = (value: number, [min, max]: readonly [number, number]) =>
  Math.min(max, Math.max(min, value))

export function usePanes() {
  /** Starts a drag; `side` says which edge moved and which way it grows. */
  function start(event: PointerEvent, side: 'sidebar' | 'panel') {
    event.preventDefault()
    const startX = event.clientX
    const startWidth = layout[side]
    layout.dragging = side

    const move = (moved: PointerEvent) => {
      // The right panel grows as the pointer moves left, hence the sign flip.
      const delta = side === 'sidebar' ? moved.clientX - startX : startX - moved.clientX
      layout[side] = clamp(startWidth + delta, limits[side])
    }
    const stop = () => {
      layout.dragging = null
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', stop)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }

    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', stop)
  }

  function reset(side: 'sidebar' | 'panel') {
    layout[side] = side === 'sidebar' ? 252 : 400
  }

  return { layout, start, reset }
}
