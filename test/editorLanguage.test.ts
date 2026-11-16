import { describe, expect, it } from 'vitest'
import { editorLanguage } from '~/composables/useCodeMirror'

describe('the editor language for a path', () => {
  it('is none for an extension that is a name Object has', async () => {
    for (const name of ['notes.constructor', 'x.toString', 'a.__proto__', 'b.hasOwnProperty']) {
      expect(await editorLanguage(name)).toBeNull()
    }
  })

  it('is still found for the ones it knows', async () => {
    expect(await editorLanguage('app/a.yml')).not.toBeNull()
  })
})
