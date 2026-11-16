// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
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

describe('focus and the modal', () => {
  it('moves into the box, keeps Tab inside it, and goes back where it was on closing', async () => {
    const trigger = document.createElement('button')
    document.body.appendChild(trigger)
    trigger.focus()
    const wrapper = mount(AppModal, {
      attachTo: document.body,
      props: { title: 'Delete branch?' },
      slots: { footer: '<button class="cancel">Cancel</button><button class="go">Delete</button>' }
    })
    await flushPromises()
    const box = wrapper.find('.modal').element as HTMLElement
    expect(document.activeElement).toBe(box)

    // From the last button, Tab comes round to the first, not to the window.
    const go = wrapper.find('.go').element as HTMLElement
    go.focus()
    go.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }))
    expect(box.contains(document.activeElement)).toBe(true)
    expect(document.activeElement).not.toBe(go)

    wrapper.unmount()
    expect(document.activeElement).toBe(trigger)
    trigger.remove()
  })
})
