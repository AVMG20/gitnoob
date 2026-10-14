import { describe, expect, it } from 'vitest'
import { composeMessage, onTrunk, onlyPrefix, ticketFrom, ticketPrefix } from '~/composables/useCommitFlow'

describe('the ticket a branch is named after', () => {
  it('is found at the start, after a folder, or on its own', () => {
    expect(ticketFrom('ABC-123-foo')).toBe('ABC-123')
    expect(ticketFrom('feature/ABC-123/foo')).toBe('ABC-123')
    expect(ticketFrom('feature/ABC-123-login-page')).toBe('ABC-123')
    expect(ticketFrom('jo/PROJ2-7')).toBe('PROJ2-7')
    expect(ticketPrefix('ABC-123-foo')).toBe('ABC-123: ')
  })

  it('is not guessed from a branch that only looks a little like one', () => {
    expect(ticketFrom('fix-123-thing')).toBeNull()
    expect(ticketFrom('main')).toBeNull()
    expect(ticketFrom('release/1.2-3')).toBeNull()
    expect(ticketFrom('XABC-123foo')).toBeNull()
    expect(ticketFrom(null)).toBeNull()
  })
})

describe('a box holding only the prefix', () => {
  it('counts as empty', () => {
    expect(onlyPrefix('ABC-1: ', 'ABC-1: ')).toBe(true)
    expect(onlyPrefix('ABC-1:', 'ABC-1: ')).toBe(true)
    expect(onlyPrefix('  ', null)).toBe(true)
    expect(onlyPrefix('ABC-1: Fix it', 'ABC-1: ')).toBe(false)
    expect(onlyPrefix('Fix it', null)).toBe(false)
  })
})

describe('the trunk', () => {
  it('is whatever the repository says, by name or as its remote copy', () => {
    expect(onTrunk('main', 'main', [])).toBe(true)
    expect(onTrunk('develop', 'origin/develop', ['origin'])).toBe(true)
    expect(onTrunk('main', 'develop', ['origin'])).toBe(false)
    // `feature/x` as the trunk is not a remote called `feature`.
    expect(onTrunk('x', 'feature/x', ['origin'])).toBe(false)
    expect(onTrunk('main', null, ['origin'])).toBe(false)
  })
})

describe('the message git is given', () => {
  it('gets a blank line between subject and body', () => {
    expect(composeMessage('Subject\nbody')).toBe('Subject\n\nbody')
    expect(composeMessage('  Subject  \n\n\nbody\n')).toBe('Subject\n\nbody')
    expect(composeMessage('Only')).toBe('Only')
  })
})
