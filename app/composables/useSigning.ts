import { ShieldAlert, ShieldCheck, ShieldQuestionMark, ShieldX } from 'lucide-vue-next'
import type { SignatureVerdict } from './useGit'

/**
 * How a signature reads: one glyph, one colour, one sentence.
 *
 * Said in one place because three components say it — the mark in the graph,
 * the line in the commit details, and the hint under the commit box — and a
 * good signature that is green in one of them and amber in another is worse
 * than no mark at all.
 */
export interface SignatureLook {
  icon: unknown
  /** The class the colour hangs off: `good`, `warn` or `bad`. */
  tone: 'good' | 'warn' | 'bad'
  /** The short of it, for a tooltip or a heading. */
  title: string
  /** What it means, in a clause that follows the title after a dash. */
  because: string
}

const LOOKS: Record<Exclude<SignatureVerdict, 'none'>, SignatureLook> = {
  good: {
    icon: ShieldCheck,
    tone: 'good',
    title: 'Signed',
    because: 'a key this machine trusts'
  },
  untrusted: {
    icon: ShieldAlert,
    tone: 'warn',
    title: 'Signed by an unvouched-for key',
    // The four codes folded into this one — untrusted, expired, expired key,
    // revoked key — all leave the reader in the same place: the signature is
    // real and it still does not establish who made it.
    because: 'nothing here says whose key it is'
  },
  bad: {
    icon: ShieldX,
    tone: 'bad',
    title: 'Bad signature',
    because: 'this commit is not what was signed'
  },
  unchecked: {
    icon: ShieldQuestionMark,
    tone: 'warn',
    title: 'Signed, but not checked',
    because: 'the key it was made with is not on this machine'
  }
}

/** The look for a verdict, or null for a commit nobody signed. */
export function signatureLook(verdict: SignatureVerdict | null | undefined): SignatureLook | null {
  if (!verdict || verdict === 'none') return null
  return LOOKS[verdict]
}

/** The whole sentence, for a tooltip: who, and what that is worth. */
export function signatureTitle(
  verdict: SignatureVerdict | null | undefined,
  signer?: string | null
) {
  const look = signatureLook(verdict)
  if (!look) return ''
  const who = signer ? `${look.title} by ${signer}` : look.title
  return `${who} — ${look.because}`
}
