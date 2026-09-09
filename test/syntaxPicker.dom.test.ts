// @vitest-environment happy-dom
import { mount } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'
import { nextTick } from 'vue'
import SearchSelect from '~/components/SearchSelect.vue'
import { useSyntax } from '~/composables/useSyntax'

/**
 * The theme picker in Settings.
 *
 * There were eight schemes and they fitted in a grid of buttons; there are
 * sixty-six themes and they do not, so the list is the same searchable one the
 * review dialog picks a branch with. What is worth a test is that the rows it
 * is given are usable: that a theme can be found by the name somebody knows it
 * by, that dark and light are told apart, and that picking one says which.
 */
function openPicker(chosen: string) {
  const { choices } = useSyntax()
  return mount(SearchSelect, {
    props: { modelValue: chosen, options: choices.value, placeholder: 'Choose a theme…' },
    attachTo: document.body
  })
}

// The list is teleported to the body, so it is not under the wrapper's root.
const rows = () =>
  [...document.body.querySelectorAll('.row .label')].map((node) => node.textContent?.trim() ?? '')

async function search(wrapper: ReturnType<typeof mount>, query: string) {
  await wrapper.find('.face').trigger('click')
  await nextTick()
  const field = document.body.querySelector('input[type="search"]') as HTMLInputElement
  field.value = query
  field.dispatchEvent(new Event('input'))
  await nextTick()
}

afterEach(() => {
  document.body.innerHTML = ''
})

describe('the syntax theme picker', () => {
  it('shows the chosen theme on the face, with which background it is for', () => {
    const wrapper = openPicker('github-light')
    expect(wrapper.find('.face .text').text()).toBe('GitHub Light')
    expect(wrapper.find('.face .note').text()).toBe('Light')
    wrapper.unmount()
  })

  it('finds a theme by the name it is known by', async () => {
    const wrapper = openPicker('github-dark')
    await search(wrapper, 'dracula')
    expect(rows().some((row) => row.includes('Dracula'))).toBe(true)
    wrapper.unmount()
  })

  it('finds a theme by its id, which is not always its name', async () => {
    const wrapper = openPicker('github-dark')
    // "mocha" is in `catppuccin-mocha` but the name reads "Catppuccin Mocha";
    // searching the id is what makes a half-remembered name work.
    await search(wrapper, 'mocha')
    expect(rows().some((row) => row.includes('Catppuccin'))).toBe(true)
    wrapper.unmount()
  })

  it('takes the words in any order, so "light github" finds it too', async () => {
    const wrapper = openPicker('github-dark')
    await search(wrapper, 'light github')
    expect(rows().some((row) => row.includes('GitHub Light'))).toBe(true)
    wrapper.unmount()
  })

  it('offers no colour at all as one of the rows', async () => {
    const wrapper = openPicker('github-dark')
    await search(wrapper, 'no colours')
    expect(rows()).toContain('No colours')
    wrapper.unmount()
  })

  it('says so rather than showing an empty list', async () => {
    const wrapper = openPicker('github-dark')
    await search(wrapper, 'not a theme anybody wrote')
    expect(rows()).toHaveLength(0)
    expect(document.body.querySelector('.none')).not.toBeNull()
    wrapper.unmount()
  })

  it('hands back the theme id when one is picked', async () => {
    const wrapper = openPicker('github-dark')
    await search(wrapper, 'nord')
    ;(document.body.querySelector('.row') as HTMLElement).click()
    await nextTick()
    expect(wrapper.emitted('update:modelValue')?.[0]?.[0]).toBe('nord')
    wrapper.unmount()
  })

  it('ticks the one that is on', async () => {
    const wrapper = openPicker('monokai')
    await search(wrapper, 'monokai')
    const on = document.body.querySelectorAll('.row.on')
    expect(on).toHaveLength(1)
    expect(on[0]!.querySelector('.label')?.textContent).toContain('Monokai')
    wrapper.unmount()
  })
})
