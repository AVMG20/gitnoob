// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import Toasts from '~/components/Toasts.vue'
import { useToasts } from '~/composables/useToasts'

const toasts = useToasts()

beforeEach(() => toasts.clear())
afterEach(() => toasts.clear())

describe('the toast stack', () => {
  it('shows nothing at all while nothing has gone wrong', () => {
    const wrapper = mount(Toasts)
    expect(wrapper.find('.toasts').exists()).toBe(false)
  })

  it('opens git\'s own words under the sentence, and only on asking', async () => {
    toasts.fail('Checkout: error: would be overwritten by checkout:\n\tapp/app.vue')
    const wrapper = mount(Toasts)
    expect(wrapper.find('.title').text()).toContain('Commit or stash')
    expect(wrapper.find('.detail').exists()).toBe(false)

    await wrapper.find('.more .link').trigger('click')
    expect(wrapper.find('.detail').text()).toContain('app/app.vue')
  })

  it('closes one from its own corner', async () => {
    toasts.fail('Push: rejected (fetch first)')
    const wrapper = mount(Toasts)
    await wrapper.find('.close').trigger('click')
    expect(toasts.items.value).toHaveLength(0)
    expect(wrapper.find('.toast').exists()).toBe(false)
  })

  it('offers to clear the lot once there is more than one', async () => {
    toasts.fail('Push: rejected (fetch first)')
    const wrapper = mount(Toasts)
    expect(wrapper.find('.clear').exists()).toBe(false)

    toasts.fail('Commit: nothing to commit')
    await wrapper.vm.$nextTick()
    await wrapper.find('.clear').trigger('click')
    expect(toasts.items.value).toHaveLength(0)
  })
})
