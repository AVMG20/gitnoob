// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { defineComponent, h } from 'vue'
import FileEditor from '~/components/FileEditor.vue'
import ImageView from '~/components/ImageView.vue'
import CodeFontToast from '~/components/CodeFontToast.vue'
import { useGit } from '~/composables/useGit'
import { useConfig } from '~/composables/useConfig'
import { useShortcuts } from '~/composables/useShortcuts'
import { resetEdits } from '~/composables/useFileEdit'
import { aimAt } from '~/composables/useInvoke'
import { openTab, resetTabs, switchTabs, tabId } from '~/composables/useTabs'
import { codeRow } from '~/composables/useCode'
import { CODE_FONT_DEFAULT, CODE_FONT_MAX, installCodeFontKeys, resetCodeFont } from '~/composables/useCodeFont'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ ask: vi.fn(async () => true) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }))

/**
 * ⌘+ ⌘− ⌘0 over code size the code, everywhere code is shown, and save it;
 * anywhere else they are still the window's zoom, and the image viewer's.
 */

const asked = vi.mocked(invoke)
const git = useGit()
const isMac = navigator.userAgent.includes('Mac')
const mod = { metaKey: isMac, ctrlKey: !isMac }
let saved: Record<string, unknown>[] = []
let wrappers: ReturnType<typeof mount>[] = []
const zoomed = vi.fn()

beforeEach(async () => {
  resetTabs()
  resetEdits()
  resetCodeFont()
  switchTabs('/repos/shop')
  aimAt('/repos/shop')
  git.store.repo = { path: '/repos/shop', name: 'shop', head: 'main' } as never
  saved = []
  zoomed.mockReset()
  asked.mockReset()
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'read_file') {
      return { text: '<?php\nreturn 1;\n', hash: 'h', eol: 'lf', bom: false, readonly: null, missing: false }
    }
    if (cmd === 'config_get') return { version: 1, active_profile: null, global: { editor_font_size: 12 }, profiles: [] }
    if (cmd === 'config_set_global') {
      saved.push(args!.global as Record<string, unknown>)
      return { version: 1, active_profile: null, global: args!.global, profiles: [] }
    }
    if (cmd === 'image_read') {
      return { mime: 'image/png', size: 10, too_large: false, lfs: null, missing: false, data: btoa('img') }
    }
    if (cmd === 'unused_in' || cmd === 'usage_counts') return []
    return null
  })
  URL.createObjectURL = vi.fn(() => 'blob:test/1')
  URL.revokeObjectURL = vi.fn()
  await useConfig().load()
  installCodeFontKeys()
})

afterEach(() => {
  for (const one of wrappers) one.unmount()
  wrappers = []
  git.store.repo = null
})

const wait = (ms: number) => new Promise((done) => setTimeout(done, ms))
const sizeVar = () => document.documentElement.style.getPropertyValue('--code-size')

/** The window's zoom keys, bound the way app.vue binds them. */
const Window = defineComponent({
  setup() {
    useShortcuts({ 'zoom.in': zoomed, 'zoom.out': zoomed, 'zoom.reset': zoomed })
    return () => h('div', { class: 'elsewhere' }, 'sidebar')
  }
})

async function showEditor() {
  openTab({ path: 'app/Tax.php', mode: 'file' })
  const w = mount(FileEditor, {
    attachTo: document.body,
    props: { tabId: tabId('app/Tax.php'), path: 'app/Tax.php', hunks: [], runs: null, blame: false, find: null, fallback: null }
  })
  wrappers.push(w)
  await flushPromises()
  return w
}

function press(target: Element, key: string, code: string) {
  const event = new KeyboardEvent('keydown', { key, code, bubbles: true, cancelable: true, ...mod })
  target.dispatchEvent(event)
  return event
}

describe('the code font', () => {
  it('grows by a pixel on ⌘+ in the editor, everywhere code is drawn, and is saved', async () => {
    const w = await showEditor()
    const content = w.find('.cm-content').element
    press(content, '=', 'Equal')
    expect(sizeVar()).toBe('13px')
    expect(document.documentElement.style.getPropertyValue('--code-row')).toBe(`${codeRow(13)}px`)
    // The keypad's + and ⇧⌘= (⌘+) count too.
    press(content, '+', 'NumpadAdd')
    expect(sizeVar()).toBe('14px')
    press(content, '-', 'Minus')
    expect(sizeVar()).toBe('13px')
    await wait(350)
    await flushPromises()
    expect(saved.at(-1)).toMatchObject({ editor_font_size: 13 })
    // ⌘0 over code is the code's standard size, not the window's.
    press(content, '0', 'Digit0')
    expect(sizeVar()).toBe(`${CODE_FONT_DEFAULT}px`)
    expect(zoomed).not.toHaveBeenCalled()
  })

  it('leaves ⌘+ to the window’s zoom outside code', async () => {
    const other = mount(Window, { attachTo: document.body })
    wrappers.push(other)
    await showEditor()
    const elsewhere = other.find('.elsewhere').element
    elsewhere.dispatchEvent(new MouseEvent('mousemove', { bubbles: true }))
    const event = press(document.body, '=', 'Equal')
    expect(zoomed).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
    expect(sizeVar()).toBe(`${CODE_FONT_DEFAULT}px`)
  })

  it('stops at the largest size', async () => {
    const w = await showEditor()
    const content = w.find('.cm-content').element
    for (let i = 0; i < 40; i++) press(content, '=', 'Equal')
    expect(sizeVar()).toBe(`${CODE_FONT_MAX}px`)
  })

  it('says the size in one notice that changes in place, and Reset takes it back', async () => {
    const toast = mount(CodeFontToast, { attachTo: document.body })
    wrappers.push(toast)
    const w = await showEditor()
    const content = w.find('.cm-content').element
    press(content, '=', 'Equal')
    press(content, '=', 'Equal')
    await flushPromises()
    expect(document.querySelectorAll('.font-toast')).toHaveLength(1)
    expect(toast.find('.font-toast').text()).toContain('Font size 14 px')
    expect(toast.find('.reset').text()).toBe(`Reset to ${CODE_FONT_DEFAULT} px`)
    await toast.find('.reset').trigger('click')
    expect(sizeVar()).toBe(`${CODE_FONT_DEFAULT}px`)
    expect(toast.find('.font-toast').text()).toContain(`Font size ${CODE_FONT_DEFAULT} px`)
    expect(toast.find('.reset').exists()).toBe(false)
  })

  it('leaves the image viewer its own ⌘+', async () => {
    const image = mount(ImageView, {
      attachTo: document.body,
      props: { path: 'img/logo.png', side: { source: 'work' } }
    })
    wrappers.push(image)
    await flushPromises()
    const before = image.find('.pct').text()
    image.find('.image-view').element.dispatchEvent(new MouseEvent('mousemove', { bubbles: true }))
    press(document.body, '=', 'Equal')
    await flushPromises()
    expect(image.find('.pct').text()).not.toBe(before)
    expect(sizeVar()).toBe(`${CODE_FONT_DEFAULT}px`)
  })
})
