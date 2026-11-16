// @vitest-environment happy-dom
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { EditorView } from '@codemirror/view'
import { EditorState } from '@codemirror/state'
import { basics } from '~/composables/useCodeMirror'

/**
 * The editor's selection: strong enough to see on every theme, and not
 * washed out by the active line's tint.
 */

describe('the selection in the editor', () => {
  it('marks the editor while text is selected, so the active line steps aside', () => {
    const parent = document.createElement('div')
    document.body.appendChild(parent)
    const view = new EditorView({
      state: EditorState.create({ doc: 'one\ntwo\nthree\n', extensions: [basics()] }),
      parent
    })
    expect(view.dom.classList.contains('cm-hasSelection')).toBe(false)
    view.dispatch({ selection: { anchor: 4, head: 7 } })
    expect(view.dom.classList.contains('cm-hasSelection')).toBe(true)
    view.dispatch({ selection: { anchor: 5 } })
    expect(view.dom.classList.contains('cm-hasSelection')).toBe(false)
    view.destroy()
    parent.remove()
  })

  it('stands out from the ground of every theme, and keeps the text readable on it', () => {
    const css = readFileSync(resolve(__dirname, '../app/assets/css/themes.css'), 'utf8')
    const themes = [...css.matchAll(/\[data-theme='([^']+)'\]\s*\{([^}]*)\}/g)]
    expect(themes.length).toBeGreaterThan(10)
    const rgb = (hex: string) => [0, 2, 4].map((at) => parseInt(hex.slice(1 + at, 3 + at), 16) / 255)
    const linear = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
    const luminance = (c: number[]) => 0.2126 * linear(c[0]!) + 0.7152 * linear(c[1]!) + 0.0722 * linear(c[2]!)
    const contrast = (a: number[], b: number[]) => {
      const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x)
      return (hi! + 0.05) / (lo! + 0.05)
    }
    const mix = (a: number[], b: number[], amount: number) => a.map((c, at) => c * amount + b[at]! * (1 - amount))
    for (const [, name, body] of themes) {
      const value = (key: string) => rgb(new RegExp(`${key}:\\s*(#[0-9a-f]{6})`, 'i').exec(body!)![1]!)
      const bg = value('--bg')
      // The same mixes as `chrome` in useCodeMirror.ts.
      const focused = mix(value('--primary'), bg, 0.35)
      const blurred = mix(value('--primary'), bg, 0.2)
      expect(contrast(focused, bg), name).toBeGreaterThanOrEqual(1.5)
      expect(contrast(blurred, bg), name).toBeGreaterThan(contrast(value('--primary-bg'), bg))
      expect(contrast(value('--fg'), focused), name).toBeGreaterThanOrEqual(4.5)
    }
  })
})
