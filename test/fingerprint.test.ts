import { describe, expect, it } from 'vitest'
import { fingerprint } from '~/composables/useGit'

describe('the snapshot fingerprint', () => {
  it('is short, the same for the same text, and different for a one-character change', () => {
    const rows = JSON.stringify(Array.from({ length: 5000 }, (_, at) => ({ oid: `c${at}`, lane: at % 7 })))
    expect(fingerprint(rows)).toBe(fingerprint(rows.slice()))
    expect(fingerprint(rows).length).toBeLessThan(40)
    expect(fingerprint(rows)).not.toBe(fingerprint(rows.replace('"lane":3', '"lane":4')))
    expect(fingerprint('ab')).not.toBe(fingerprint('ba'))
  })
})
