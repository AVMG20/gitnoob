import { describe, expect, it } from 'vitest'
import { escapeIgnore, extensionOf, ignoreChoices } from '~/composables/ignoreRules'

describe('ignore rules for a path', () => {
  it('escapes what git would read as a wildcard, a comment or a negation', () => {
    expect(escapeIgnore('a[1].txt')).toBe('a\\[1\\].txt')
    expect(escapeIgnore('star*.log')).toBe('star\\*.log')
    expect(escapeIgnore('#notes')).toBe('\\#notes')
    expect(escapeIgnore('!keep')).toBe('\\!keep')
    expect(escapeIgnore('trailing  ')).toBe('trailing\\ \\ ')
  })

  it('anchors a file and its folder, and offers its extension', () => {
    expect(ignoreChoices('app/etc/env.php', false).map((one) => one.pattern)).toEqual([
      '/app/etc/env.php',
      '*.php',
      '/app/etc/'
    ])
    expect(ignoreChoices('.env', false).map((one) => one.pattern)).toEqual(['/.env'])
    expect(ignoreChoices('var/cache', true).map((one) => one.pattern)).toEqual(['/var/cache/'])
  })

  it('finds no extension on a dotfile or a bare name', () => {
    expect(extensionOf('.env')).toBe('')
    expect(extensionOf('Makefile')).toBe('')
    expect(extensionOf('a/b.tar.gz')).toBe('gz')
  })
})
