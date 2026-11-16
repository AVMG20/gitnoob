import { reactive, watch } from 'vue'
import { codeFont, codeRow } from './useCode'
import { useConfig } from './useConfig'
import { keysFor, matchesShortcut, SHORTCUTS } from './useShortcuts'

/**
 * How large code is drawn: one size for every view of code.
 *
 * The editor, the diff, the search preview, a change's card, a review's diff
 * and the conflict resolver all take their font from `--code-size` and their
 * row from `--code-row`, set here on the root, so one press changes all of
 * them at once. The row grows with the font (one and a half times it), and
 * the views that place their rows themselves read `codeRow()` rather than a
 * fixed 18.
 *
 * ⌘+ ⌘− ⌘0 over code change it; anywhere else they are still the window's
 * zoom. Saved in the config with the other editor settings, and shown for a
 * moment at the foot of the code with a way back to the standard size.
 */

export const CODE_FONT_DEFAULT = 12
export const CODE_FONT_MIN = 9
export const CODE_FONT_MAX = 28

const size = codeFont

export { codeRow }

/** The code font's size in px, live. */
export function codeFontSize() {
  return size.value
}

function apply(font: number) {
  if (typeof document === 'undefined') return
  const root = document.documentElement
  root.style.setProperty('--code-size', `${font}px`)
  root.style.setProperty('--code-row', `${codeRow(font)}px`)
}

apply(size.value)

/**
 * Views that keep their place across a change of size: each is asked before
 * the change and handed back a function to run after it, once the new size
 * has been laid out.
 */
const keepers = new Set<() => (() => void) | void>()

export function onCodeFont(keep: () => (() => void) | void) {
  keepers.add(keep)
  return () => keepers.delete(keep)
}

let saveTimer: ReturnType<typeof setTimeout> | undefined

/** Sets the size everywhere, keeps the caret's line where it was, and saves it. */
export function setCodeFont(font: number, save = true) {
  const next = Math.min(CODE_FONT_MAX, Math.max(CODE_FONT_MIN, Math.round(font)))
  if (next === size.value) return
  const after = [...keepers].map((keep) => keep()).filter((one): one is () => void => !!one)
  size.value = next
  apply(next)
  if (after.length) {
    const run = () => after.forEach((one) => one())
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(run)
    else run()
  }
  if (save) {
    clearTimeout(saveTimer)
    saveTimer = setTimeout(() => void store(next), 300)
  }
}

async function store(font: number) {
  const config = useConfig()
  const settings = config.settings.value
  if (!settings || settings.editor_font_size === font) return
  await config
    .updateGlobal((current) => ({ ...current, editor_font_size: font }))
    .catch(() => undefined)
}

let following = false

/** Takes the size from the config as it loads and whenever it changes there. */
export function followConfig() {
  if (following) return
  following = true
  const config = useConfig()
  watch(
    () => config.settings.value?.editor_font_size,
    (font) => {
      if (typeof font === 'number' && font !== size.value) setCodeFont(font, false)
    },
    { immediate: true }
  )
}

// --- the notice at the foot of the code

export const fontToast = reactive({
  shown: false,
  size: CODE_FONT_DEFAULT,
  /** Where it goes: the middle of the code's bottom edge, in the window. */
  x: 0,
  bottom: 24
})

let fadeTimer: ReturnType<typeof setTimeout> | undefined
let held = false

function fadeIn(ms: number) {
  clearTimeout(fadeTimer)
  fadeTimer = setTimeout(() => {
    if (!held) fontToast.shown = false
  }, ms)
}

/** Shows the size under the code it was changed over, or refreshes the one showing. */
function announce(over: Element | null) {
  const box = over?.getBoundingClientRect?.()
  if (box && box.width) {
    fontToast.x = box.left + box.width / 2
    fontToast.bottom = Math.max(16, (window.innerHeight || box.bottom) - box.bottom + 16)
  } else {
    fontToast.x = (window.innerWidth || 0) / 2
    fontToast.bottom = 24
  }
  fontToast.size = size.value
  fontToast.shown = true
  fadeIn(2000)
}

/** The pointer on the notice keeps it up. */
export function holdFontToast(on: boolean) {
  held = on
  if (!on && fontToast.shown) fadeIn(2000)
}

/** The notice's button: the standard size, shown, and then away. */
export function resetFromToast() {
  setCodeFont(CODE_FONT_DEFAULT)
  fontToast.size = size.value
  held = false
  fadeIn(1200)
}

// --- the keys

/** What counts as code for the keys: anything marked so, and every CodeMirror view. */
const CODE_VIEWS = '[data-code-view], .cm-editor'

let hovered: Element | null = null
let clicked: Element | null = null

function onPointer(event: PointerEvent | MouseEvent) {
  hovered = (event.target as Element | null) ?? null
}

function onDown(event: PointerEvent | MouseEvent) {
  clicked = (event.target as Element | null) ?? null
}

/**
 * The code view a press is about: the one the caret is in, else, when nothing
 * in particular has the focus, the one under the pointer or the one last
 * clicked in (a diff takes no focus). Null means it is not about code, and the
 * window's zoom has it.
 */
export function codeViewFor(event: KeyboardEvent): Element | null {
  const target = event.target as Element | null
  const inside = target?.closest?.(CODE_VIEWS)
  if (inside) return inside
  // Typing in some other field, a dialog box: not code.
  const idle = !target || target === document.body || target === document.documentElement
  if (!idle) return null
  const under = hovered?.isConnected ? hovered.closest?.(CODE_VIEWS) : null
  const last = clicked?.isConnected ? clicked.closest?.(CODE_VIEWS) : null
  return under ?? last ?? null
}

const CHORDS: Record<string, { code: string[]; key: string[] }> = {
  'editor.fontBigger': { code: ['Equal', 'NumpadAdd'], key: ['=', '+'] },
  'editor.fontSmaller': { code: ['Minus', 'NumpadSubtract'], key: ['-', '_'] },
  'editor.fontReset': { code: ['Digit0', 'Numpad0'], key: ['0'] }
}

const isMac = typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac')

/**
 * Which of the three a press is: as bound, or on its standard keys by where
 * they sit, so ⌘= and ⌘+, and the keypad's + and −, all count.
 */
function which(event: KeyboardEvent): keyof typeof CHORDS | null {
  for (const id of Object.keys(CHORDS) as (keyof typeof CHORDS)[]) {
    if (matchesShortcut(event, id)) return id
    const standard = SHORTCUTS.find((one) => one.id === id)?.keys
    if (keysFor(id) !== standard) continue
    const mod = isMac ? event.metaKey : event.ctrlKey
    if (!mod || event.altKey) continue
    const { code, key } = CHORDS[id]!
    if (code.includes(event.code) || key.includes(event.key)) return id
  }
  return null
}

function onKey(event: KeyboardEvent) {
  if (event.defaultPrevented) return
  const id = which(event)
  if (!id) return
  const over = codeViewFor(event)
  if (!over) return
  event.preventDefault()
  event.stopImmediatePropagation()
  if (id === 'editor.fontBigger') setCodeFont(size.value + 1)
  else if (id === 'editor.fontSmaller') setCodeFont(size.value - 1)
  else setCodeFont(CODE_FONT_DEFAULT)
  announce(over)
}

let installed = false

/**
 * Listens for the keys, ahead of the window's zoom: in the capture phase, and
 * stopping the press once it was taken for code. The image viewer's own ⌘+
 * is not code and goes on to it.
 */
export function installCodeFontKeys() {
  if (installed || typeof window === 'undefined') return
  installed = true
  window.addEventListener('keydown', onKey, true)
  window.addEventListener('pointermove', onPointer, true)
  window.addEventListener('mousemove', onPointer, true)
  window.addEventListener('pointerdown', onDown, true)
  window.addEventListener('mousedown', onDown, true)
}

/** For tests. */
export function resetCodeFont() {
  clearTimeout(saveTimer)
  clearTimeout(fadeTimer)
  held = false
  hovered = null
  clicked = null
  size.value = CODE_FONT_DEFAULT
  apply(CODE_FONT_DEFAULT)
  fontToast.shown = false
}
