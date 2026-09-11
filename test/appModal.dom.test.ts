// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import AppModal from '~/components/AppModal.vue'

const show = (keep = false) =>
  mount(AppModal, {
    props: { title: 'A box', keep },
    slots: { default: '<textarea class="text" />' }
  })

/**
 * The scrim closes a dialog, but only on a click that was a click: one that
 * began and ended outside the box. A drag that starts in a field and lets go
 * past the edge is not that, and a dialog full of typed work never closes
 * on the scrim at all.
 */
describe('the modal scrim', () => {
  it('closes on a click that starts and ends on the scrim', async () => {
    const wrapper = show()
    const scrim = wrapper.find('.scrim')
    await scrim.trigger('pointerdown')
    await scrim.trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(1)
  })

  it('stays open when a press inside the box is released on the scrim', async () => {
    const wrapper = show()
    await wrapper.find('.text').trigger('pointerdown')
    await wrapper.find('.scrim').trigger('click')
    expect(wrapper.emitted('close')).toBeUndefined()
  })

  it('never closes a kept dialog from the scrim', async () => {
    const wrapper = show(true)
    const scrim = wrapper.find('.scrim')
    await scrim.trigger('pointerdown')
    await scrim.trigger('click')
    expect(wrapper.emitted('close')).toBeUndefined()
  })

  it('still closes a kept dialog from the ✕ and from Escape', async () => {
    const wrapper = show(true)
    await wrapper.find('.head .btn').trigger('click')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(wrapper.emitted('close')).toHaveLength(2)
    wrapper.unmount()
  })
})
