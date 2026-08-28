// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import AppModal from '~/components/AppModal.vue'
import SquashDialog from '~/components/SquashDialog.vue'
import { useGit, type Folded, type SquashPreview } from '~/composables/useGit'
import { useAi } from '~/composables/useAi'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
const git = useGit()
const ai = useAi()
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const folded = (over: Partial<Folded> = {}): Folded => ({
  oid: 'a'.repeat(40),
  short: 'aaaaaaa',
  summary: 'Add the parser',
  message: 'Add the parser',
  author: 'Arno Visker',
  time: Math.floor(Date.now() / 1000) - 3600,
  pushed: false,
  ...over
})

const preview = (over: Partial<SquashPreview> = {}): SquashPreview => ({
  commits: [
    folded(),
    folded({ oid: 'b'.repeat(40), short: 'bbbbbbb', summary: 'wip: fix it', message: 'wip: fix it' })
  ],
  message: 'Add the parser\n\nwip: fix it',
  onto: '0000001',
  above: 0,
  branch: 'main',
  refusal: null,
  ...over
})

/** Answers `squash_preview` with `given`, and every other call with null. */
function answering(given: SquashPreview | null, squash: unknown = 'Squashed 2 commits into one') {
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'squash_preview') return given
    if (cmd === 'squash') return squash
    return null
  })
}

const oids = ['a'.repeat(40), 'b'.repeat(40)]

const show = (list = oids) =>
  mount(SquashDialog, {
    props: { oids: list },
    global: { components: { AppModal } }
  })

beforeEach(() => {
  calls = []
  asked.mockReset()
  answering(preview())
  git.store.busyLabel = null
  git.store.log = []
  ai.store.busy = null
  ai.store.status = { configured: true, model: 'a/model', default_commit_prompt: '' }
})

describe('the squash dialog', () => {
  it('asks the backend what the fold would do, for the commits it was given', async () => {
    show()
    await flushPromises()
    expect(calls[0]).toEqual({ cmd: 'squash_preview', args: { oids } })
  })

  it('lists every commit that becomes one', async () => {
    const wrapper = show()
    await flushPromises()
    const rows = wrapper.findAll('.commits li')
    expect(rows).toHaveLength(2)
    expect(rows[0]!.text()).toContain('Add the parser')
    expect(rows[1]!.text()).toContain('wip: fix it')
  })

  it('starts the box off with the messages joined, oldest first', async () => {
    const wrapper = show()
    await flushPromises()
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe(
      'Add the parser\n\nwip: fix it'
    )
  })

  it('sends what was typed rather than what it offered', async () => {
    const wrapper = show()
    await flushPromises()
    await wrapper.find('textarea').setValue('feat: a parser that works')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(calls.find((c) => c.cmd === 'squash')?.args).toEqual({
      oids,
      message: 'feat: a parser that works'
    })
  })

  it('closes and says the marks can go once the fold lands', async () => {
    const wrapper = show()
    await flushPromises()
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(wrapper.emitted('done')).toHaveLength(1)
    expect(wrapper.emitted('close')).toHaveLength(1)
    // What git said reaches the log rather than being swallowed.
    expect(git.store.log.some((line) => line.text.includes('Squashed 2'))).toBe(true)
  })

  it('stays open when the fold failed, so the message is not lost', async () => {
    answering(preview(), null)
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} })
      if (cmd === 'squash_preview') return preview()
      if (cmd === 'squash') throw new Error('it went wrong')
      return null
    })
    const wrapper = show()
    await flushPromises()
    await wrapper.find('textarea').setValue('feat: mine')
    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()

    expect(wrapper.emitted('close')).toBeUndefined()
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe('feat: mine')
  })

  it('will not fold with an empty message', async () => {
    const wrapper = show()
    await flushPromises()
    await wrapper.find('textarea').setValue('   \n  ')
    expect(wrapper.find('.btn-primary').attributes('disabled')).toBeDefined()

    await wrapper.find('.btn-primary').trigger('click')
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'squash')).toBe(false)
  })

  it('shows the refusal instead of a message box, and offers no button', async () => {
    answering(
      preview({
        refusal: 'Those commits are not next to each other: 2 other commits sit between them.'
      })
    )
    const wrapper = show()
    await flushPromises()

    expect(wrapper.find('.note.bad').text()).toContain('not next to each other')
    expect(wrapper.find('textarea').exists()).toBe(false)
    expect(wrapper.find('.btn-primary').attributes('disabled')).toBeDefined()
    // The commits are still named, so it is clear which selection was refused.
    expect(wrapper.findAll('.commits li')).toHaveLength(2)
  })

  it('warns that a published commit means a force push, and says on which branch', async () => {
    answering(preview({ commits: [folded({ pushed: true }), folded({ short: 'bbbbbbb' })] }))
    const wrapper = show()
    await flushPromises()

    const note = wrapper.find('.note')
    expect(note.text()).toContain('force push')
    expect(note.text()).toContain('main')
    expect(wrapper.findAll('.tag').map((tag) => tag.text())).toEqual(['pushed'])
  })

  it('says nothing about force pushing when none of them left this machine', async () => {
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.note').exists()).toBe(false)
  })

  it('says what happens to the commits above the fold', async () => {
    answering(preview({ above: 3 }))
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.small').text()).toContain('The 3 commits above are replayed')
    expect(wrapper.find('.small').text()).toContain('they get new hashes too')
    expect(wrapper.find('.small').text()).toContain('Undo puts every one of them back')
  })

  it('says it in the singular when one commit sits above the fold', async () => {
    answering(preview({ above: 1 }))
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.small').text()).toContain('The commit above is replayed')
    expect(wrapper.find('.small').text()).not.toContain('The 1 commit')
  })

  it('says nothing about replaying when the fold is at the tip', async () => {
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.small').text()).not.toContain('replayed')
  })

  it('says where the fold lands, and says so differently at the root', async () => {
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.small').text()).toContain('They fold onto 0000001')

    answering(preview({ onto: null }))
    const atRoot = show()
    await flushPromises()
    expect(atRoot.find('.small').text()).toContain('start of the history')
  })

  it('folds on cmd-enter without reaching for the button', async () => {
    const wrapper = show()
    await flushPromises()
    await wrapper.find('textarea').trigger('keydown', { key: 'Enter', metaKey: true })
    await flushPromises()
    expect(calls.some((c) => c.cmd === 'squash')).toBe(true)
  })

  it('does nothing at all while the preview is still being read', async () => {
    answering(null)
    const wrapper = show()
    expect(wrapper.text()).toContain('Reading those commits')
    expect(wrapper.find('.btn-primary').attributes('disabled')).toBeDefined()
  })

  it('says so when the preview cannot be read, rather than reading for ever', async () => {
    asked.mockImplementation(async (cmd: string) => {
      if (cmd === 'squash_preview') throw new Error('fatal: bad object')
      return null
    })
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.note.bad').text()).toContain('Could not read those commits')
    expect(wrapper.text()).not.toContain('Reading those commits')
    expect(wrapper.find('.btn-primary').attributes('disabled')).toBeDefined()
    // Cancel still works; there is nothing else the dialog can do.
    await wrapper.find('.btn-ghost').trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(1)
  })
})

describe('writing the squash message with a model', () => {
  it('replaces the join with one message about the fold', async () => {
    asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      calls.push({ cmd, args: args ?? {} })
      if (cmd === 'squash_preview') return preview()
      if (cmd === 'ai_squash_message') {
        return { summary: 'Add a parser for the log format', body: 'The old one guessed.' }
      }
      return null
    })
    const wrapper = show()
    await flushPromises()

    await wrapper.find('.write').trigger('click')
    await flushPromises()

    // The commits are named, not the range: the backend works the fold out.
    expect(calls.find((c) => c.cmd === 'ai_squash_message')?.args).toEqual({ oids })
    expect((wrapper.find('textarea').element as HTMLTextAreaElement).value).toBe(
      'Add a parser for the log format\n\nThe old one guessed.'
    )
  })

  it('offers nothing to press when no model is configured', async () => {
    ai.store.status = { configured: false, model: null, default_commit_prompt: '' }
    const wrapper = show()
    await flushPromises()
    expect(wrapper.find('.write').exists()).toBe(false)
  })
})
