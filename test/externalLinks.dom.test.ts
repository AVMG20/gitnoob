// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { installExternalLinks } from '~/composables/externalLinks'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => null) }))

const asked = vi.mocked(invoke)
let uninstall: () => void = () => {}

beforeEach(() => {
  asked.mockClear()
  uninstall = installExternalLinks()
})

afterEach(() => {
  uninstall()
  document.body.innerHTML = ''
})

function click(html: string) {
  document.body.innerHTML = html
  const inner = document.body.querySelector('[data-click]') as HTMLElement
  const event = new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 })
  inner.dispatchEvent(event)
  return event
}

describe('web links', () => {
  it('go to the browser, from the link or from something inside it', () => {
    const event = click('<a href="https://example.com/x" target="_blank"><b data-click>x</b></a>')
    expect(event.defaultPrevented).toBe(true)
    expect(asked).toHaveBeenCalledWith('open_external', { url: 'https://example.com/x' })
  })

  it('leave other links and links already handled alone', () => {
    click('<a href="#" data-click>here</a>')
    document.body.innerHTML = '<a href="https://example.com" data-click>x</a>'
    const link = document.body.querySelector('a')!
    link.addEventListener('click', (event) => event.preventDefault())
    link.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 }))
    expect(asked).not.toHaveBeenCalled()
  })
})
