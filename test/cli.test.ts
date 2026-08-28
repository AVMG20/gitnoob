import { describe, expect, it } from 'vitest'
import { parseCommandLine } from '~/composables/cli'

describe('the line typed at the prompt', () => {
  it('splits on spaces and drops the git people type from habit', () => {
    expect(parseCommandLine('git log --oneline -5')).toEqual({ args: ['log', '--oneline', '-5'] })
    expect(parseCommandLine('  status  ')).toEqual({ args: ['status'] })
  })

  it('keeps a quoted message as one argument', () => {
    expect(parseCommandLine('commit -m "fix the thing"')).toEqual({
      args: ['commit', '-m', 'fix the thing']
    })
    expect(parseCommandLine('commit -m "it\'s done" --amend')).toEqual({
      args: ['commit', '-m', "it's done", '--amend']
    })
    expect(parseCommandLine('commit -m "say \\"hi\\""')).toEqual({
      args: ['commit', '-m', 'say "hi"']
    })
  })

  it('keeps an empty quoted argument', () => {
    expect(parseCommandLine('commit --allow-empty-message -m ""')).toEqual({
      args: ['commit', '--allow-empty-message', '-m', '']
    })
  })

  it('lets a backslash keep a space', () => {
    expect(parseCommandLine('add my\\ file.txt')).toEqual({ args: ['add', 'my file.txt'] })
  })

  it('refuses a quote left open, and an empty line', () => {
    expect(parseCommandLine('commit -m "unfinished')).toEqual({ error: 'Missing a closing "' })
    expect(parseCommandLine('')).toEqual({ error: 'Nothing to run' })
    expect(parseCommandLine('git')).toEqual({ error: 'Nothing to run' })
  })
})
