import { reactive, watch } from 'vue'

const KEY = 'gitnoob.layout'

const limits = { sidebar: [180, 460], panel: [280, 720], result: [40, 1200] } as const

const DEFAULTS = { sidebar: 252, panel: 400, result: 260 }

export type Edge = keyof typeof DEFAULTS

const layout = reactive({
  sidebar: 252,
  panel: 400,
  /** How tall the resolver keeps the result pane. */
  result: 260,
  /** Which edge is being dragged, if any. */
  dragging: null as Edge | null
})

try {
  const saved = JSON.parse(localStorage.getItem(KEY) ?? '{}')
  if (typeof saved.sidebar === 'number') layout.sidebar = saved.sidebar
  if (typeof saved.panel === 'number') layout.panel = saved.panel
  if (typeof saved.result === 'number') layout.result = saved.result
} catch {
  // A missing or unreadable preference just means the defaults.
}

watch(
  () => [layout.sidebar, layout.panel, layout.result],
  () => {
    try {
      localStorage.setItem(
        KEY,
        JSON.stringify({
          sidebar: layout.sidebar,
          panel: layout.panel,
          result: layout.result
        })
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
  function start(event: PointerEvent, side: Edge) {
    event.preventDefault()
    // The result pane is the one edge that moves up and down, and it grows as
    // the pointer goes up — as the right panel grows when it goes left.
    const down = side === 'result'
    const from = down ? event.clientY : event.clientX
    const startSize = layout[side]
    layout.dragging = side

    const move = (moved: PointerEvent) => {
      const at = down ? moved.clientY : moved.clientX
      const delta = side === 'sidebar' ? at - from : from - at
      layout[side] = clamp(startSize + delta, limits[side])
    }
    const stop = () => {
      layout.dragging = null
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', stop)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }

    document.body.style.cursor = down ? 'row-resize' : 'col-resize'
    document.body.style.userSelect = 'none'
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', stop)
  }

  function reset(side: Edge) {
    layout[side] = DEFAULTS[side]
  }

  return { layout, start, reset }
}
