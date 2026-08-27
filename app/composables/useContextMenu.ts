import { reactive } from 'vue'

export interface MenuItem {
  label: string
  /** A lucide component, passed straight to the renderer. */
  icon?: unknown
  /**
   * What the item does. Whatever it hands back is thrown away — the menu closes
   * either way — so the return type is `unknown` rather than `void`.
   *
   * It was `() => void | Promise<void>`, and that is not the same thing:
   * TypeScript lets a function returning a value stand in for one returning
   * `void`, but not for a *union* containing `void`. Since almost every action
   * here is an existing operation that reports something back — a checkout
   * returning its outcome, a copy returning what it copied — the old signature
   * rejected most of the menu, which is where the bulk of this project's type
   * errors came from.
   */
  action?: () => unknown
  /** Renders a divider; label is ignored. */
  separator?: boolean
  /** Renders in red, for anything that destroys work. */
  danger?: boolean
  disabled?: boolean
  hint?: string
  /**
   * A nested menu. An item with children opens them on hover rather than doing
   * anything itself, which is how a family of related choices — the three reset
   * modes, say — can be offered without spending three rows on them or making
   * the caller invent a dialog.
   */
  children?: MenuItem[]
}

const state = reactive({
  open: false,
  x: 0,
  y: 0,
  title: '' as string,
  items: [] as MenuItem[]
})

export function useContextMenu() {
  return {
    state,
    /** Opens the menu at the pointer, kept inside the window. */
    show(event: MouseEvent, items: MenuItem[], title = '') {
      event.preventDefault()
      event.stopPropagation()
      state.items = items
      state.title = title
      state.x = event.clientX
      state.y = event.clientY
      state.open = true
    },
    close() {
      state.open = false
      state.items = []
    }
  }
}
