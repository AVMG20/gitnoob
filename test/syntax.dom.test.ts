// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * The stored choice is read once, as the composable loads, so that code is
 * never seen in one theme and repainted in another. That means each of these
 * has to put the stored value in place and then load the module fresh.
 */
async function load(stored?: string) {
  localStorage.clear()
  if (stored !== undefined) localStorage.setItem('gitnoob.syntax', stored)
  vi.resetModules()
  return import('../app/composables/useSyntax')
}

beforeEach(() => {
  delete document.documentElement.dataset.light
})

afterEach(() => {
  localStorage.clear()
})

describe('the syntax theme list', () => {
  it('offers every theme Shiki ships, and no colour at all beside them', async () => {
    const { SYNTAX_THEMES, PLAIN } = await load()
    expect(SYNTAX_THEMES[0]!.id).toBe(PLAIN)
    // Sixty-odd of them; the exact number is Shiki's to change.
    expect(SYNTAX_THEMES.length).toBeGreaterThan(50)
    expect(SYNTAX_THEMES.some((one) => one.id === 'catppuccin-mocha')).toBe(true)
    expect(SYNTAX_THEMES.some((one) => one.id === 'github-light')).toBe(true)
  })

  it('marks which background each was drawn for', async () => {
    const { SYNTAX_THEMES } = await load()
    const kind = (id: string) => SYNTAX_THEMES.find((one) => one.id === id)?.kind
    expect(kind('github-dark')).toBe('Dark')
    expect(kind('github-light')).toBe('Light')
  })

  it('searches on the id as well as the name', async () => {
    const { useSyntax } = await load()
    const rows = useSyntax().choices.value
    const mocha = rows.find((row) => row.value === 'catppuccin-mocha')
    expect(mocha?.hint).toBe('catppuccin-mocha')
    expect(mocha?.label).toContain('Catppuccin')
  })
})

describe('what happens to a choice made before the change', () => {
  it('carries each of the eight old schemes over to its nearest Shiki theme', async () => {
    for (const [was, now] of [
      ['onedark', 'one-dark-pro'],
      ['monokai', 'monokai'],
      ['solarized', 'solarized-dark'],
      ['github', 'github-dark'],
      ['vscode', 'dark-plus'],
      ['theme', 'dark-plus'],
      ['gitkraken', 'github-dark-dimmed']
    ]) {
      const { useSyntax } = await load(was)
      expect(useSyntax().syntax.value).toBe(now)
    }
  })

  it('keeps "no colours", which Shiki has no theme for', async () => {
    const { useSyntax, PLAIN } = await load('plain')
    expect(useSyntax().syntax.value).toBe(PLAIN)
  })

  it('keeps a Shiki theme that was already chosen', async () => {
    const { useSyntax } = await load('nord')
    expect(useSyntax().syntax.value).toBe('nord')
  })

  it('falls back rather than trusting a name it does not know', async () => {
    const { useSyntax } = await load('something-invented')
    expect(useSyntax().syntax.value).toBe('github-dark')
  })
})

describe('the default', () => {
  it('is a dark theme under a dark window', async () => {
    const { useSyntax } = await load()
    expect(useSyntax().syntax.value).toBe('github-dark')
  })

  it('is a light theme under a light one, which the root element carries', async () => {
    document.documentElement.dataset.light = ''
    const { useSyntax } = await load()
    expect(useSyntax().syntax.value).toBe('github-light')
  })
})

describe('choosing one', () => {
  it('remembers it, so the next window opens in it', async () => {
    const { useSyntax } = await load()
    useSyntax().setSyntax('dracula')
    expect(localStorage.getItem('gitnoob.syntax')).toBe('dracula')
    expect(useSyntax().syntax.value).toBe('dracula')
  })

  it('ignores a name that is not one of the themes on offer', async () => {
    const { useSyntax } = await load()
    const syntax = useSyntax()
    syntax.setSyntax('not-a-theme')
    expect(syntax.syntax.value).toBe('github-dark')
  })
})

/**
 * The scheme that used to ship was "Match the theme", which swapped its colours
 * the moment a light app theme went on. A Shiki theme knows nothing about the
 * window, so the default has to keep following it.
 */
describe('the default following the window', () => {
  /** `useTheme` puts this on the root element for every light theme. */
  const goLight = () => {
    document.documentElement.dataset.light = ''
  }
  const goDark = () => {
    delete document.documentElement.dataset.light
  }
  /** The observer fires on a microtask, so let one go by. */
  const settle = () => new Promise((done) => setTimeout(done, 0))

  it('follows the window to light and back while nothing has been picked', async () => {
    const { useSyntax } = await load()
    const syntax = useSyntax()
    expect(syntax.syntax.value).toBe('github-dark')

    goLight()
    await settle()
    expect(syntax.syntax.value).toBe('github-light')

    goDark()
    await settle()
    expect(syntax.syntax.value).toBe('github-dark')
  })

  it('stops following once a theme has actually been chosen', async () => {
    const { useSyntax } = await load()
    const syntax = useSyntax()
    syntax.setSyntax('dracula')

    goLight()
    await settle()
    expect(syntax.syntax.value).toBe('dracula')
  })

  it('never follows a window whose theme was chosen in an earlier session', async () => {
    const { useSyntax } = await load('monokai')
    const syntax = useSyntax()

    goLight()
    await settle()
    expect(syntax.syntax.value).toBe('monokai')
  })
})
