import { describe, expect, it } from 'vitest'
import { signatureLook, signatureTitle } from '~/composables/useSigning'

describe('how a signature reads', () => {
  it('says nothing at all about a commit nobody signed', () => {
    expect(signatureLook('none')).toBeNull()
    expect(signatureLook(null)).toBeNull()
    expect(signatureLook(undefined)).toBeNull()
    expect(signatureTitle('none')).toBe('')
  })

  it('is green only for a key the machine trusts', () => {
    expect(signatureLook('good')?.tone).toBe('good')
    expect(signatureLook('untrusted')?.tone).toBe('warn')
    expect(signatureLook('unchecked')?.tone).toBe('warn')
    expect(signatureLook('bad')?.tone).toBe('bad')
  })

  it('names the signer when git named one', () => {
    expect(signatureTitle('good', 'Ramon Robben')).toBe(
      'Signed by Ramon Robben — a key this machine trusts'
    )
    expect(signatureTitle('good')).toBe('Signed — a key this machine trusts')
  })

  it('does not call an unvouched-for signature good', () => {
    const said = signatureTitle('untrusted', 'A Contributor')
    expect(said).toContain('unvouched-for')
    expect(said).not.toMatch(/^Signed by A Contributor —/)
  })
})
