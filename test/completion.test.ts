import { describe, expect, it } from 'vitest'
import {
  commonPrefix,
  completionsFor,
  replaceWord,
  type CompletionSource
} from '~/composables/useCompletion'

const source: CompletionSource = {
  branches: ['main', 'feature/export', 'feature/exit-code'],
  remotes: ['origin/main', 'origin/feature/export'],
  tags: ['v0.4.9'],
  files: ['app/app.vue', 'app/components/SideBar.vue']
}

const matches = (line: string) => completionsFor(line, source).matches

describe('what Tab offers at the prompt', () => {
  it('completes the subcommand while the first word is being typed', () => {
    expect(matches('che')).toEqual(['checkout', 'cherry-pick'])
    expect(matches('git che')).toEqual(['checkout', 'cherry-pick'])
    expect(matches('stat')).toEqual(['status'])
  })

  it('offers branches, remotes and tags where a ref belongs', () => {
    expect(matches('checkout feature/ex')).toEqual(['feature/exit-code', 'feature/export'])
    expect(matches('merge origin/')).toEqual(['origin/feature/export', 'origin/main'])
    expect(matches('show v0')).toEqual(['v0.4.9'])
  })

  it('offers changed files where a path belongs', () => {
    expect(matches('add app/')).toEqual(['app/app.vue', 'app/components/SideBar.vue'])
  })

  it('has its own second word for the commands that carry one', () => {
    expect(matches('submodule ')).toContain('update')
    expect(matches('stash p')).toEqual(['pop', 'push'])
    expect(matches('remote se')).toEqual(['set-url'])
  })

  it('gives a checkout both refs and paths, since it takes either', () => {
    const found = matches('checkout ')
    expect(found).toContain('main')
    expect(found).toContain('app/app.vue')
  })

  it('offers nothing for a flag rather than guessing at one', () => {
    expect(matches('commit --am')).toEqual([])
  })

  it('does not offer a word that is already finished', () => {
    expect(matches('status')).toEqual([])
    expect(matches('checkout main')).toEqual([])
  })

  it('lists everything once the verb is one it has no rule for', () => {
    const found = matches('bisect start ma')
    expect(found).toEqual(['main'])
  })
})

describe('filling the word in', () => {
  it('fills as far as the matches agree', () => {
    expect(commonPrefix(['feature/exit-code', 'feature/export'])).toBe('feature/ex')
    expect(commonPrefix(['main'])).toBe('main')
    expect(commonPrefix([])).toBe('')
    expect(commonPrefix(['main', 'origin/main'])).toBe('')
  })

  it('replaces only the word being typed', () => {
    expect(replaceWord('checkout fea', 'feature/export')).toBe('checkout feature/export')
    expect(replaceWord('checkout ', 'main')).toBe('checkout main')
    expect(replaceWord('', 'status')).toBe('status')
  })
})
