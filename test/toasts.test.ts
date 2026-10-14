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

  it('takes good news away first, and a failure a while later', () => {
    toasts.info('Branch moved to 1a2b3c4')
    toasts.fail('Push: rejected (fetch first)')
    vi.advanceTimersByTime(10_000)
    // Long enough for the good news, not for something to act on.
    expect(toasts.items.value.map((one) => one.level)).toEqual(['error'])

    vi.advanceTimersByTime(10_000)
    expect(toasts.items.value).toHaveLength(0)
  })

  it('stops the clock while the stack is being read', () => {
    toasts.fail('Push: rejected (fetch first)')
    toasts.hold(true)
    vi.advanceTimersByTime(60_000)
    expect(toasts.items.value).toHaveLength(1)

    // Letting go starts the wait again rather than finishing what was left.
    toasts.hold(false)
    vi.advanceTimersByTime(14_000)
    expect(toasts.items.value).toHaveLength(1)
    vi.advanceTimersByTime(2_000)
    expect(toasts.items.value).toHaveLength(0)
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
    expect(toasts.items.value[0]!.title).toContain('Nothing staged')
  })
})

describe('an offer made about one repository', () => {
  it('runs its buttons only while that repository is in front', async () => {
    const { aimAt } = await import('~/composables/useInvoke')
    aimAt('/work/one')
    const ran: string[] = []
    const made = toasts.offer('one has 2 commits you do not have yet', [
      { label: 'Pull', run: () => { ran.push('pull') } }
    ])
    aimAt('/work/two')
    await toasts.act(made, made.actions![0]!)
    expect(ran).toEqual([])
    expect(toasts.items.value.map((one) => one.title)).toEqual([
      'That was about one. Switch back to it to do this; nothing was changed.'
    ])

    aimAt('/work/one')
    const again = toasts.offer('one has 2 commits you do not have yet', [
      { label: 'Pull', run: () => { ran.push('pull') } }
    ])
    await toasts.act(again.id, again.actions![0]!)
    expect(ran).toEqual(['pull'])
    aimAt(null)
  })
})
