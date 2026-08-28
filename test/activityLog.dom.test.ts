// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import ActivityLog from '~/components/ActivityLog.vue'
import { useGit } from '~/composables/useGit'
import ResizeHandle from '~/components/ResizeHandle.vue'
import { usePanes } from '~/composables/usePanes'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)
let calls: { cmd: string; args: Record<string, unknown> }[] = []

const git = useGit()

function answer(out: Partial<{ ok: boolean; code: number; stdout: string; stderr: string }>) {
  asked.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    calls.push({ cmd, args: args ?? {} })
    if (cmd === 'run_git') return { argv: [], ok: true, code: 0, stdout: '', stderr: '', ...out }
    return null
  })
}

async function type(wrapper: ReturnType<typeof mount>, line: string) {
  const input = wrapper.find('input')
  await input.setValue(line)
  await wrapper.find('form').trigger('submit')
  await flushPromises()
}

beforeEach(() => {
  calls = []
  asked.mockReset()
  git.store.log.length = 0
  git.store.repo = { path: '/tmp/repo', name: 'repo', head: 'main', detached: false } as never
})

describe('the console', () => {
  it('reads oldest first, with the prompt beneath the transcript', async () => {
    answer({ stdout: 'abc1234 first\n' })
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    await type(wrapper, 'log --oneline -1')

    const rows = wrapper.findAll('.entry .text').map((one) => one.text())
    const command = rows.findIndex((text) => text === 'git log --oneline -1')
    const output = rows.findIndex((text) => text === 'abc1234 first')
    expect(command).toBeGreaterThanOrEqual(0)
    expect(output).toBe(command + 1)

    // The input is the last thing in the console, under everything else.
    const order = [...wrapper.element.querySelectorAll('.body, .prompt-row')]
    expect(order[0]!.className).toContain('body')
    expect(order[1]!.className).toContain('prompt-row')
  })


  it('opens with the terminal button, caret in the prompt', async () => {
    const wrapper = mount(ActivityLog, { attachTo: document.body })
    expect(wrapper.find('input').exists()).toBe(false)
    await wrapper.find('.term').trigger('click')
    expect(wrapper.find('input').exists()).toBe(true)
    expect(document.activeElement).toBe(wrapper.find('input').element)
    wrapper.unmount()
  })

  it('runs what was typed and writes the command and its output to the log', async () => {
    answer({ stdout: 'abc1234 first\n' })
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    await type(wrapper, 'git log --oneline -1')

    expect(calls.find((c) => c.cmd === 'run_git')?.args.args).toEqual(['log', '--oneline', '-1'])
    const lines = git.store.log.map((l) => [l.level, l.text])
    expect(lines).toContainEqual(['command', 'git log --oneline -1'])
    expect(lines).toContainEqual(['output', 'abc1234 first'])
    expect(wrapper.find('input').element.value).toBe('')
  })

  it('shows a failed command in red, with what git said, and no notice', async () => {
    answer({ ok: false, code: 128, stderr: "fatal: not a valid object name: 'nope'" })
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    await type(wrapper, 'show nope')

    // The store is newest-first; the console turns it round for reading, so
    // what the user sees is the command with its output beneath it.
    expect(git.store.log[0]).toMatchObject({ level: 'output' })
    expect(git.store.log[1]).toMatchObject({ level: 'failed', text: 'git show nope' })
    expect(wrapper.find('.entry.failed').exists()).toBe(true)
  })

  it('recalls earlier lines with the arrow keys', async () => {
    answer({})
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    await type(wrapper, 'status')
    await type(wrapper, 'fetch')

    const input = wrapper.find('input')
    await input.trigger('keydown', { key: 'ArrowUp' })
    expect(input.element.value).toBe('fetch')
    await input.trigger('keydown', { key: 'ArrowUp' })
    expect(input.element.value).toBe('status')
    await input.trigger('keydown', { key: 'ArrowDown' })
    await input.trigger('keydown', { key: 'ArrowDown' })
    expect(input.element.value).toBe('')
  })

  it('has nothing to type into without a repository', async () => {
    git.store.repo = null
    const wrapper = mount(ActivityLog)
    await wrapper.find('.term').trigger('click')
    expect(wrapper.find('input').attributes('disabled')).toBeDefined()
  })
})

/**
 * How tall the console is.
 *
 * It used to take its height from its own contents, so a session with a few
 * hundred lines in it opened over the whole window. It is a pane now: a
 * height, a handle to drag, and the window's own limit above that.
 */
describe('the size of the console', () => {
  const { layout, reset, start } = usePanes()

  /** One drag of the console's handle, `by` pixels down the screen. */
  function drag(by: number) {
    start(new PointerEvent('pointerdown', { clientY: 400 }), 'console')
    window.dispatchEvent(new PointerEvent('pointermove', { clientY: 400 + by }))
    window.dispatchEvent(new PointerEvent('pointerup', { clientY: 400 + by }))
  }

  const open = async () => {
    const wrapper = mount(ActivityLog, { global: { components: { ResizeHandle } } })
    await wrapper.find('.strip').trigger('click')
    await flushPromises()
    return wrapper
  }

  beforeEach(() => reset('console'))

  it('is as tall as the layout says, whatever is in it', async () => {
    for (let at = 0; at < 200; at += 1) git.note(`line ${at}`)
    const wrapper = await open()
    expect(wrapper.find('.body').attributes('style')).toContain('height: 220px')
  })

  it('has a handle of its own, which grows it upwards and stops at the limits', async () => {
    const wrapper = await open()
    expect(wrapper.findComponent(ResizeHandle).props('side')).toBe('console')

    // Dragging up grows it, and neither direction runs away with the window.
    drag(-120)
    expect(layout.console).toBe(340)
    drag(-5000)
    expect(layout.console).toBe(640)
    drag(5000)
    expect(layout.console).toBe(96)
  })

  it('has no handle to drag while it is shut', async () => {
    const wrapper = mount(ActivityLog, { global: { components: { ResizeHandle } } })
    expect(wrapper.findComponent(ResizeHandle).exists()).toBe(false)
  })
})
