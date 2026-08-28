// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import AppModal from '~/components/AppModal.vue'
import ConfirmDialog from '~/components/ConfirmDialog.vue'

const show = (props: Record<string, unknown> = {}) =>
  mount(ConfirmDialog, {
    props: { title: 'Remove remote upstream?', ...props },
    global: { components: { AppModal } }
  })

/**
 * The question with two answers.
 *
 * Typing the name back belongs to what cannot be undone. Everywhere else it is
 * a hurdle, and the red frame round the whole dialog was the same mistake in
 * paint: the button already says which answer is the destructive one.
 */
describe('the confirm dialog', () => {
  it('asks with a button rather than a box to type in', () => {
    const wrapper = show({ confirm: 'Remove remote', danger: true, hint: 'Local branches stay.' })

    expect(wrapper.find('input').exists()).toBe(false)
    expect(wrapper.text()).toContain('Local branches stay.')
    expect(wrapper.find('.btn-danger').text()).toBe('Remove remote')

    wrapper.find('.btn-danger').trigger('click')
    expect(wrapper.emitted('confirm')).toHaveLength(1)
  })

  it('leaves the frame alone, whatever the answer costs', () => {
    const wrapper = show({ danger: true })
    expect(wrapper.find('.modal').classes()).not.toContain('danger')
  })

  it('closes without confirming when the cancel is taken', () => {
    const wrapper = show()
    wrapper.find('.btn-ghost').trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(1)
    expect(wrapper.emitted('confirm')).toBeUndefined()
  })
})
