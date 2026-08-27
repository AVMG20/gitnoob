import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useToasts } from '~/composables/useToasts'

/**
 * The stack in the corner. One module-level list, as the app has, so each test
 * clears it rather than getting a fresh one.
 */
const toasts = useToasts()

beforeEach(() => {
  vi.useFakeTimers()
  toasts.clear()
})

afterEach(() => {
  toasts.clear()
  vi.useRealTimers()
})

describe('the notices in the corner', () => {
  it('says what to do, keeping git\'s own words behind it', () => {
    toasts.fail('Checkout: error: Your local changes would be overwritten by checkout:\n\tapp.vue')
    const [only] = toasts.items.value
    expect(only!.level).toBe('error')
    expect(only!.title).toContain('Commit or stash')
    expect(only!.detail).toContain('app.vue')
  })

  it('counts a repeat rather than stacking it', () => {
    toasts.fail('Push: rejected (fetch first)')
    toasts.fail('Push: rejected (fetch first)')
    toasts.fail('Push: rejected (fetch first)')
    expect(toasts.items.value).toHaveLength(1)
    expect(toasts.items.value[0]!.count).toBe(3)
  })

  it('keeps two different failures apart', () => {
    toasts.fail('Push: rejected (fetch first)')
    toasts.fail('Commit: nothing to commit')
    expect(toasts.items.value).toHaveLength(2)
  })

  it('takes good news away again, and leaves failures standing', () => {
    toasts.info('Branch moved to 1a2b3c4')
    toasts.fail('Push: rejected (fetch first)')
    vi.advanceTimersByTime(10_000)
    expect(toasts.items.value.map((one) => one.level)).toEqual(['error'])
  })

  it('drops the oldest rather than filling the window', () => {
    for (let n = 0; n < 7; n++) toasts.fail(`Failure number ${n}`)
    expect(toasts.items.value).toHaveLength(4)
    expect(toasts.items.value[0]!.title).toBe('Failure number 3')
  })

  it('dismisses one by hand', () => {
    const first = toasts.fail('Push: rejected (fetch first)')
    toasts.fail('Commit: nothing to commit')
    toasts.dismiss(first.id)
    expect(toasts.items.value).toHaveLength(1)
    expect(toasts.items.value[0]!.title).toContain('nothing staged')
  })
})
