import { describe, expect, it } from 'vitest'
import { SHORTCUTS } from '~/composables/useShortcuts'
import { TIPS, pickTips } from '~/composables/useTips'

/**
 * The tips on the home tab. A short list repeats itself, a key that is written
 * down twice goes out of date in one of the two places, and a sentence that
 * only makes sense once you have found the feature is not a tip.
 */
describe('the tips', () => {
  it('has enough of them that three are rarely the same three', () => {
    expect(TIPS.length).toBeGreaterThanOrEqual(21)
  })

  it('names each one once', () => {
    expect(new Set(TIPS.map((one) => one.id)).size).toBe(TIPS.length)
  })

  it('takes its keys from the shortcut list rather than spelling them again', () => {
    const known = new Set(SHORTCUTS.map((one) => one.keys))
    for (const tip of TIPS) {
      if (tip.keys) expect(known.has(tip.keys), `${tip.id}: ${tip.keys}`).toBe(true)
    }
  })

  it('says something, in a sentence', () => {
    for (const tip of TIPS) {
      expect(tip.text.length, tip.id).toBeGreaterThan(30)
      expect(tip.text.endsWith('.'), tip.id).toBe(true)
    }
  })

  it('picks without repeating itself', () => {
    for (let round = 0; round < 50; round += 1) {
      const three = pickTips(3)
      expect(three).toHaveLength(3)
      expect(new Set(three.map((one) => one.id)).size).toBe(3)
    }
  })

  it('asks for more than there are and gets what there is', () => {
    expect(pickTips(500)).toHaveLength(TIPS.length)
  })
})
